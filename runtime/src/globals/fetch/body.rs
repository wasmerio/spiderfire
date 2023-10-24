/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::fmt::{Display, Formatter};

use bytes::Bytes;
use hyper::Body;
use multipart::client::multipart;
use mozjs::gc::Traceable;
use mozjs::jsapi::{ESClass, Heap, JSTracer};
use mozjs::jsval::JSVal;

use ion::{Context, Error, ErrorKind, Result, Value, ClassDefinition};
use ion::conversions::FromValue;

use crate::globals::file::blob::Blob;
use crate::globals::form_data::{FormData, FormDataEntryValue};
use crate::globals::url::UrlSearchParams;

#[derive(Debug, Clone)]
enum FetchBodyInner {
	Bytes(Bytes),
}

#[derive(Clone, Debug)]
pub enum FetchBodyKind {
	String,
	FormData(String), // The boundary changes, so it has to be stored
	FormUrlEncoded,
}

impl Display for FetchBodyKind {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			FetchBodyKind::String => f.write_str("text/plain;charset=UTF-8"),
			FetchBodyKind::FormData(str) => f.write_str(str.as_str()),
			FetchBodyKind::FormUrlEncoded => f.write_str("application/x-www-form-urlencoded"),
		}
	}
}

#[derive(Debug)]
pub struct FetchBody {
	body: FetchBodyInner,
	source: Option<Box<Heap<JSVal>>>,
	pub(crate) kind: Option<FetchBodyKind>,
}

impl FetchBody {
	pub fn is_empty(&self) -> bool {
		match &self.body {
			FetchBodyInner::Bytes(bytes) => bytes.is_empty(),
		}
	}

	pub fn to_http_body(&self) -> Body {
		match &self.body {
			FetchBodyInner::Bytes(bytes) => Body::from(bytes.clone()),
		}
	}

	pub fn to_bytes(&self) -> &Bytes {
		match &self.body {
			FetchBodyInner::Bytes(bytes) => bytes,
		}
	}
}

impl Clone for FetchBody {
	fn clone(&self) -> FetchBody {
		FetchBody {
			body: self.body.clone(),
			source: self.source.as_ref().map(|s| Heap::boxed(s.get())),
			kind: self.kind.clone(),
		}
	}
}

impl Default for FetchBody {
	fn default() -> FetchBody {
		FetchBody {
			body: FetchBodyInner::Bytes(Bytes::new()),
			source: None,
			kind: None,
		}
	}
}

unsafe impl Traceable for FetchBody {
	unsafe fn trace(&self, trc: *mut JSTracer) {
		unsafe {
			self.source.trace(trc);
		}
	}
}

#[macro_export]
macro_rules! typedarray_to_bytes {
	($body:expr) => {
		Err(::ion::Error::new("Expected TypedArray or ArrayBuffer", ::ion::ErrorKind::Type))
	};
	($body:expr, [$arr:ident, true]$(, $($rest:tt)*)?) => {
		paste::paste! {
			if let Ok(arr) = <::mozjs::typedarray::$arr>::from($body) {
				Ok(Bytes::copy_from_slice(unsafe { arr.as_slice() }))
			} else if let Ok(arr) = <::mozjs::typedarray::[<Heap $arr>]>::from($body) {
				Ok(Bytes::copy_from_slice(unsafe { arr.as_slice() }))
			} else {
				$crate::typedarray_to_bytes!($body$(, $($rest)*)?)
			}
		}
	};
	($body:expr, [$arr:ident, false]$(, $($rest:tt)*)?) => {
		paste::paste! {
			if let Ok(arr) = <::mozjs::typedarray::$arr>::from($body) {
				let bytes: &[u8] = cast_slice(arr.as_slice());
				Ok(Bytes::copy_from_slice(bytes))
			} else if let Ok(arr) = <::mozjs::typedarray::[<Heap $arr>]>::from($body) {
				let bytes: &[u8] = cast_slice(arr.as_slice());
				Ok(Bytes::copy_from_slice(bytes))
			} else {
				$crate::typedarray_to_bytes!($body$(, $($rest)*)?)
			}
		}
	};
}

impl<'cx> FromValue<'cx> for FetchBody {
	type Config = ();
	fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, _: bool, _: Self::Config) -> Result<FetchBody>
	where
		'cx: 'v,
	{
		if value.handle().is_string() {
			Ok(FetchBody {
				body: FetchBodyInner::Bytes(Bytes::from(String::from_value(cx, value, true, ()).unwrap())),
				source: Some(Heap::boxed(value.handle().get())),
				kind: Some(FetchBodyKind::String),
			})
		} else if value.handle().is_object() {
			let object = value.to_object(cx);

			let class = object.get_builtin_class(cx);
			if class == ESClass::String {
				let string = object.unbox_primitive(cx).unwrap();
				Ok(FetchBody {
					body: FetchBodyInner::Bytes(Bytes::from(String::from_value(cx, &string, true, ()).unwrap())),
					source: Some(Heap::boxed(value.handle().get())),
					kind: Some(FetchBodyKind::String),
				})
			} else if Blob::instance_of(cx, &object, None) {
				let blob = Blob::get_private(&object);
				Ok(FetchBody {
					body: FetchBodyInner::Bytes(blob.get_bytes()),
					source: Some(Heap::boxed(value.handle().get())),
					kind: None,
				})
			} else if FormData::instance_of(cx, &object, None) {
				let form_data = FormData::get_private(&object);

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

				Ok(FetchBody {
					body: FetchBodyInner::Bytes(bytes),
					source: Some(Heap::boxed(value.handle().get())),
					kind: Some(FetchBodyKind::FormData(content_type)),
				})
			} else if UrlSearchParams::instance_of(cx, &object, None) {
				let search_params = UrlSearchParams::get_private(&object);

				let mut serializer = form_urlencoded::Serializer::new(String::new());
				for (key, value) in search_params.all_pairs() {
					serializer.append_pair(key.as_str(), value.as_str());
				}
				let body = serializer.finish();

				Ok(FetchBody {
					body: FetchBodyInner::Bytes(body.into_bytes().into()),
					source: Some(Heap::boxed(value.handle().get())),
					kind: Some(FetchBodyKind::FormUrlEncoded),
				})
			} else {
				let bytes = typedarray_to_bytes!(object.handle().get(), [ArrayBuffer, true], [ArrayBufferView, true])?;
				Ok(FetchBody {
					body: FetchBodyInner::Bytes(bytes),
					source: Some(Heap::boxed(value.handle().get())),
					kind: None,
				})
			}
		} else {
			Err(Error::new("Expected Body to be String or Object", ErrorKind::Type))
		}
	}
}
