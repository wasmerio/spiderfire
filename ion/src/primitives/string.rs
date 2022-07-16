/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::ops::{Deref, DerefMut};
use std::string;

use mozjs::conversions::jsstr_to_string;
use mozjs::jsapi::{JS, JS_GetEmptyString, JS_GetStringCharAt, JS_GetStringLength, JS_NewUCStringCopyN, JSString};
use mozjs::jsval::StringValue;
use mozjs::rust::{Handle, ToString};
use mozjs::rust::jsapi_wrapped::JS_ConcatStrings;
use mozjs_sys::jsgc::{GCMethods, RootKind};

use crate::{Context, Local, Value};
use crate::value::{FromValue, FromValueNative, ToValue, ToValueNative};

pub struct String {
	str: *mut JSString,
}

impl String {
	pub fn new<'c>(cx: &Context<'c>) -> Local<'c, String> {
		let str = unsafe { JS_GetEmptyString(cx.cx()) };
		Local::new(cx, String { str })
	}

	pub fn from_str<'c>(cx: &Context<'c>, str: &str) -> Local<'c, String> {
		let mut string: Vec<u16> = Vec::with_capacity(str.len());
		string.extend(str.encode_utf16());
		let jsstr = unsafe { JS_NewUCStringCopyN(cx.cx(), string.as_ptr(), string.len()) };
		if jsstr.is_null() {
			panic!("Failed to create string. ({})", str);
		}
		String::from_raw(cx, jsstr)
	}

	pub(crate) fn from_raw<'c>(cx: &Context<'c>, str: *mut JSString) -> Local<'c, String> {
		Local::new(cx, String { str })
	}

	pub fn to_string(&self, cx: &Context) -> string::String {
		unsafe { jsstr_to_string(cx.cx(), self.str) }
	}

	pub fn to_value<'c>(&self, cx: &Context<'c>) -> Local<'c, Value> {
		Value::from_raw(cx, StringValue(unsafe { &*self.str }))
	}

	#[allow(clippy::len_without_is_empty)]
	pub fn len(&self) -> usize {
		unsafe { JS_GetStringLength(self.str) }
	}

	pub fn char_at(&self, cx: &Context, index: usize) -> Option<char> {
		let mut char = 0;
		let result = unsafe { JS_GetStringCharAt(cx.cx(), self.str, index, &mut char) };
		result.then(|| char::from_u32(char as u32)).flatten()
	}

	pub fn concat<'c>(&self, cx: &Context<'c>, other: Local<String>) -> Local<'c, String> {
		let handle = unsafe { Handle::from_marked_location(&self.str) };
		let other = unsafe { Handle::from_marked_location(&other.str) };
		let jsstr = unsafe { JS_ConcatStrings(cx.cx(), handle, other) };
		Local::new(cx, String { str: jsstr })
	}
}

impl RootKind for String {
	#[allow(non_snake_case)]
	fn rootKind() -> JS::RootKind {
		JS::RootKind::String
	}
}

impl GCMethods for String {
	unsafe fn initial() -> Self {
		String { str: GCMethods::initial() }
	}

	unsafe fn post_barrier(v: *mut Self, prev: Self, next: Self) {
		GCMethods::post_barrier(&mut (*v).str, prev.str, next.str)
	}
}

impl Deref for String {
	type Target = *mut JSString;

	fn deref(&self) -> &Self::Target {
		&self.str
	}
}

impl DerefMut for String {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.str
	}
}

impl FromValue for String {
	fn from_value<'c, 's: 'c>(cx: &Context<'c>, value: Local<'s, Value>) -> Result<Local<'c, Self>, ()> {
		Ok(String::from_raw(cx, unsafe { ToString(cx.cx(), Handle::from_marked_location(&**value)) }))
	}
}

impl ToValue for String {
	fn to_value<'c, 's: 'c>(this: Local<'s, Self>, cx: &Context<'c>) -> Local<'c, Value> {
		this.to_value(cx)
	}
}

impl FromValueNative for string::String {
	fn from_value_native(cx: &Context, value: Local<Value>) -> Result<Self, ()> {
		Ok(unsafe { jsstr_to_string(cx.cx(), ToString(cx.cx(), Handle::from_marked_location(&**value))) })
	}
}

impl ToValueNative for string::String {
	fn to_value_native<'c, 's: 'c>(&self, cx: &Context<'c>) -> Local<'c, Value> {
		String::from_str(cx, self).to_value(cx)
	}
}
