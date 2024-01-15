/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::convert::Infallible;
use std::fmt;
use std::fmt::{Display, Formatter};

use bytes::Bytes;
use futures::{StreamExt, stream};
use hyper::Body;
use hyper::body::HttpBody;
use ion::class::NativeObject;
use ion::typedarray::ArrayBuffer;
use mozjs::c_str;
use mozjs::jsapi::{CheckReadableStreamControllerCanCloseOrEnqueue, JSObject};
use multipart::client::multipart;
use mozjs::jsval::JSVal;

use ion::{
	Context, Error, ErrorKind, Result, Value, Heap, ReadableStream, Exception, Promise, TracedHeap, ClassDefinition,
	Function, Object,
};
use ion::conversions::{FromValue, ToValue};

use crate::globals::file::{Blob, BufferSource, File, BlobPart, FileOptions, BlobOptions};
use crate::globals::form_data::{FormData, FormDataEntryValue};
use crate::globals::streams::{NativeStreamSourceCallbacks, NativeStreamSource};
use crate::globals::url::URLSearchParams;
use crate::promise::future_to_promise;

use super::header::Header;

#[derive(Debug, Traceable)]
#[non_exhaustive]
pub enum FetchBodyInner {
	None,
	Bytes(#[trace(no_trace)] Bytes),
	Stream(#[trace(no_trace)] ReadableStream),
}

impl FetchBodyInner {
	pub fn try_clone(&mut self, cx: &Context) -> Result<Self> {
		Ok(match self {
			Self::None => Self::None,
			Self::Bytes(bytes) => Self::Bytes(bytes.clone()),
			Self::Stream(stream) => Self::Stream(stream.try_clone(cx)?),
		})
	}
}

impl Default for FetchBodyInner {
	fn default() -> Self {
		Self::None
	}
}

#[derive(Clone, Debug, Traceable)]
#[non_exhaustive]
pub enum FetchBodyKind {
	String,
	Blob(String),
	FormData(String),
	URLSearchParams,
}

impl Display for FetchBodyKind {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		match self {
			FetchBodyKind::String => f.write_str("text/plain;charset=UTF-8"),
			FetchBodyKind::Blob(mime) => f.write_str(mime),
			FetchBodyKind::URLSearchParams => f.write_str("application/x-www-form-urlencoded;charset=UTF-8"),
			FetchBodyKind::FormData(str) => f.write_str(str.as_str()),
		}
	}
}

#[derive(Debug, Traceable, Default)]
pub struct FetchBody {
	#[trace(no_trace)]
	pub body: FetchBodyInner,
	pub source: Option<Heap<JSVal>>,
	pub kind: Option<FetchBodyKind>,
}

#[derive(Debug)]
pub enum FetchBodyLength {
	None,
	Known(usize),
	Unknown,
}

impl FetchBody {
	pub fn is_none(&self) -> bool {
		matches!(self.body, FetchBodyInner::None)
	}

	pub fn len(&self) -> FetchBodyLength {
		match &self.body {
			FetchBodyInner::None => FetchBodyLength::None,
			FetchBodyInner::Bytes(bytes) => FetchBodyLength::Known(bytes.len()),
			FetchBodyInner::Stream(_) => FetchBodyLength::Unknown,
		}
	}

	// We're running on a single-threaded runtime, but hyper doesn't
	// know this, so it requires any streams to be Send, which is
	// impossible with anything SpiderMonkey-related. Instead, we have
	// to use a channel body, and create a second future that reads
	// the stream and queues the chunks to the channel.
	pub fn into_http_body(self, cx: Context) -> Result<(Body, Option<impl std::future::Future<Output = ()>>)> {
		match self.body {
			FetchBodyInner::None => Ok((Body::empty(), None)),
			FetchBodyInner::Bytes(bytes) => Ok((Body::from(bytes), None)),
			FetchBodyInner::Stream(stream) => {
				let reader = stream.into_reader(&cx)?;
				let mut stream = Box::pin(reader.into_rust_stream(cx.duplicate()));
				drop(cx);
				let (mut sender, body) = Body::channel();
				let future = async move {
					loop {
						let chunk = stream.next().await;
						match chunk {
							None => break,
							Some(Ok(bytes)) => {
								if let Err(_) = sender.send_data(Bytes::from(bytes)).await {
									sender.abort();
									break;
								}
							}
							Some(Err(_)) => {
								sender.abort();
								break;
							}
						}
					}
				};
				Ok((body, Some(future)))
			}
		}
	}

	pub async fn into_bytes(self, cx: Context) -> Result<Option<Bytes>> {
		match self.body {
			FetchBodyInner::None => Ok(None),
			FetchBodyInner::Bytes(bytes) => Ok(Some(bytes)),
			FetchBodyInner::Stream(stream) => {
				let reader = stream.into_reader(&cx)?;
				let (_, bytes) = cx.await_native_cx(|cx| reader.read_to_end(cx)).await;
				Ok(Some(bytes.map_err(|e| e.to_error())?.into()))
			}
		}
	}

	pub async fn into_text(self, cx: Context) -> Result<String> {
		let bytes = self.into_bytes(cx).await?;
		String::from_utf8(bytes.unwrap_or_default().into())
			.map_err(|e| Error::new(&format!("Invalid UTF-8 sequence: {}", e), ErrorKind::Normal))
	}

	pub async fn into_json(self, cx: Context) -> Result<*mut JSObject> {
		let (cx, text) = cx.await_native_cx(|cx| self.into_text(cx)).await;
		let text = text?;
		Ok((*ion::json::parse(&cx, text)?).get())
	}

	pub async fn into_blob(self, cx: Context, content_type: Option<Header>) -> Result<*mut JSObject> {
		let (cx, bytes) = cx.await_native_cx(|cx| self.into_bytes(cx)).await;
		let blob = match (content_type, bytes?) {
			(Some(header), Some(bytes)) => Blob::new_with_kind(bytes, header.to_string()),
			(None, Some(bytes)) => Blob::new(bytes),
			(_, None) => Blob::new(Bytes::default()),
		};
		Ok(Blob::new_object(&cx, Box::new(blob)))
	}

	pub async fn into_form_data(self, cx: Context, content_type: Header) -> Result<*mut JSObject> {
		let (cx, bytes) = cx.await_native_cx(|cx| self.into_bytes(cx)).await;
		let bytes = bytes?.unwrap_or_default();

		let content_type = content_type.to_string();

		if content_type.starts_with("application/x-www-form-urlencoded") {
			let parsed = form_urlencoded::parse(bytes.as_ref());
			let mut form_data = FormData::constructor();

			for (key, val) in parsed {
				form_data.append_native_string(key.into_owned(), val.into_owned());
			}

			Ok(FormData::new_object(&cx, Box::new(form_data)))
		} else if content_type.starts_with("multipart/form-data") {
			const MULTIPART_HEADER_WITH_BOUNDARY: &str = "multipart/form-data; boundary=";
			if !content_type.starts_with(MULTIPART_HEADER_WITH_BOUNDARY) {
				return Err(Error::new(
					"multipart/form-data content without a boundary, cannot parse",
					ErrorKind::Normal,
				));
			}

			let boundary = &content_type.as_str()[MULTIPART_HEADER_WITH_BOUNDARY.len()..];

			// TODO: read asynchronously from stream bodies. This is complicated by the fact
			// that multer expects a `Send` stream, but we're running on a single-threaded
			// runtime.
			let mut parser = multer::Multipart::new(
				stream::once(async { std::result::Result::<_, Infallible>::Ok(bytes) }),
				boundary,
			);

			let mut form_data = FormData::constructor();
			let mut cx = cx;

			while let Some(next) = parser.next_field().await? {
				let name = next.name().unwrap_or("").to_string();
				let file_name = next.file_name().map(|f| f.to_string());
				let content_type = next.content_type().map(|c| c.to_string());

				let bytes;
				(cx, bytes) = cx.await_native(next.bytes()).await;
				let bytes = bytes.map_err(|_| Error::new("Failed to read form data from body", ErrorKind::Normal))?;

				match file_name {
					Some(file_name) => {
						let file = File::constructor(
							vec![BlobPart(bytes)],
							file_name,
							Some(FileOptions {
								last_modified: None,
								blob: BlobOptions {
									endings: crate::globals::file::Endings::Transparent,
									kind: content_type,
								},
							}),
						);
						let file = cx.root_object(File::new_object(&cx, Box::new(file))).into();

						form_data.append_native_file(name, File::get_private(&file));
					}

					None => form_data.append_native_string(
						name,
						String::from_utf8(bytes.into())
							.map_err(|_| Error::new("String contains invalid UTF-8 sequence", ErrorKind::Normal))?,
					),
				}
			}

			Ok(FormData::new_object(&cx, Box::new(form_data)))
		} else {
			Err(Error::new(
				"Invalid content-type, cannot decide form data format",
				ErrorKind::Type,
			))
		}
	}

	pub fn try_clone(&mut self, cx: &Context) -> Result<Self> {
		Ok(Self {
			body: self.body.try_clone(cx)?,
			source: self.source.as_ref().map(|s| Heap::new(s.get())),
			kind: self.kind.clone(),
		})
	}

	pub async fn try_clone_with_cached_body(&mut self, cx: Context) -> Result<Self> {
		// Can't move out of a reference. We need to instead swap the body out and then back in again.
		let mut my_body = FetchBodyInner::None;
		std::mem::swap(&mut self.body, &mut my_body);

		let (mut my_body, cloned_body) = match my_body {
			FetchBodyInner::None => (FetchBodyInner::None, FetchBodyInner::None),
			FetchBodyInner::Bytes(bytes) => (FetchBodyInner::Bytes(bytes.clone()), FetchBodyInner::Bytes(bytes)),
			FetchBodyInner::Stream(stream) => {
				let reader = stream.into_reader(&cx)?;
				let bytes: Bytes = reader.read_to_end(cx).await.map_err(|e| e.to_error())?.into();
				(FetchBodyInner::Bytes(bytes.clone()), FetchBodyInner::Bytes(bytes))
			}
		};

		std::mem::swap(&mut self.body, &mut my_body);

		Ok(Self {
			body: cloned_body,
			source: self.source.as_ref().map(|s| Heap::new(s.get())),
			kind: self.kind.clone(),
		})
	}
}

impl<'cx> FromValue<'cx> for FetchBody {
	type Config = ();
	fn from_value(cx: &'cx Context, value: &Value, strict: bool, _: ()) -> Result<FetchBody> {
		if value.handle().is_string() {
			let bytes = Bytes::from(String::from_value(cx, value, strict, ()).unwrap());
			return Ok(FetchBody {
				body: FetchBodyInner::Bytes(bytes),
				source: Some(Heap::new(value.get())),
				kind: Some(FetchBodyKind::String),
			});
		} else if value.handle().is_object() {
			if let Some(stream) = ReadableStream::from_local(&value.to_object(cx)) {
				return Ok(FetchBody {
					body: FetchBodyInner::Stream(stream),
					source: Some(Heap::new(value.get())),
					kind: None,
				});
			}
			if let Ok(source) = BufferSource::from_value(cx, value, strict, false) {
				return Ok(FetchBody {
					body: FetchBodyInner::Bytes(source.to_bytes()),
					source: Some(Heap::new(value.get())),
					kind: None,
				});
			} else if let Ok(blob) = <&Blob>::from_value(cx, value, strict, ()) {
				let bytes = blob.as_bytes().clone();
				return Ok(FetchBody {
					body: FetchBodyInner::Bytes(bytes),
					source: Some(Heap::new(value.get())),
					kind: blob.kind().map(FetchBodyKind::Blob),
				});
			} else if let Ok(form_data) = <&FormData>::from_value(cx, value, strict, ()) {
				let mut form = multipart::Form::default();
				for kv in form_data.all_pairs() {
					match &kv.value {
						FormDataEntryValue::String(str) => form.add_text(kv.key.as_str(), str),
						FormDataEntryValue::File(file) => {
							// TODO: remove to_vec call
							let file = File::get_private(&file.root(cx).into());
							form.add_reader_file(
								kv.key.as_str(),
								std::io::Cursor::new(file.blob.as_bytes().to_vec()),
								&file.name,
							)
						}
					}
				}
				let content_type = form.content_type();

				// TODO: store the form directly
				let builder = hyper::Request::builder();
				let req = form.set_body::<multipart::Body>(builder).unwrap();
				let bytes = futures::executor::block_on(hyper::body::to_bytes(req.into_body())).unwrap();

				return Ok(FetchBody {
					body: FetchBodyInner::Bytes(bytes),
					source: Some(Heap::new(value.handle().get())),
					kind: Some(FetchBodyKind::FormData(content_type)),
				});
			} else if let Ok(search_params) = <&URLSearchParams>::from_value(cx, value, strict, ()) {
				let bytes = Bytes::from(
					form_urlencoded::Serializer::new(String::new())
						.extend_pairs(search_params.pairs())
						.finish(),
				);
				return Ok(FetchBody {
					body: FetchBodyInner::Bytes(bytes),
					source: Some(Heap::new(value.get())),
					kind: Some(FetchBodyKind::URLSearchParams),
				});
			}
		}
		Err(Error::new("Expected Valid Body", ErrorKind::Type))
	}
}

pub fn hyper_body_to_stream(cx: &Context, body: Body) -> Option<ReadableStream> {
	let source = HyperBodyStreamSource { body };
	crate::globals::streams::readable_stream_from_callbacks(cx, Box::new(source))
}

struct HyperBodyStreamSource {
	body: Body,
}

impl NativeStreamSourceCallbacks for HyperBodyStreamSource {
	fn start<'cx>(
		&self, _source: &'cx NativeStreamSource, cx: &'cx Context, _controller: ion::Object<'cx>,
	) -> ion::ResultExc<Value<'cx>> {
		Ok(Value::undefined(cx))
	}

	fn pull<'cx>(
		&self, source: &'cx NativeStreamSource, cx: &'cx Context, controller: ion::Object<'cx>,
	) -> ion::ResultExc<ion::Promise> {
		unsafe {
			if !CheckReadableStreamControllerCanCloseOrEnqueue(
				cx.as_ptr(),
				controller.handle().into(),
				c_str!("enqueue"),
			) {
				return Err(Exception::Error(Error::new(
					"Readable stream is already closed",
					ErrorKind::Type,
				)));
			}

			let stream_source = TracedHeap::new(source.reflector().get());
			let controller = TracedHeap::from_local(&controller);

			Ok(future_to_promise(cx, move |cx| async move {
				let (cx, chunk) = cx
					.await_native(
						NativeStreamSource::get_mut_private(&mut stream_source.to_local().into())
							.get_typed_source_mut::<Self>()
							.body
							.data(),
					)
					.await;

				let controller = ion::Object::from(controller.root(&cx));
				match chunk {
					None => {
						let close_func =
							Function::from_object(&cx, &controller.get(&cx, "close").unwrap().to_object(&cx)).unwrap();
						close_func.call(&cx, &controller, &[]).map_err(|e| e.unwrap().exception)?;
						ion::ResultExc::<_>::Ok(())
					}

					Some(chunk) => {
						let chunk = chunk
							.map_err(|_| Error::new("Failed to read request body from network", ErrorKind::Normal))?;
						let array_buffer = Object::from(
							ArrayBuffer::copy_from_bytes(&cx, chunk.as_ref())
								.ok_or_else(|| Error::new("Failed to allocate array", ErrorKind::Normal))?
								.into_local(),
						)
						.as_value(&cx);

						let enqueue_func =
							Function::from_object(&cx, &controller.get(&cx, "enqueue").unwrap().to_object(&cx))
								.unwrap();
						enqueue_func.call(&cx, &controller, &[array_buffer]).map_err(|e| e.unwrap().exception)?;
						Ok(())
					}
				}
			})
			.expect("Future queue should be running"))
		}
	}

	fn cancel<'cx>(self: Box<Self>, cx: &'cx Context, _reason: Value) -> ion::ResultExc<ion::Promise> {
		drop(self.body);
		Ok(Promise::new_resolved(cx, Value::undefined(cx)))
	}
}
