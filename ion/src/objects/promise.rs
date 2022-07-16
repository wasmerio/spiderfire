/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::future::Future;
use std::mem::transmute;
use std::ops::{Deref, DerefMut};

use futures::executor::block_on;
use libffi::high::ClosureMut3;
use mozjs::jsapi::{JS, JSContext, JSObject};
use mozjs::jsval::JSVal;
use mozjs::rust::{Handle, HandleObject};
use mozjs::rust::jsapi_wrapped::{IsPromiseObject, NewPromiseObject};
use mozjs_sys::jsgc::{GCMethods, RootKind};

use crate::{Context, Local, Object, Value};
use crate::functions::{Arguments, Function};
use crate::value::ToValueNative;

#[derive(Clone, Debug)]
pub struct Promise {
	pub(crate) promise: *mut JSObject,
}

impl Promise {
	pub fn new<'c>(cx: &Context<'c>) -> Local<'c, Promise> {
		Promise::from_raw(cx, unsafe { NewPromiseObject(cx.cx(), HandleObject::null()) }).unwrap()
	}

	pub(crate) fn new_with_executor<'c, 's, F>(cx: &Context<'c>, mut executor: F) -> Option<Local<'c, Promise>>
	where
		F: FnMut(&Context<'c>, &Local<'c, Function>, &Local<'c, Function>) -> crate::Result<()>,
	{
		unsafe {
			let mut native = |_: *mut JSContext, argc: u32, vp: *mut JSVal| {
				// TODO: Fix Lifetimes
				// let cx = Context::new(&mut cx);
				let args = Arguments::new(cx, argc, vp);
				let undefined = Value::undefined(cx);

				let resolve = Function::from_object_raw(cx, (**args.get(0).unwrap_or_else(|| &undefined)).to_object());
				let reject = Function::from_object_raw(cx, (**args.get(1).unwrap_or_else(|| &undefined)).to_object());

				match (resolve, reject) {
					(Some(resolve), Some(reject)) => match executor(cx, &resolve, &reject) {
						Ok(()) => true as u8,
						Err(error) => {
							error.throw(cx);
							false as u8
						}
					},
					_ => false as u8,
				}
			};
			let closure = ClosureMut3::new(&mut native);
			let fn_ptr = transmute::<_, &unsafe extern "C" fn(*mut JSContext, u32, *mut JSVal) -> bool>(closure.code_ptr());
			let function = Function::new(cx, "executor", Some(*fn_ptr), 2, 0);

			let executor = function.to_object(cx);
			let handle = Handle::from_marked_location(&**executor);
			let promise = NewPromiseObject(cx.cx(), handle);
			if !promise.is_null() {
				Some(Promise::from_raw(cx, promise).unwrap())
			} else {
				None
			}
		}
	}

	pub fn new_with_future<'c, F, Output, Error>(cx: &Context<'c>, future: F) -> Option<Local<'c, Promise>>
	where
		F: Future<Output = Result<Output, Error>>,
		Output: ToValueNative,
		Error: ToValueNative,
	{
		let mut future = Some(future);
		let null = Object::null(cx);
		Promise::new_with_executor(cx, |cx, resolve, reject| {
			block_on(async {
				let future = future.take().unwrap();
				match future.await {
					Ok(v) => {
						let value = v.to_value_native(cx);
						if let Err(Some(error)) = resolve.call(cx, &null, vec![&value]) {
							error.print();
						}
					}
					Err(v) => {
						let value = v.to_value_native(cx);
						if let Err(Some(error)) = reject.call(cx, &null, vec![&value]) {
							error.print();
						}
					}
				}
			});
			Ok(())
		})
	}

	pub(crate) fn from_raw<'c>(cx: &Context<'c>, promise: *mut JSObject) -> Option<Local<'c, Promise>> {
		if Promise::is_promise_raw(promise) {
			Some(Local::new(cx, Promise { promise }))
		} else {
			None
		}
	}

	pub(crate) fn is_promise_raw(obj: *mut JSObject) -> bool {
		let handle = unsafe { Handle::from_marked_location(&obj) };
		unsafe { IsPromiseObject(handle) }
	}
}

impl RootKind for Promise {
	#[allow(non_snake_case)]
	fn rootKind() -> JS::RootKind {
		JS::RootKind::Object
	}
}

impl GCMethods for Promise {
	unsafe fn initial() -> Self {
		Promise { promise: GCMethods::initial() }
	}

	unsafe fn post_barrier(v: *mut Self, prev: Self, next: Self) {
		GCMethods::post_barrier(&mut (*v).promise, prev.promise, next.promise)
	}
}

impl Deref for Promise {
	type Target = *mut JSObject;

	fn deref(&self) -> &Self::Target {
		&self.promise
	}
}

impl DerefMut for Promise {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.promise
	}
}
