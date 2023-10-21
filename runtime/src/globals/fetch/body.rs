/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::fmt::{Display, Formatter};

use bytes::Bytes;
use hyper::Body;
use mozjs::gc::Traceable;
use mozjs::jsapi::{ESClass, Heap, JSTracer};
use mozjs::jsval::JSVal;

use ion::{Context, Error, ErrorKind, Result, Value};
use ion::conversions::FromValue;

#[derive(Debug, Clone)]
enum FetchBodyInner {
	Bytes(Bytes),
}

#[derive(Copy, Clone, Debug)]
pub enum FetchBodyKind {
	String,
}

impl Display for FetchBodyKind {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			FetchBodyKind::String => f.write_str("text/plain;charset=UTF-8"),
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
			kind: self.kind,
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
