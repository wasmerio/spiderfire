/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use mozjs::jsapi::JS;
use mozjs::jsval::{BooleanValue, JSVal};
use mozjs::rust::{Handle, ToBoolean};
use mozjs_sys::jsgc::{GCMethods, RootKind};

use crate::{Context, Local, Value};
use crate::value::{FromValue, FromValueNative, ToValue, ToValueNative};

pub struct Boolean {
	bool: Value,
}

impl Boolean {
	pub fn new<'c>(cx: &Context<'c>, bool: bool) -> Local<'c, Boolean> {
		Boolean::from_raw(cx, BooleanValue(bool))
	}

	pub(crate) fn from_raw<'c>(cx: &Context<'c>, val: JSVal) -> Local<'c, Boolean> {
		Local::new(cx, Boolean { bool: Value { val } })
	}
}

impl RootKind for Boolean {
	#[allow(non_snake_case)]
	fn rootKind() -> JS::RootKind {
		JS::RootKind::Value
	}
}

impl GCMethods for Boolean {
	unsafe fn initial() -> Self {
		Boolean { bool: GCMethods::initial() }
	}

	unsafe fn post_barrier(v: *mut Self, prev: Self, next: Self) {
		GCMethods::post_barrier(&mut (*v).bool, prev.bool, next.bool)
	}
}

impl FromValue for Boolean {
	fn from_value<'c, 's: 'c>(cx: &Context<'c>, value: Local<'s, Value>) -> Result<Local<'c, Self>, ()> {
		bool::from_value_native(cx, value).map(|b| Boolean::new(cx, b))
	}
}

impl ToValue for Boolean {
	fn to_value<'c, 's: 'c>(this: Local<'s, Self>, cx: &Context<'c>) -> Local<'c, Value> {
		Value::from_raw(cx, this.bool.val)
	}
}

impl FromValueNative for bool {
	fn from_value_native(_: &Context, value: Local<Value>) -> Result<Self, ()> {
		Ok(unsafe { ToBoolean(Handle::from_marked_location(&**value)) })
	}
}

impl ToValueNative for bool {
	fn to_value_native<'c, 's: 'c>(&self, cx: &Context<'c>) -> Local<'c, Value> {
		Boolean::to_value(Boolean::new(cx, *self), cx)
	}
}
