/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::{ptr, slice};
use std::ops::{Deref, DerefMut};

use mozjs::jsapi::{JSObject, GetArrayBufferViewLengthAndData};
use mozjs_sys::jsapi::JS_IsArrayBufferViewObject;

use crate::conversions::FromValue;
use crate::{Context, Error, ErrorKind, Local, Result};

pub struct ArrayBufferView<'ab> {
	buffer: Local<'ab, *mut JSObject>,
}

impl<'ab> ArrayBufferView<'ab> {
	pub fn from(object: Local<'ab, *mut JSObject>) -> Option<ArrayBufferView<'ab>> {
		if ArrayBufferView::is_array_buffer_view(object.get()) {
			Some(ArrayBufferView { buffer: object })
		} else {
			None
		}
	}

	pub unsafe fn from_unchecked(object: Local<'ab, *mut JSObject>) -> ArrayBufferView<'ab> {
		ArrayBufferView { buffer: object }
	}

	/// Returns a pointer and length to the contents of the [ArrayBuffer].
	///
	/// The pointer may be invalidated if the [ArrayBuffer] is detached.
	pub fn data(&self) -> (*mut u8, usize) {
		let mut len = 0;
		let mut shared = false;
		let mut data = ptr::null_mut();
		unsafe { GetArrayBufferViewLengthAndData(self.get(), &mut len, &mut shared, &mut data) };
		(data, len)
	}

	/// Returns the length of the [ArrayBuffer].
	pub fn len(&self) -> usize {
		self.data().1
	}

	/// Returns a slice to the contents of the [ArrayBuffer].
	///
	/// The slice may be invalidated if the [ArrayBuffer] is detached.
	pub unsafe fn as_slice(&self) -> &[u8] {
		let (ptr, len) = self.data();
		unsafe { slice::from_raw_parts(ptr, len) }
	}

	/// Returns a mutable slice to the contents of the [ArrayBuffer].
	///
	/// The slice may be invalidated if the [ArrayBuffer] is detached.
	pub unsafe fn as_mut_slice(&mut self) -> &mut [u8] {
		let (ptr, len) = self.data();
		unsafe { slice::from_raw_parts_mut(ptr, len) }
	}

	/// Checks if an object is an array buffer.
	#[allow(clippy::not_unsafe_ptr_arg_deref)]
	pub fn is_array_buffer_view(object: *mut JSObject) -> bool {
		unsafe { JS_IsArrayBufferViewObject(object) }
	}
}

impl<'ab> Deref for ArrayBufferView<'ab> {
	type Target = Local<'ab, *mut JSObject>;

	fn deref(&self) -> &Self::Target {
		&self.buffer
	}
}

impl<'ab> DerefMut for ArrayBufferView<'ab> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.buffer
	}
}

impl<'cx> FromValue<'cx> for ArrayBufferView<'cx> {
	type Config = ();

	fn from_value(cx: &'cx Context, value: &crate::Value, _strict: bool, _config: Self::Config) -> Result<Self> {
		let value = value.handle();
		if value.is_object() {
			let object = value.to_object();
			let local = cx.root_object(object);
			Self::from(local).ok_or_else(|| Error::new("Expected ArrayBufferView", ErrorKind::Type))
		} else {
			Err(Error::new("Expected Object", ErrorKind::Type))
		}
	}
}
