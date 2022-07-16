/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::ops::{Deref, DerefMut};

use mozjs::jsapi::{JS, JSString};
use mozjs::jsval::{JSVal, NullValue, UndefinedValue};
use mozjs_sys::jsgc::{GCMethods, RootKind};

use crate::context::{Context, Local};

#[derive(Clone, Debug)]
pub struct Value {
	pub(crate) val: JSVal,
}

impl Value {
	pub(crate) fn from_raw<'c>(cx: &Context<'c>, val: JSVal) -> Local<'c, Value> {
		Local::new(cx, Value { val })
	}

	pub fn is_boolean(&self) -> bool {
		self.val.is_boolean()
	}

	pub fn is_number(&self) -> bool {
		self.val.is_number()
	}

	pub fn is_string(&self) -> bool {
		self.val.is_string()
	}

	pub fn is_symbol(&self) -> bool {
		self.val.is_symbol()
	}

	pub fn is_null(&self) -> bool {
		self.val.is_null()
	}

	pub fn is_undefined(&self) -> bool {
		self.val.is_undefined()
	}

	pub fn is_object(&self) -> bool {
		self.val.is_object()
	}

	// TODO: IMPROVE
	pub fn to_boolean(&self) -> bool {
		self.val.to_boolean()
	}

	pub fn to_number(&self) -> f64 {
		self.val.to_number()
	}

	pub fn to_jsstr(&self) -> *mut JSString {
		self.val.to_string()
	}

	pub fn null<'c, 's: 'c>(cx: &'c Context<'s>) -> Local<'s, Value> {
		Local::new(cx, Value { val: NullValue() })
	}

	pub fn undefined<'c, 's: 'c>(cx: &'c Context<'s>) -> Local<'s, Value> {
		Local::new(cx, Value { val: UndefinedValue() })
	}
}

impl RootKind for Value {
	#[allow(non_snake_case)]
	fn rootKind() -> JS::RootKind {
		JS::RootKind::Value
	}
}

impl GCMethods for Value {
	unsafe fn initial() -> Self {
		Value { val: GCMethods::initial() }
	}

	unsafe fn post_barrier(v: *mut Self, prev: Self, next: Self) {
		GCMethods::post_barrier(&mut (*v).val, prev.val, next.val)
	}
}

impl Deref for Value {
	type Target = JSVal;

	fn deref(&self) -> &Self::Target {
		&self.val
	}
}

impl DerefMut for Value {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.val
	}
}

pub trait FromValue
where
	Self: RootKind + GCMethods + Sized,
{
	fn from_value<'c, 's: 'c>(cx: &Context<'c>, value: Local<'s, Value>) -> Result<Local<'c, Self>, ()>;
}

pub trait ToValue
where
	Self: RootKind + GCMethods + Sized,
{
	fn to_value<'c, 's: 'c>(this: Local<'s, Self>, cx: &Context<'c>) -> Local<'c, Value>;
}

pub trait FromValueNative
where
	Self: Sized,
{
	fn from_value_native(cx: &Context, value: Local<Value>) -> Result<Self, ()>;
}

pub trait ToValueNative
where
	Self: Sized,
{
	fn to_value_native<'c, 's: 'c>(&self, cx: &Context<'c>) -> Local<'c, Value>;
}
