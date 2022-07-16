/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use mozjs::jsapi::JS;
use mozjs::jsval::{DoubleValue, Int32Value, JSVal};
use mozjs::rust::{Handle, ToNumber};
use mozjs_sys::jsgc::{GCMethods, RootKind};

use crate::{Context, Local, Value};
use crate::value::{FromValue, FromValueNative, ToValue, ToValueNative};

pub struct Number {
	num: Value,
}

impl Number {
	pub fn new<'c>(cx: &Context<'c>, number: f64) -> Local<'c, Number> {
		Number::from_raw(cx, DoubleValue(number))
	}

	pub fn new_i32<'c>(cx: &Context<'c>, number: i32) -> Local<'c, Number> {
		Number::from_raw(cx, Int32Value(number))
	}

	pub(crate) fn from_raw<'c>(cx: &Context<'c>, val: JSVal) -> Local<'c, Number> {
		Local::new(cx, Number { num: Value { val } })
	}

	pub fn to_f64(&self) -> f64 {
		self.num.val.to_number()
	}

	pub fn to_i32(&self) -> i32 {
		self.num.val.to_int32()
	}
}

impl RootKind for Number {
	#[allow(non_snake_case)]
	fn rootKind() -> JS::RootKind {
		JS::RootKind::Value
	}
}

impl GCMethods for Number {
	unsafe fn initial() -> Self {
		Number { num: GCMethods::initial() }
	}

	unsafe fn post_barrier(v: *mut Self, prev: Self, next: Self) {
		GCMethods::post_barrier(&mut (*v).num, prev.num, next.num)
	}
}

impl FromValue for Number {
	fn from_value<'c, 's: 'c>(cx: &Context<'c>, value: Local<'s, Value>) -> Result<Local<'c, Self>, ()> {
		f64::from_value_native(cx, value).map(|f| Number::new(cx, f))
	}
}

impl ToValue for Number {
	fn to_value<'c, 's: 'c>(this: Local<'s, Self>, cx: &Context<'c>) -> Local<'c, Value> {
		Value::from_raw(cx, this.num.val)
	}
}

impl FromValueNative for f64 {
	fn from_value_native(cx: &Context, value: Local<Value>) -> Result<Self, ()> {
		unsafe { ToNumber(cx.cx(), Handle::from_marked_location(&**value)) }
	}
}

impl ToValueNative for f64 {
	fn to_value_native<'c, 's: 'c>(&self, cx: &Context<'c>) -> Local<'c, Value> {
		Number::to_value(Number::new(cx, *self), cx)
	}
}

impl ToValueNative for i32 {
	fn to_value_native<'c, 's: 'c>(&self, cx: &Context<'c>) -> Local<'c, Value> {
		Number::to_value(Number::new_i32(cx, *self), cx)
	}
}
