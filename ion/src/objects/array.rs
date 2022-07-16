/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::ops::{Deref, DerefMut};

use mozjs::conversions::{ConversionResult, FromJSValConvertible};
use mozjs::error::throw_type_error;
use mozjs::jsapi::{AssertSameCompartment, HandleValueArray, JS, JSObject, JSVal, NewArrayObject, ObjectOpResult};
use mozjs::jsval::ObjectValue;
use mozjs::rust::{Handle, MutableHandle};
use mozjs::rust::jsapi_wrapped::{GetArrayLength, IsArray, JS_DefineElement, JS_DeleteElement, JS_GetElement, JS_HasElement, JS_SetElement};
use mozjs_sys::jsgc::{GCMethods, RootKind};
use mozjs_sys::jsval::JSVal;

use crate::{Context, Local, Value};
use crate::exception::Exception;
use crate::flags::PropertyFlags;
use crate::value::{FromValue, ToValue};

#[derive(Clone, Debug)]
pub struct Array {
	pub(crate) arr: *mut JSObject,
}

impl Array {
	pub fn new<'c>(cx: &Context<'c>) -> Local<'c, Array> {
		Array::from_slice(cx, &[])
	}

	pub fn from_slice<'c>(cx: &Context<'c>, slice: &[Local<Value>]) -> Local<'c, Array> {
		let values: Vec<_> = slice.iter().map(|x| ***x).collect();
		Array::from_handle(cx, unsafe { HandleValueArray::from_rooted_slice(values.as_slice()) })
	}

	pub(crate) fn from_handle<'c>(cx: &Context<'c>, handle: HandleValueArray) -> Local<'c, Array> {
		let arr = unsafe { NewArrayObject(cx.cx(), &handle) };
		Local::new(cx, Array { arr })
	}

	pub(crate) fn from_raw<'c>(cx: &Context<'c>, arr: *mut JSObject) -> Option<Local<'c, Array>> {
		if unsafe { Array::is_array_raw(cx, arr) } {
			Some(Local::new(cx, Array { arr }))
		} else {
			None
		}
	}

	pub fn to_vec<'c>(&self, cx: &Context<'c>) -> Vec<Local<'c, Value>> {
		let handle = unsafe { Handle::from_marked_location(&**self.to_value(cx)) };
		if let ConversionResult::Success(vec) = unsafe { Vec::<JSVal>::from_jsval(cx.cx(), handle, ()).unwrap() } {
			vec.into_iter().map(|v| Value::from_raw(cx, v)).collect()
		} else {
			Vec::new()
		}
	}

	pub fn to_value<'c>(&self, cx: &Context<'c>) -> Local<'c, Value> {
		Value::from_raw(cx, ObjectValue(self.arr))
	}

	pub fn len(&self, cx: &Context) -> Option<u32> {
		let handle = unsafe { Handle::from_marked_location(&self.arr) };

		let mut length = u32::MAX;
		if unsafe { GetArrayLength(cx.cx(), handle, &mut length) } {
			Some(length)
		} else {
			None
		}
	}

	pub fn has(&self, cx: &Context, index: u32) -> bool {
		let handle = unsafe { Handle::from_marked_location(&self.arr) };
		let mut found = false;

		if unsafe { JS_HasElement(cx.cx(), handle, index, &mut found) } {
			found
		} else {
			Exception::clear(cx);
			false
		}
	}

	pub fn get<'c>(&self, cx: &Context<'c>, index: u32) -> Option<Local<'c, Value>> {
		if self.has(cx, index) {
			let handle = unsafe { Handle::from_marked_location(&self.arr) };
			let mut val = Value::undefined(cx);
			unsafe { JS_GetElement(cx.cx(), handle, index, &mut MutableHandle::from_marked_location(&mut **val)) };
			Some(val)
		} else {
			None
		}
	}

	pub fn set(&mut self, cx: &Context, index: u32, value: Local<Value>) -> bool {
		let handle = unsafe { Handle::from_marked_location(&self.arr) };

		unsafe { JS_SetElement(cx.cx(), handle, index, Handle::from_marked_location(&**value)) }
	}

	pub fn define(&mut self, cx: &Context, index: u32, value: Local<Value>, attrs: PropertyFlags) -> bool {
		let handle = unsafe { Handle::from_marked_location(&self.arr) };

		unsafe { JS_DefineElement(cx.cx(), handle, index, Handle::from_marked_location(&**value), attrs.bits() as u32) }
	}

	pub fn delete(&self, cx: &Context, index: u32) -> (bool, ObjectOpResult) {
		let handle = unsafe { Handle::from_marked_location(&self.arr) };

		let mut result = ObjectOpResult { code_: 0 };

		(unsafe { JS_DeleteElement(cx.cx(), handle, index, &mut result) }, result)
	}

	pub(crate) unsafe fn is_array_raw(cx: &Context, obj: *mut JSObject) -> bool {
		rooted!(in(cx.cx()) let mut robj = obj);

		let mut is_array = false;
		IsArray(cx.cx(), robj.handle(), &mut is_array) && is_array
	}
}

impl RootKind for Array {
	#[allow(non_snake_case)]
	fn rootKind() -> JS::RootKind {
		JS::RootKind::Object
	}
}

impl GCMethods for Array {
	unsafe fn initial() -> Self {
		Array { arr: GCMethods::initial() }
	}

	unsafe fn post_barrier(v: *mut Self, prev: Self, next: Self) {
		GCMethods::post_barrier(&mut (*v).arr, prev.arr, next.arr)
	}
}

impl Deref for Array {
	type Target = *mut JSObject;

	fn deref(&self) -> &Self::Target {
		&self.arr
	}
}

impl DerefMut for Array {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.arr
	}
}

impl FromValue for Array {
	fn from_value<'c, 's: 'c>(cx: &Context<'c>, value: Local<'s, Value>) -> Result<Local<'c, Self>, ()> {
		if !value.is_object() {
			unsafe { throw_type_error(cx.cx(), "Value is not an object") };
			return Err(());
		}

		let object = value.to_object();
		unsafe { AssertSameCompartment(cx.cx(), object) };
		if unsafe { !Array::is_array_raw(cx, object) } {
			unsafe { throw_type_error(cx.cx(), "Value is not an array") };
			return Err(());
		}

		Array::from_raw(cx, object).ok_or(())
	}
}

impl ToValue for Array {
	fn to_value<'c, 's: 'c>(this: Local<'s, Self>, cx: &Context<'c>) -> Local<'c, Value> {
		this.to_value(cx)
	}
}
