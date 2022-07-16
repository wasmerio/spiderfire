/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::ops::{Deref, DerefMut};

use mozjs::conversions::jsstr_to_string;
use mozjs::error::throw_type_error;
use mozjs::jsapi::{
	AssertSameCompartment, HandleValueArray, JS, JS_GetFunctionArity, JS_GetFunctionDisplayId, JS_GetFunctionId, JS_GetFunctionObject,
	JS_GetObjectFunction, JS_NewFunction, JS_ObjectIsFunction, JSContext, JSFunction, JSFunctionSpec, JSObject, NewFunctionFromSpec1,
};
use mozjs::jsval::JSVal;
use mozjs::rust::{Handle, MutableHandle};
use mozjs::rust::jsapi_wrapped::{JS_CallFunction, JS_DecompileFunction, JS_GetFunctionLength};
use mozjs_sys::jsgc::{GCMethods, RootKind};

use crate::{Context, Local, Object, Value};
use crate::exception::{ErrorReport, Exception};
use crate::value::{FromValue, ToValue};

pub type NativeFunction = unsafe extern "C" fn(*mut JSContext, u32, *mut JSVal) -> bool;

#[derive(Clone, Debug)]
pub struct Function {
	pub(crate) func: *mut JSFunction,
}

impl Function {
	pub fn new<'c>(cx: &Context<'c>, name: &str, func: Option<NativeFunction>, nargs: u32, flags: u32) -> Local<'c, Function> {
		let name = format!("{}\0", name);
		Function::from_raw(cx, unsafe { JS_NewFunction(cx.cx(), func, nargs, flags, name.as_ptr() as *const i8) })
	}

	pub fn from_spec<'c>(cx: &Context<'c>, spec: *const JSFunctionSpec) -> Local<'c, Function> {
		Function::from_raw(cx, unsafe { NewFunctionFromSpec1(cx.cx(), spec) })
	}

	pub unsafe fn from_object_raw<'c>(cx: &Context<'c>, obj: *mut JSObject) -> Option<Local<'c, Function>> {
		if Function::is_function_raw(obj) {
			Some(Function::from_raw(cx, JS_GetObjectFunction(obj)))
		} else {
			None
		}
	}

	pub fn from_raw<'c>(cx: &Context<'c>, func: *mut JSFunction) -> Local<'c, Function> {
		Local::new(cx, Function { func })
	}

	pub fn to_object<'c>(&self, cx: &Context<'c>) -> Local<'c, Object> {
		Object::from_raw(cx, unsafe { JS_GetFunctionObject(self.func) })
	}

	pub fn to_string(&self, cx: &Context) -> String {
		unsafe {
			let handle = Handle::from_marked_location(&self.func);
			let str = JS_DecompileFunction(cx.cx(), handle);
			jsstr_to_string(cx.cx(), str)
		}
	}

	pub fn name(&self, cx: &Context) -> Option<String> {
		let id = unsafe { JS_GetFunctionId(self.func) };
		if !id.is_null() {
			Some(unsafe { jsstr_to_string(cx.cx(), id) })
		} else {
			None
		}
	}

	pub fn display_name(&self, cx: &Context) -> Option<String> {
		let id = unsafe { JS_GetFunctionDisplayId(self.func) };
		if !id.is_null() {
			Some(unsafe { jsstr_to_string(cx.cx(), id) })
		} else {
			None
		}
	}

	pub fn nargs(&self) -> u16 {
		unsafe { JS_GetFunctionArity(self.func) }
	}

	pub fn length(&self, cx: &Context) -> Option<u16> {
		let handle = unsafe { Handle::from_marked_location(&self.func) };
		let mut length = 0;
		if unsafe { JS_GetFunctionLength(cx.cx(), handle, &mut length) } {
			Some(length)
		} else {
			None
		}
	}

	pub fn call<'c>(
		&self, cx: &Context<'c>, this: &Local<'c, Object>, args: Vec<&Local<'c, Value>>,
	) -> Result<Local<'c, Value>, Option<ErrorReport>> {
		let values = args.iter().map(|v| ****v).collect::<Vec<JSVal>>();
		self.call_handle(cx, this, unsafe { HandleValueArray::from_rooted_slice(&values) })
	}

	pub fn call_handle<'c>(
		&self, cx: &Context<'c>, this: &Local<'c, Object>, args: HandleValueArray,
	) -> Result<Local<'c, Value>, Option<ErrorReport>> {
		let handle = unsafe { Handle::from_marked_location(&self.func) };
		let this = unsafe { Handle::from_marked_location(&(*this).obj) };
		let mut rval = Value::undefined(cx);
		let mut rval_handle = unsafe { MutableHandle::from_marked_location(&mut rval.val) };

		if unsafe { JS_CallFunction(cx.cx(), this, handle, &args, &mut rval_handle) } {
			Ok(rval)
		} else if let Some(exception) = Exception::new(cx) {
			Err(Some(ErrorReport::new(exception)))
		} else {
			Err(None)
		}
	}

	pub(crate) unsafe fn is_function_raw(obj: *mut JSObject) -> bool {
		JS_ObjectIsFunction(obj)
	}
}

impl RootKind for Function {
	#[allow(non_snake_case)]
	fn rootKind() -> JS::RootKind {
		JS::RootKind::Object
	}
}

impl GCMethods for Function {
	unsafe fn initial() -> Self {
		Function { func: GCMethods::initial() }
	}

	unsafe fn post_barrier(v: *mut Self, prev: Self, next: Self) {
		GCMethods::post_barrier(&mut (*v).func, prev.func, next.func)
	}
}

impl Deref for Function {
	type Target = *mut JSFunction;

	fn deref(&self) -> &Self::Target {
		&self.func
	}
}

impl DerefMut for Function {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.func
	}
}

impl FromValue for Function {
	fn from_value<'c, 's: 'c>(cx: &Context<'c>, value: Local<'s, Value>) -> Result<Local<'c, Self>, ()> {
		if !value.is_object() && unsafe { !Function::is_function_raw((**value).to_object()) } {
			unsafe { throw_type_error(cx.cx(), "Value is not a function") };
			return Err(());
		}

		let object = value.to_object();
		unsafe { AssertSameCompartment(cx.cx(), object) };
		Ok(unsafe { Function::from_object_raw(cx, object).unwrap() })
	}
}

impl ToValue for Function {
	fn to_value<'c, 's: 'c>(this: Local<'s, Self>, cx: &Context<'c>) -> Local<'c, Value> {
		this.to_object(cx).to_value(cx)
	}
}
