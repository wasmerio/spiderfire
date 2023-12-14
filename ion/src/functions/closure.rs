/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use mozjs::glue::JS_GetReservedSlot;
use mozjs::jsapi::{
	GCContext, GetFunctionNativeReserved, JS_NewObject, JS_SetReservedSlot, JSClass, JSCLASS_BACKGROUND_FINALIZE,
	JSClassOps, JSContext, JSObject,
};
use mozjs::jsval::{JSVal, PrivateValue, UndefinedValue};

use crate::{Arguments, Context, Object, ResultExc, Value, Exception, Error, ErrorKind};
use crate::conversions::IntoValue;
use crate::functions::__handle_native_function_result;
use crate::objects::class_reserved_slots;

const CLOSURE_SLOT: u32 = 0;
const ONCE_STATUS_SLOT: u32 = 1;

const ONCE_STATUS_NOT_ONCE: u32 = 0;
const ONCE_STATUS_ONCE: u32 = 1;

pub type Closure = dyn for<'cx> FnMut(&mut Arguments<'cx>) -> ResultExc<Value<'cx>> + 'static;
pub type ClosureOnce = dyn for<'cx> FnOnce(&mut Arguments<'cx>) -> ResultExc<Value<'cx>> + 'static;

pub(crate) fn create_closure_object(cx: &Context, closure: Box<Closure>) -> Object {
	unsafe {
		let object = Object::from(cx.root_object(JS_NewObject(cx.as_ptr(), &CLOSURE_CLASS)));
		JS_SetReservedSlot(
			object.handle().get(),
			CLOSURE_SLOT,
			&PrivateValue(Box::into_raw(Box::new(closure)).cast_const().cast()),
		);
		JS_SetReservedSlot(
			object.handle().get(),
			ONCE_STATUS_SLOT,
			&PrivateValue((&ONCE_STATUS_NOT_ONCE) as *const _ as *const c_void),
		);
		object
	}
}

pub(crate) fn create_closure_object_once(cx: &Context, closure: Box<ClosureOnce>) -> Object {
	unsafe {
		let object = Object::from(cx.root_object(JS_NewObject(cx.as_ptr(), &CLOSURE_CLASS)));
		JS_SetReservedSlot(
			object.handle().get(),
			CLOSURE_SLOT,
			&PrivateValue(Box::into_raw(Box::new(Some(closure))).cast_const().cast()),
		);
		JS_SetReservedSlot(
			object.handle().get(),
			ONCE_STATUS_SLOT,
			&PrivateValue((&ONCE_STATUS_ONCE) as *const _ as *const c_void),
		);
		object
	}
}

pub(crate) unsafe extern "C" fn call_closure(cx: *mut JSContext, argc: u32, vp: *mut JSVal) -> bool {
	let cx = &unsafe { Context::new_unchecked(cx) };
	let args = &mut unsafe { Arguments::new(cx, argc, vp) };

	let callee = cx.root_object(args.call_args().callee());
	let reserved = cx.root_value(unsafe { *GetFunctionNativeReserved(callee.get(), 0) });

	let mut once_status_value = UndefinedValue();
	unsafe { JS_GetReservedSlot(reserved.handle().to_object(), ONCE_STATUS_SLOT, &mut once_status_value) };
	let once_status = unsafe { *(once_status_value.to_private() as *mut u32) };

	let mut closure_value = UndefinedValue();
	unsafe { JS_GetReservedSlot(reserved.handle().to_object(), CLOSURE_SLOT, &mut closure_value) };

	let result = match once_status {
		ONCE_STATUS_NOT_ONCE => {
			let closure = unsafe { &mut *(closure_value.to_private() as *mut Box<Closure>) };

			catch_unwind(AssertUnwindSafe(|| {
				closure(args).map(|result| Box::new(result).into_value(cx, args.rval()))
			}))
		}

		ONCE_STATUS_ONCE => {
			let closure = unsafe { &mut *(closure_value.to_private() as *mut Option<Box<ClosureOnce>>) };

			match closure.take() {
				Some(closure) => catch_unwind(AssertUnwindSafe(|| {
					closure(args).map(|result| Box::new(result).into_value(cx, args.rval()))
				})),

				None => Ok(Err(Exception::Error(Error::new(
					"Once closure cannot be called multiple times",
					ErrorKind::Type,
				)))),
			}
		}

		_ => Ok(Err(Exception::Error(Error::new(
			"Internal error: invalid once status on closure",
			ErrorKind::Type,
		)))),
	};

	__handle_native_function_result(cx, result)
}

unsafe extern "C" fn finalise_closure(_: *mut GCContext, object: *mut JSObject) {
	let mut once_status_value = UndefinedValue();
	unsafe { JS_GetReservedSlot(object, ONCE_STATUS_SLOT, &mut once_status_value) };
	let once_status = unsafe { *(once_status_value.to_private() as *mut u32) };

	let mut closure_value = UndefinedValue();
	unsafe {
		JS_GetReservedSlot(object, CLOSURE_SLOT, &mut closure_value);

		match once_status {
			ONCE_STATUS_NOT_ONCE => drop(Box::from_raw(closure_value.to_private() as *mut Box<Closure>)),
			ONCE_STATUS_ONCE => drop(Box::from_raw(
				closure_value.to_private() as *mut Option<Box<ClosureOnce>>
			)),
			_ => (),
		}
	}
}

static CLOSURE_OPS: JSClassOps = JSClassOps {
	addProperty: None,
	delProperty: None,
	enumerate: None,
	newEnumerate: None,
	resolve: None,
	mayResolve: None,
	finalize: Some(finalise_closure),
	call: None,
	construct: None,
	trace: None,
};

static CLOSURE_CLASS: JSClass = JSClass {
	name: "Closure\0".as_ptr().cast(),
	flags: JSCLASS_BACKGROUND_FINALIZE | class_reserved_slots(2),
	cOps: &CLOSURE_OPS,
	spec: ptr::null_mut(),
	ext: ptr::null_mut(),
	oOps: ptr::null_mut(),
};
