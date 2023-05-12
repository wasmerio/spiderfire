/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::iter::Iterator;
use std::string::String as RustString;

use mozjs::conversions::{ConversionResult, FromJSValConvertible};
pub use mozjs::conversions::ConversionBehavior;
use mozjs::jsapi::{AssertSameCompartment, AssertSameCompartment1, JSFunction, JSObject, JSString};
use mozjs::jsapi::Symbol as JSSymbol;
use mozjs::jsval::JSVal;
use mozjs::rust::{ToBoolean, ToNumber, ToString};

use crate::{Array, Context, Date, Error, ErrorKind, ForOfIterator, Function, Object, Promise, Result, String, Symbol, Value};

pub trait FromValue<'cx>: Sized {
	type Config;
	unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, strict: bool, config: Self::Config) -> Result<Self>
	where
		'cx: 'v;
}

impl<'cx> FromValue<'cx> for bool {
	type Config = ();

	unsafe fn from_value<'v>(_: &'cx Context, value: &Value<'v>, strict: bool, _: ()) -> Result<bool>
	where
		'cx: 'v,
	{
		if value.is_boolean() {
			return Ok(value.to_boolean());
		}

		if strict {
			Err(Error::new("Expected Boolean in Strict Conversion", ErrorKind::Type))
		} else {
			Ok(ToBoolean(value.handle()))
		}
	}
}

macro_rules! impl_from_value_for_integer {
	($ty:ty) => {
		impl<'cx> FromValue<'cx> for $ty {
			type Config = ConversionBehavior;

			unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, strict: bool, config: ConversionBehavior) -> Result<$ty>
			where
				'cx: 'v,
			{
				if strict && !value.is_number() {
					return Err(Error::new("Expected Number in Strict Conversion", ErrorKind::Type));
				}

				match <$ty>::from_jsval(**cx, value.handle(), config) {
					Ok(ConversionResult::Success(number)) => Ok(number),
					Err(_) => Err(Error::none()),
					_ => unreachable!(),
				}
			}
		}
	};
}

impl_from_value_for_integer!(u8);
impl_from_value_for_integer!(u16);
impl_from_value_for_integer!(u32);
impl_from_value_for_integer!(u64);

impl_from_value_for_integer!(i8);
impl_from_value_for_integer!(i16);
impl_from_value_for_integer!(i32);
impl_from_value_for_integer!(i64);

impl<'cx> FromValue<'cx> for f32 {
	type Config = ();

	unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, strict: bool, _: ()) -> Result<f32>
	where
		'cx: 'v,
	{
		f64::from_value(cx, value, strict, ()).map(|float| float as f32)
	}
}

impl<'cx> FromValue<'cx> for f64 {
	type Config = ();

	unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, strict: bool, _: ()) -> Result<f64>
	where
		'cx: 'v,
	{
		if strict && !value.is_number() {
			return Err(Error::new("Expected Number in Strict Conversion", ErrorKind::Type));
		}

		ToNumber(**cx, value.handle()).map_err(|_| Error::new("Unable to Convert Value to Number", ErrorKind::Type))
	}
}

impl<'cx> FromValue<'cx> for *mut JSString {
	type Config = ();

	unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, strict: bool, _: ()) -> Result<*mut JSString>
	where
		'cx: 'v,
	{
		if strict && !value.is_string() {
			return Err(Error::new("Expected String in Strict Conversion", ErrorKind::Type));
		}
		Ok(ToString(**cx, value.handle()))
	}
}

impl<'cx> FromValue<'cx> for String<'cx> {
	type Config = ();

	unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, strict: bool, config: ()) -> Result<String<'cx>>
	where
		'cx: 'v,
	{
		<*mut JSString>::from_value(cx, value, strict, config).map(|str| String::from(cx.root_string(str)))
	}
}

impl<'cx> FromValue<'cx> for RustString {
	type Config = ();

	unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, strict: bool, config: ()) -> Result<RustString>
	where
		'cx: 'v,
	{
		// TODO: Replace with Result::flatten once stabilised
		String::from_value(cx, value, strict, config)
			.map(|s| s.to_owned_string(cx).ok_or_else(|| Error::new("Expected Linear String", ErrorKind::Type)))?
	}
}

impl<'cx> FromValue<'cx> for *mut JSObject {
	type Config = ();

	unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, _: bool, _: ()) -> Result<*mut JSObject>
	where
		'cx: 'v,
	{
		if !value.is_object() {
			return Err(Error::new("Expected Object", ErrorKind::Type));
		}
		let object = (**value).to_object();
		AssertSameCompartment(**cx, object);

		Ok(object)
	}
}

impl<'cx> FromValue<'cx> for Object<'cx> {
	type Config = ();

	unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, _: bool, _: ()) -> Result<Object<'cx>>
	where
		'cx: 'v,
	{
		if !value.is_object() {
			return Err(Error::new("Expected Object", ErrorKind::Type));
		}
		let object = value.to_object(cx);
		AssertSameCompartment(**cx, **object);

		Ok(object)
	}
}

impl<'cx> FromValue<'cx> for Array<'cx> {
	type Config = ();

	unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, _: bool, _: ()) -> Result<Array<'cx>>
	where
		'cx: 'v,
	{
		if !value.is_object() {
			return Err(Error::new("Expected Array", ErrorKind::Type));
		}

		let object = value.to_object(cx).into_local();
		if let Some(array) = Array::from(cx, object) {
			AssertSameCompartment(**cx, **array);
			Ok(array)
		} else {
			Err(Error::new("Expected Array", ErrorKind::Type))
		}
	}
}

impl<'cx> FromValue<'cx> for Date<'cx> {
	type Config = ();

	unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, _: bool, _: ()) -> Result<Date<'cx>>
	where
		'cx: 'v,
	{
		if !value.is_object() {
			return Err(Error::new("Expected Date", ErrorKind::Type));
		}

		let object = value.to_object(cx).into_local();
		if let Some(date) = Date::from(cx, object) {
			AssertSameCompartment(**cx, **date);
			Ok(date)
		} else {
			Err(Error::new("Expected Date", ErrorKind::Type))
		}
	}
}

impl<'cx> FromValue<'cx> for Promise<'cx> {
	type Config = ();

	unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, _: bool, _: ()) -> Result<Promise<'cx>>
	where
		'cx: 'v,
	{
		if !value.is_object() {
			return Err(Error::new("Expected Promise", ErrorKind::Type));
		}

		let object = value.to_object(cx).into_local();
		if let Some(promise) = Promise::from(object) {
			AssertSameCompartment(**cx, **promise);
			Ok(promise)
		} else {
			Err(Error::new("Expected Promise", ErrorKind::Type))
		}
	}
}

impl<'cx> FromValue<'cx> for *mut JSFunction {
	type Config = ();

	unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, strict: bool, config: ()) -> Result<*mut JSFunction>
	where
		'cx: 'v,
	{
		Function::from_value(cx, value, strict, config).map(|f| **f)
	}
}

impl<'cx> FromValue<'cx> for Function<'cx> {
	type Config = ();

	unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, _: bool, _: ()) -> Result<Function<'cx>>
	where
		'cx: 'v,
	{
		if !value.is_object() {
			return Err(Error::new("Expected Function", ErrorKind::Type));
		}

		let function_obj = value.to_object(cx);
		if let Some(function) = Function::from_object(cx, &function_obj) {
			AssertSameCompartment(**cx, **function_obj);
			Ok(function)
		} else {
			Err(Error::new("Expected Function", ErrorKind::Type))
		}
	}
}

impl<'cx> FromValue<'cx> for *mut JSSymbol {
	type Config = ();

	unsafe fn from_value<'v>(_: &'cx Context, value: &Value<'v>, _: bool, _: ()) -> Result<*mut JSSymbol>
	where
		'cx: 'v,
	{
		if value.is_symbol() {
			Ok(value.to_symbol())
		} else {
			Err(Error::new("Expected Symbol", ErrorKind::Type))
		}
	}
}

impl<'cx> FromValue<'cx> for Symbol<'cx> {
	type Config = ();

	unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, strict: bool, config: Self::Config) -> Result<Symbol<'cx>>
	where
		'cx: 'v,
	{
		<*mut JSSymbol>::from_value(cx, value, strict, config).map(|s| cx.root_symbol(s).into())
	}
}

impl<'cx> FromValue<'cx> for JSVal {
	type Config = ();

	unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, _: bool, _: ()) -> Result<JSVal>
	where
		'cx: 'v,
	{
		AssertSameCompartment1(**cx, value.handle().into());
		Ok(***value)
	}
}

impl<'cx> FromValue<'cx> for Value<'cx> {
	type Config = ();

	unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, _: bool, _: ()) -> Result<Value<'cx>>
	where
		'cx: 'v,
	{
		AssertSameCompartment1(**cx, value.handle().into());
		Ok(cx.root_value(***value).into())
	}
}

impl<'cx, T: FromValue<'cx>> FromValue<'cx> for Option<T> {
	type Config = T::Config;

	unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, strict: bool, config: T::Config) -> Result<Option<T>>
	where
		'cx: 'v,
	{
		if value.is_null_or_undefined() {
			Ok(None)
		} else {
			Ok(Some(T::from_value(cx, value, strict, config)?))
		}
	}
}

impl<'cx, T: FromValue<'cx>> FromValue<'cx> for Vec<T>
where
	T::Config: Clone,
{
	type Config = T::Config;

	unsafe fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, strict: bool, config: T::Config) -> Result<Vec<T>>
	where
		'cx: 'v,
	{
		if !value.is_object() {
			return Err(Error::new("Expected Object", ErrorKind::Type));
		}
		let object = value.to_object(cx);
		if strict && !Array::is_array(cx, &object) {
			return Err(Error::new("Expected Array", ErrorKind::Type));
		}

		let iterator = ForOfIterator::from_object(cx, &object)?;
		iterator.map(|value| T::from_value(cx, &value?, strict, config.clone())).collect()
	}
}
