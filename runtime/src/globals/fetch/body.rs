/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::fmt;
use std::fmt::{Display, Formatter};

use bytes::Bytes;
use futures::StreamExt;
use hyper::Body;
use hyper::body::HttpBody;
use ion::class::NativeObject;
use ion::typedarray::ArrayBuffer;
use mozjs::c_str;
use mozjs::jsapi::CheckReadableStreamControllerCanCloseOrEnqueue;
use multipart::client::multipart;
use mozjs::jsval::JSVal;

use ion::{
	Context, Error, ErrorKind, Result, Value, Heap, ReadableStream, Exception, Promise, TracedHeap, ClassDefinition,
	Function,
};
use ion::conversions::{FromValue, ToValue};

use crate::globals::file::{Blob, buffer_source_to_bytes};
use crate::globals::form_data::{FormData, FormDataEntryValue};
use crate::globals::streams::{NativeStreamSourceCallbacks, NativeStreamSource};
use crate::globals::url::URLSearchParams;
use crate::promise::future_to_promise;

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
	#[ion(no_trace)]
	pub body: Option<ReadableStream>,
	pub length: Option<usize>,
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
		self.body.is_none()
	}

	pub fn len(&self) -> FetchBodyLength {
		match (&self.body, &self.length) {
			(None, _) => FetchBodyLength::None,
			(_, None) => FetchBodyLength::Unknown,
			(_, Some(len)) => FetchBodyLength::Known(*len),
		}
	}

	// We're running on a single-threaded runtime, but hyper doesn't
	// know this, so it requires any streams to be Send, which is
	// impossible with anything SpiderMonkey-related. Instead, we have
	// to use a channel body, and create a second future that reads
	// the stream and queues the chunks to the channel.
	pub fn into_http_body(self, cx: Context) -> Result<(Body, Option<impl std::future::Future<Output = ()>>)> {
		match self.body {
			None => Ok((Body::empty(), None)),
			Some(stream) => {
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
			None => Ok(None),
			Some(stream) => {
				let reader = stream.into_reader(&cx)?;
				let bytes = reader.read_to_end(cx).await.map_err(|e| e.to_error())?;
				Ok(Some(bytes.into()))
			}
		}
	}

	pub fn try_clone(&mut self, cx: &Context) -> Result<Self> {
		Ok(Self {
			body: self.body.as_mut().map(|s| s.try_clone(cx)).transpose()?,
			length: self.length.clone(),
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
				length: Some(bytes.len()),
				body: Some(ReadableStream::from_bytes(cx, bytes)),
				source: Some(Heap::new(value.get())),
				kind: Some(FetchBodyKind::String),
			});
		} else if value.handle().is_object() {
			if let Some(stream) = ReadableStream::from_local(&value.to_object(cx)) {
				return Ok(FetchBody {
					body: Some(stream),
					length: None,
					source: Some(Heap::new(value.get())),
					kind: None,
				});
			}
			if let Ok(bytes) = buffer_source_to_bytes(&value.to_object(cx)) {
				return Ok(FetchBody {
					length: Some(bytes.len()),
					body: Some(ReadableStream::from_bytes(cx, bytes)),
					source: Some(Heap::new(value.get())),
					kind: None,
				});
			} else if let Ok(blob) = <&Blob>::from_value(cx, value, strict, ()) {
				let bytes = blob.as_bytes().clone();
				return Ok(FetchBody {
					length: Some(bytes.len()),
					body: Some(ReadableStream::from_bytes(cx, bytes)),
					source: Some(Heap::new(value.get())),
					kind: blob.kind().map(FetchBodyKind::Blob),
				});
			} else if let Ok(form_data) = <&FormData>::from_value(cx, value, strict, ()) {
				let mut form = multipart::Form::default();
				for kv in form_data.all_pairs() {
					match &kv.value {
						FormDataEntryValue::String(str) => form.add_text(kv.key.as_str(), str),
						FormDataEntryValue::File(bytes, name) => {
							// TODO: remove to_vec call
							form.add_reader_file(kv.key.as_str(), std::io::Cursor::new(bytes.to_vec()), name.as_str())
						}
					}
				}
				let content_type = form.content_type();

				// TODO: store the form directly
				let builder = hyper::Request::builder();
				let req = form.set_body::<multipart::Body>(builder).unwrap();
				let bytes = futures::executor::block_on(hyper::body::to_bytes(req.into_body())).unwrap();

				return Ok(FetchBody {
					length: Some(bytes.len()),
					body: Some(ReadableStream::from_bytes(cx, bytes)),
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
					length: Some(bytes.len()),
					body: Some(ReadableStream::from_bytes(cx, bytes)),
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
						let array_buffer = ArrayBuffer::from(chunk.as_ref()).to_object(&cx)?.as_value(&cx);

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
