/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::fmt::Debug;
use std::future::Future;
use std::ops::{Deref, DerefMut};

use futures::executor::block_on;
use mozjs::glue::JS_GetPromiseResult;
use mozjs::jsapi::{
	AddPromiseReactions, GetPromiseID, GetPromiseState, IsPromiseObject, JSObject, NewPromiseObject, PromiseState,
	RejectPromise, ResolvePromise, AddPromiseReactionsIgnoringUnhandledRejection, CallOriginalPromiseResolve,
	CallOriginalPromiseReject,
};
use mozjs::rust::HandleObject;
use mozjs_sys::jsapi::JS_GetPendingException;

use crate::conversions::IntoValue;
use crate::{Context, Function, Local, Object, Value, TracedHeap};
use crate::{conversions::ToValue, flags::PropertyFlags};

/// Represents a [Promise] in the JavaScript Runtime.
/// Refer to [MDN](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/Promise) for more details.
pub struct Promise {
	promise: TracedHeap<*mut JSObject>,
}

impl Debug for Promise {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Promise").finish()
	}
}

impl Promise {
	/// Creates a new [Promise] which never resolves.
	pub fn new(cx: &Context) -> Promise {
		Promise {
			promise: TracedHeap::from_local(
				&cx.root_object(unsafe { NewPromiseObject(cx.as_ptr(), HandleObject::null().into()) }),
			),
		}
	}

	pub fn new_resolved<'cx>(cx: &'cx Context, value: impl IntoValue<'cx>) -> Promise {
		let mut val = Value::undefined(cx);
		Box::new(value).into_value(cx, &mut val);

		Promise {
			promise: TracedHeap::from_local(
				&cx.root_object(unsafe { CallOriginalPromiseResolve(cx.as_ptr(), val.handle().into()) }),
			),
		}
	}

	pub fn new_rejected<'cx>(cx: &'cx Context, value: impl IntoValue<'cx>) -> Promise {
		let mut val = Value::undefined(cx);
		Box::new(value).into_value(cx, &mut val);

		Promise {
			promise: TracedHeap::from_local(
				&cx.root_object(unsafe { CallOriginalPromiseReject(cx.as_ptr(), val.handle().into()) }),
			),
		}
	}

	pub fn new_rejected_with_pending_exception(cx: &Context) -> Promise {
		let mut val = Value::undefined(cx);
		unsafe { JS_GetPendingException(cx.as_ptr(), val.handle_mut().into()) };

		Self::new_rejected(cx, val)
	}

	pub fn new_from_result<'cx>(cx: &'cx Context, value: Result<impl IntoValue<'cx>, impl IntoValue<'cx>>) -> Promise {
		match value {
			Ok(o) => Self::new_resolved(cx, o),

			Err(e) => Self::new_rejected(cx, e),
		}
	}

	/// Creates a new [Promise] with an executor.
	/// The executor is a function that takes in two functions, `resolve` and `reject`.
	/// `resolve` and `reject` can be called with a [Value] to resolve or reject the promise with the given value.
	pub fn new_with_executor<F>(cx: &Context, executor: F) -> Option<Promise>
	where
		F: for<'cx> FnOnce(&'cx Context, Function<'cx>, Function<'cx>) -> crate::Result<()> + 'static,
	{
		use crate::Exception;

		unsafe {
			let function = Function::from_closure_once(
				cx,
				"executor",
				Box::new(move |args| {
					let cx = args.cx();

					let resolve_obj = args.value(0).unwrap().to_object(cx).into_local();
					let reject_obj = args.value(1).unwrap().to_object(cx).into_local();
					let resolve = Function::from_object(cx, &resolve_obj).unwrap();
					let reject = Function::from_object(cx, &reject_obj).unwrap();

					match executor(cx, resolve, reject) {
						Ok(()) => Ok(Value::undefined(args.cx())),
						Err(error) => Err(Exception::Error(error)),
					}
				}),
				2,
				PropertyFlags::empty(),
			);
			let executor = function.to_object(cx);
			let promise = NewPromiseObject(cx.as_ptr(), executor.handle().into());

			if !promise.is_null() {
				Some(Promise {
					promise: TracedHeap::from_local(&cx.root_object(promise)),
				})
			} else {
				None
			}
		}
	}

	/// Creates a new [Promise] with a [Future].
	/// The future is run to completion on the current thread and cannot interact with an asynchronous runtime.
	///
	/// The [Result] of the future determines if the promise is resolved or rejected.
	pub fn block_on_future<F, Output, Error>(cx: &Context, future: F) -> Option<Promise>
	where
		F: Future<Output = Result<Output, Error>> + 'static,
		Output: for<'cx> ToValue<'cx> + 'static,
		Error: for<'cx> ToValue<'cx> + 'static,
	{
		Promise::new_with_executor(cx, move |cx, resolve, reject| {
			let null = Object::null(cx);
			block_on(async move {
				match future.await {
					Ok(output) => {
						let value = output.as_value(cx);
						if let Err(Some(error)) = resolve.call(cx, &null, &[value]) {
							println!("{}", error.format(cx));
						}
					}
					Err(error) => {
						let value = error.as_value(cx);
						if let Err(Some(error)) = reject.call(cx, &null, &[value]) {
							println!("{}", error.format(cx));
						}
					}
				}
			});
			Ok(())
		})
	}

	/// Creates a [Promise] from an object.
	pub fn from(object: Local<'_, *mut JSObject>) -> Option<Promise> {
		if Promise::is_promise(&object) {
			Some(Promise { promise: TracedHeap::from_local(&object) })
		} else {
			None
		}
	}

	/// Creates a [Promise] from an object.
	pub fn from_raw(object: *mut JSObject, cx: &'_ Context) -> Option<Promise> {
		if Promise::is_promise_raw(cx, object) {
			Some(Promise { promise: TracedHeap::new(object) })
		} else {
			None
		}
	}

	/// Creates a [Promise] from aj object
	///
	/// ### Safety
	/// Object must be a Promise.
	pub unsafe fn from_unchecked(object: Local<*mut JSObject>) -> Promise {
		Promise { promise: TracedHeap::from_local(&object) }
	}

	/// Returns the ID of the [Promise].
	pub fn id(&self, cx: &Context) -> u64 {
		unsafe { GetPromiseID(self.root(cx).handle().into()) }
	}

	/// Returns the state of the [Promise].
	///
	/// The state can be `Pending`, `Fulfilled` and `Rejected`.
	pub fn state(&self, cx: &Context) -> PromiseState {
		unsafe { GetPromiseState(self.root(cx).handle().into()) }
	}

	/// Returns the result of the [Promise].
	///
	/// ### Note
	/// Currently leads to a sefault.
	pub fn result<'cx>(&self, cx: &'cx Context) -> Value<'cx> {
		let mut value = Value::undefined(cx);
		unsafe { JS_GetPromiseResult(self.root(cx).handle().into(), value.handle_mut().into()) }
		value
	}

	/// Adds Reactions to the [Promise]
	///
	/// `on_resolved` is similar to calling `.then()` on a promise.
	///
	/// `on_rejected` is similar to calling `.catch()` on a promise.
	pub fn add_reactions(
		&self, cx: &'_ Context, on_resolved: Option<Function<'_>>, on_rejected: Option<Function<'_>>,
	) -> bool {
		let mut resolved = Object::null(cx);
		let mut rejected = Object::null(cx);
		if let Some(on_resolved) = on_resolved {
			resolved.handle_mut().set(on_resolved.to_object(cx).handle().get());
		}
		if let Some(on_rejected) = on_rejected {
			rejected.handle_mut().set(on_rejected.to_object(cx).handle().get());
		}
		unsafe {
			AddPromiseReactions(
				cx.as_ptr(),
				self.root(cx).handle().into(),
				resolved.handle().into(),
				rejected.handle().into(),
			)
		}
	}

	/// Adds Reactions to the [Promise] while ignoring unhandled rejections
	///
	/// `on_resolved` is similar to calling `.then()` on a promise.
	///
	/// `on_rejected` is similar to calling `.catch()` on a promise.
	pub fn add_reactions_ignoring_unhandled_rejection(
		&self, cx: &'_ Context, on_resolved: Option<Function<'_>>, on_rejected: Option<Function<'_>>,
	) -> bool {
		let mut resolved = Object::null(cx);
		let mut rejected = Object::null(cx);
		if let Some(on_resolved) = on_resolved {
			resolved.handle_mut().set(on_resolved.to_object(cx).handle().get());
		}
		if let Some(on_rejected) = on_rejected {
			rejected.handle_mut().set(on_rejected.to_object(cx).handle().get());
		}
		unsafe {
			AddPromiseReactionsIgnoringUnhandledRejection(
				cx.as_ptr(),
				self.root(cx).handle().into(),
				resolved.handle().into(),
				rejected.handle().into(),
			)
		}
	}

	/// Resolves the [Promise] with the given [Value].
	pub fn resolve(&self, cx: &Context, value: &Value) -> bool {
		unsafe { ResolvePromise(cx.as_ptr(), self.root(cx).handle().into(), value.handle().into()) }
	}

	/// Rejects the [Promise] with the given [Value].
	pub fn reject(&self, cx: &Context, value: &Value) -> bool {
		unsafe { RejectPromise(cx.as_ptr(), self.root(cx).handle().into(), value.handle().into()) }
	}

	/// Checks if a [*mut] [JSObject] is a promise.
	pub fn is_promise_raw(cx: &Context, object: *mut JSObject) -> bool {
		rooted!(in(cx.as_ptr()) let object = object);
		unsafe { IsPromiseObject(object.handle().into()) }
	}

	/// Checks if an object is a promise.
	pub fn is_promise(object: &Local<*mut JSObject>) -> bool {
		unsafe { IsPromiseObject(object.handle().into()) }
	}
}

impl Deref for Promise {
	type Target = TracedHeap<*mut JSObject>;

	fn deref(&self) -> &Self::Target {
		&self.promise
	}
}

impl DerefMut for Promise {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.promise
	}
}
