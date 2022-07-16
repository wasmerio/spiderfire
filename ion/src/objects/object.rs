/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::ops::{Deref, DerefMut};

use mozjs::conversions::jsstr_to_string;
use mozjs::error::throw_type_error;
use mozjs::glue::{RUST_JSID_IS_INT, RUST_JSID_IS_STRING, RUST_JSID_TO_INT, RUST_JSID_TO_STRING};
use mozjs::jsapi::{AssertSameCompartment, CurrentGlobalOrNull, ESClass, JS, JS_NewPlainObject, JSObject, ObjectOpResult};
use mozjs::jsval::ObjectValue;
use mozjs::rust::{Handle, HandleObject, IdVector, MutableHandle};
use mozjs::rust::jsapi_wrapped::{
	GetBuiltinClass, GetPropertyKeys, JS_DefineProperty, JS_DeleteProperty, JS_GetProperty, JS_HasOwnProperty, JS_HasProperty, JS_SetProperty,
};
use mozjs_sys::jsgc::{GCMethods, RootKind};

use crate::context::{Context, Local};
use crate::exception::Exception;
use crate::flags::{IteratorFlags, PropertyFlags};
use crate::objects::Key;
use crate::value::{FromValue, ToValue, Value};

#[derive(Clone, Debug)]
pub struct Object {
	pub(crate) obj: *mut JSObject,
}

impl Object {
	pub fn new<'c>(cx: &Context<'c>) -> Local<'c, Object> {
		Object::from_raw(cx, unsafe { JS_NewPlainObject(cx.cx()) })
	}

	pub fn null<'c>(cx: &Context<'c>) -> Local<'c, Object> {
		Object::from_raw(cx, HandleObject::null().get())
	}

	pub fn from_raw<'c>(cx: &Context<'c>, obj: *mut JSObject) -> Local<'c, Object> {
		Local::new(cx, Object { obj })
	}

	pub fn to_value<'c>(&self, cx: &Context<'c>) -> Local<'c, Value> {
		Value::from_raw(cx, ObjectValue(self.obj))
	}

	pub fn has(&self, cx: &Context, key: &str) -> bool {
		let key = format!("{}\0", key);
		let handle = unsafe { Handle::from_marked_location(&self.obj) };
		let mut found = false;

		if unsafe { JS_HasProperty(cx.cx(), handle, key.as_ptr() as *const i8, &mut found) } {
			found
		} else {
			Exception::clear(cx);
			false
		}
	}

	pub fn has_own(&self, cx: &Context, key: &str) -> bool {
		let key = format!("{}\0", key);
		let handle = unsafe { Handle::from_marked_location(&self.obj) };
		let mut found = false;

		if unsafe { JS_HasOwnProperty(cx.cx(), handle, key.as_ptr() as *const i8, &mut found) } {
			found
		} else {
			Exception::clear(cx);
			false
		}
	}

	pub fn get<'c>(&self, cx: &Context<'c>, key: &str) -> Option<Local<'c, Value>> {
		let key = format!("{}\0", key);

		if self.has(cx, &key) {
			let handle = unsafe { Handle::from_marked_location(&self.obj) };
			let mut val = Value::undefined(cx);
			unsafe {
				JS_GetProperty(
					cx.cx(),
					handle,
					key.as_ptr() as *const i8,
					&mut MutableHandle::from_marked_location(&mut **val),
				)
			};
			Some(val)
		} else {
			None
		}
	}

	pub fn set(&mut self, cx: &Context, key: &str, value: Local<Value>) -> bool {
		let key = format!("{}\0", key);
		let handle = unsafe { Handle::from_marked_location(&self.obj) };

		unsafe { JS_SetProperty(cx.cx(), handle, key.as_ptr() as *const i8, Handle::from_marked_location(&**value)) }
	}

	pub fn define(&mut self, cx: &Context, key: &str, value: Local<Value>, attrs: PropertyFlags) -> bool {
		let key = format!("{}\0", key);
		let handle = unsafe { Handle::from_marked_location(&self.obj) };

		unsafe {
			JS_DefineProperty(
				cx.cx(),
				handle,
				key.as_ptr() as *const i8,
				Handle::from_marked_location(&**value),
				attrs.bits() as u32,
			)
		}
	}

	pub fn delete(&self, cx: &Context, key: &str) -> (bool, ObjectOpResult) {
		let key = format!("{}\0", key);
		let handle = unsafe { Handle::from_marked_location(&self.obj) };

		let mut result = ObjectOpResult { code_: 0 };

		(
			unsafe { JS_DeleteProperty(cx.cx(), handle, key.as_ptr() as *const i8, &mut result) },
			result,
		)
	}

	pub fn keys(&self, cx: &Context, flags: Option<IteratorFlags>) -> Vec<Key> {
		let flags = flags.unwrap_or(IteratorFlags::OWN_ONLY);
		let handle = unsafe { Handle::from_marked_location(&self.obj) };
		let mut ids = unsafe { IdVector::new(cx.cx()) };

		unsafe { GetPropertyKeys(cx.cx(), handle, flags.bits(), ids.handle_mut()) };
		ids.iter()
			.map(|id| {
				rooted!(in(cx.cx()) let id = *id);
				unsafe {
					if RUST_JSID_IS_INT(id.handle().into()) {
						Key::Int(RUST_JSID_TO_INT(id.handle().into()))
					} else if RUST_JSID_IS_STRING(id.handle().into()) {
						Key::String(jsstr_to_string(cx.cx(), RUST_JSID_TO_STRING(id.handle().into())))
					} else {
						Key::Void
					}
				}
			})
			.collect()
	}

	pub fn get_builtin_class(&self, cx: &Context) -> Option<ESClass> {
		let handle = unsafe { Handle::from_marked_location(&self.obj) };
		let mut class = ESClass::Other;

		if unsafe { !GetBuiltinClass(cx.cx(), handle, &mut class) } {
			None
		} else {
			Some(class)
		}
	}

	pub fn global<'c>(cx: &Context<'c>) -> Local<'c, Object> {
		Object::from_raw(cx, unsafe { CurrentGlobalOrNull(cx.cx()) })
	}
}

impl RootKind for Object {
	#[allow(non_snake_case)]
	fn rootKind() -> JS::RootKind {
		JS::RootKind::Object
	}
}

impl GCMethods for Object {
	unsafe fn initial() -> Self {
		Object { obj: GCMethods::initial() }
	}

	unsafe fn post_barrier(v: *mut Self, prev: Self, next: Self) {
		GCMethods::post_barrier(&mut (*v).obj, prev.obj, next.obj)
	}
}

impl Deref for Object {
	type Target = *mut JSObject;

	fn deref(&self) -> &Self::Target {
		&self.obj
	}
}

impl DerefMut for Object {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.obj
	}
}

impl FromValue for Object {
	fn from_value<'c, 's: 'c>(cx: &Context<'c>, value: Local<'s, Value>) -> Result<Local<'c, Self>, ()> {
		if !value.is_object() {
			unsafe { throw_type_error(cx.cx(), "Value is not an object") };
			return Err(());
		}

		let object = value.to_object();
		unsafe { AssertSameCompartment(cx.cx(), object) };
		Ok(Object::from_raw(cx, object))
	}
}

impl ToValue for Object {
	fn to_value<'c, 's: 'c>(this: Local<'s, Self>, cx: &Context<'c>) -> Local<'c, Value> {
		this.to_value(cx)
	}
}
