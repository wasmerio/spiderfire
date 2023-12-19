/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::fmt;
use std::fmt::{Display, Formatter};

use bytes::Bytes;
use form_urlencoded::Serializer;
use futures::StreamExt;
use hyper::Body;
use multipart::client::multipart;
use mozjs::jsval::JSVal;

use ion::{Context, Error, ErrorKind, Result, Value, Heap, ReadableStream};
use ion::conversions::FromValue;

use crate::globals::file::{Blob, buffer_source_to_bytes};
use crate::globals::form_data::{FormData, FormDataEntryValue};
use crate::globals::url::URLSearchParams;

#[derive(Debug, Traceable)]
#[non_exhaustive]
pub enum FetchBodyInner {
	None,
	Bytes(#[ion(no_trace)] Bytes),
	Stream(#[ion(no_trace)] ReadableStream),
}

impl Clone for FetchBodyInner {
	fn clone(&self) -> Self {
		match self {
			Self::None => Self::None,
			Self::Bytes(b) => Self::Bytes(b.clone()),
			Self::Stream(s) => Self::Stream(ReadableStream::new(s.get()).unwrap()),
		}
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

#[derive(Debug, Traceable)]
pub struct FetchBody {
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
		matches!(&self.body, FetchBodyInner::None)
	}

	pub fn len(&self) -> FetchBodyLength {
		match &self.body {
			FetchBodyInner::None => FetchBodyLength::None,
			FetchBodyInner::Bytes(bytes) => FetchBodyLength::Known(bytes.len()),
			FetchBodyInner::Stream(_) => FetchBodyLength::Unknown,
		}
	}

	pub fn is_not_stream(&self) -> bool {
		matches!(&self.body, FetchBodyInner::None | FetchBodyInner::Bytes(_))
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
				let mut stream = Box::pin(reader.into_rust_stream(cx));
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
				let bytes = reader.read_to_end(cx).await.map_err(|e| e.to_error())?;
				Ok(Some(bytes.into()))
			}
		}
	}
}

impl Clone for FetchBody {
	fn clone(&self) -> FetchBody {
		FetchBody {
			body: self.body.clone(),
			source: self.source.as_ref().map(|s| Heap::new(s.get())),
			kind: self.kind.clone(),
		}
	}
}

impl Default for FetchBody {
	fn default() -> FetchBody {
		FetchBody {
			body: FetchBodyInner::None,
			source: None,
			kind: None,
		}
	}
}

impl<'cx> FromValue<'cx> for FetchBody {
	type Config = ();
	fn from_value(cx: &'cx Context, value: &Value, strict: bool, _: ()) -> Result<FetchBody> {
		if value.handle().is_string() {
			return Ok(FetchBody {
				body: FetchBodyInner::Bytes(Bytes::from(String::from_value(cx, value, strict, ()).unwrap())),
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
			if let Ok(bytes) = buffer_source_to_bytes(&value.to_object(cx)) {
				return Ok(FetchBody {
					body: FetchBodyInner::Bytes(bytes),
					source: Some(Heap::new(value.get())),
					kind: None,
				});
			} else if let Ok(blob) = <&Blob>::from_value(cx, value, strict, ()) {
				return Ok(FetchBody {
					body: FetchBodyInner::Bytes(blob.as_bytes().clone()),
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
					body: FetchBodyInner::Bytes(bytes),
					source: Some(Heap::new(value.handle().get())),
					kind: Some(FetchBodyKind::FormData(content_type)),
				});
			} else if let Ok(search_params) = <&URLSearchParams>::from_value(cx, value, strict, ()) {
				return Ok(FetchBody {
					body: FetchBodyInner::Bytes(Bytes::from(
						Serializer::new(String::new()).extend_pairs(search_params.pairs()).finish(),
					)),
					source: Some(Heap::new(value.get())),
					kind: Some(FetchBodyKind::URLSearchParams),
				});
			}
		}
		Err(Error::new("Expected Valid Body", ErrorKind::Type))
	}
}
