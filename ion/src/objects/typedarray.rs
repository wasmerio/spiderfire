/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::ops::{Deref, DerefMut};

use mozjs::jsapi::{
	Handle, JS_NewFloat32ArrayWithBuffer, JS_NewFloat64ArrayWithBuffer, JS_NewInt16ArrayWithBuffer, JS_NewInt32ArrayWithBuffer,
	JS_NewInt8ArrayWithBuffer, JS_NewUint16ArrayWithBuffer, JS_NewUint32ArrayWithBuffer, JS_NewUint8ArrayWithBuffer,
	JS_NewUint8ClampedArrayWithBuffer, JSContext, JSObject,
};
use mozjs::jsapi::Scalar::Type;
use mozjs::typedarray::CreateWith;

use crate::{Context, Error, Object, Value};
use crate::conversions::ToValue;
use crate::exception::ThrowException;

macro_rules! impl_typedarray_wrapper {
	($typedarray:ident, $ty:ty) => {
		pub struct $typedarray {
			pub buf: Vec<$ty>,
		}

		impl Deref for $typedarray {
			type Target = Vec<$ty>;

			fn deref(&self) -> &Self::Target {
				&self.buf
			}
		}

		impl DerefMut for $typedarray {
			fn deref_mut(&mut self) -> &mut Self::Target {
				&mut self.buf
			}
		}

		impl<'cx> ToValue<'cx> for $typedarray {
			unsafe fn to_value(&self, cx: &'cx Context, value: &mut Value) {
				let mut typedarray = Object::new(cx);
				if mozjs::typedarray::$typedarray::create(**cx, CreateWith::Slice(self.buf.as_slice()), typedarray.handle_mut()).is_ok() {
					typedarray.to_value(cx, value);
				} else {
					Error::new(concat!("Failed to create", stringify!($typedarray)), None).throw(cx)
				}
			}
		}
	};
}

impl_typedarray_wrapper!(Uint8Array, u8);
impl_typedarray_wrapper!(Uint16Array, u16);
impl_typedarray_wrapper!(Uint32Array, u32);
impl_typedarray_wrapper!(Int8Array, i8);
impl_typedarray_wrapper!(Int16Array, i16);
impl_typedarray_wrapper!(Int32Array, i32);
impl_typedarray_wrapper!(Float32Array, f32);
impl_typedarray_wrapper!(Float64Array, f64);
impl_typedarray_wrapper!(Uint8ClampedArray, u8);
impl_typedarray_wrapper!(ArrayBuffer, u8);

pub fn type_to_constructor(ty: Type) -> unsafe extern "C" fn(*mut JSContext, Handle<*mut JSObject>, usize, i64) -> *mut JSObject {
	match ty {
		Type::Int8 => JS_NewInt8ArrayWithBuffer,
		Type::Uint8 => JS_NewUint8ArrayWithBuffer,
		Type::Int16 => JS_NewInt16ArrayWithBuffer,
		Type::Uint16 => JS_NewUint16ArrayWithBuffer,
		Type::Int32 => JS_NewInt32ArrayWithBuffer,
		Type::Uint32 => JS_NewUint32ArrayWithBuffer,
		Type::Float32 => JS_NewFloat32ArrayWithBuffer,
		Type::Float64 => JS_NewFloat64ArrayWithBuffer,
		Type::Uint8Clamped => JS_NewUint8ClampedArrayWithBuffer,
		_ => unreachable!(),
	}
}

pub fn type_to_element_size(ty: Type) -> usize {
	match ty {
		Type::Int8 => 1,
		Type::Uint8 => 1,
		Type::Int16 => 2,
		Type::Uint16 => 2,
		Type::Int32 => 4,
		Type::Uint32 => 4,
		Type::Float32 => 4,
		Type::Float64 => 8,
		Type::Uint8Clamped => 1,
		Type::BigInt64 => 1,
		Type::BigUint64 => 1,
		_ => unreachable!(),
	}
}
