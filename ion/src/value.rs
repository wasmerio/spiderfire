/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::ops::{Deref, DerefMut};

use mozjs::jsapi::SameValue;
use mozjs::jsval::{BigIntValue, BooleanValue, DoubleValue, Int32Value, JSVal, NullValue, ObjectValue, SymbolValue, UInt32Value, UndefinedValue};

use crate::{Array, Context, Local, Object, Symbol};
use crate::bigint::BigInt;
use crate::conversions::ToValue;

/// Represents a JavaScript Value in the runtime.
/// It can represent either a primitive or an object.
#[derive(Debug)]
pub struct Value<'v> {
	val: Local<'v, JSVal>,
}

impl<'v> Value<'v> {
	/// Creates a [Value] from a boolean.
	pub fn bool(cx: &Context, b: bool) -> Value {
		Value::from(cx.root_value(BooleanValue(b)))
	}

	/// Creates a [Value] from a 32-bit signed integer.
	pub fn i32(cx: &Context, i: i32) -> Value {
		Value::from(cx.root_value(Int32Value(i)))
	}

	/// Creates a [Value] from a 32-bit unsigned integer.
	pub fn u32(cx: &Context, u: u32) -> Value {
		Value::from(cx.root_value(UInt32Value(u)))
	}

	/// Creates a [Value] from a 64-bit float.
	pub fn f64(cx: &Context, f: f64) -> Value {
		Value::from(cx.root_value(DoubleValue(f)))
	}

	/// Creates a [Value] from a string.
	pub fn string(cx: &'v Context, str: &str) -> Value<'v> {
		str.as_value(cx)
	}

	/// Creates a [Value] from a [BigInt].
	pub fn bigint<'cx>(cx: &'cx Context, bi: &BigInt) -> Value<'cx> {
		Value::from(cx.root_value(BigIntValue(unsafe { &*bi.get() })))
	}

	/// Creates a [Value] from a [Symbol].
	pub fn symbol<'cx>(cx: &'cx Context, sym: &Symbol) -> Value<'cx> {
		Value::from(cx.root_value(SymbolValue(unsafe { &*sym.get() })))
	}

	/// Creates a [Value] from an [Object].
	pub fn object<'cx>(cx: &'cx Context, object: &Object) -> Value<'cx> {
		Value::from(cx.root_value(ObjectValue(object.handle().get())))
	}

	/// Creates a [Value] from an [Array].
	pub fn array<'cx>(cx: &'cx Context, array: &Array) -> Value<'cx> {
		Value::from(cx.root_value(ObjectValue(array.handle().get())))
	}

	/// Creates an `undefined` [Value].
	pub fn undefined(cx: &Context) -> Value {
		Value::from(cx.root_value(UndefinedValue()))
	}

	/// Creates a `null` [Value].
	pub fn null(cx: &Context) -> Value {
		Value::from(cx.root_value(NullValue()))
	}

	/// Converts a [Value] to an [Object] that may not be null.
	///
	/// ### Panics
	/// This panics if the [Value] is not an object or it is null.
	pub fn to_object<'cx>(&self, cx: &'cx Context) -> Object<'cx> {
		cx.root_object(self.handle().to_object()).into()
	}

	/// Converts a [Value] to an [Object] that may be null.
	///
	/// ### Panics
	/// This panics if the [Value] is not an object.
	pub fn to_object_or_null<'cx>(&self, cx: &'cx Context) -> Object<'cx> {
		cx.root_object(self.handle().to_object_or_null()).into()
	}

	/// Compares two values for equality using the [SameValue algorithm](https://tc39.es/ecma262/multipage/abstract-operations.html#sec-samevalue).
	/// This is identical to strict equality (===), except that NaN's are equal and 0 !== -0.
	pub fn is_same(&self, cx: &Context, other: &Value) -> bool {
		let mut same = false;
		unsafe { SameValue(cx.as_ptr(), self.handle().into(), other.handle().into(), &mut same) && same }
	}

	pub fn into_local(self) -> Local<'v, JSVal> {
		self.val
	}
}

impl<'v> From<Local<'v, JSVal>> for Value<'v> {
	fn from(val: Local<'v, JSVal>) -> Value<'v> {
		Value { val }
	}
}

impl<'v> Deref for Value<'v> {
	type Target = Local<'v, JSVal>;

	fn deref(&self) -> &Self::Target {
		&self.val
	}
}

impl<'v> DerefMut for Value<'v> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.val
	}
}
