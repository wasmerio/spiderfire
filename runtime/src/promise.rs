/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::future::Future;

use tokio::task::spawn_local;

use ion::{Context, Promise};
use ion::conversions::{BoxedIntoValue, IntoValue};

use crate::ContextExt;

/// Returns None if no future queue has been initialised.
///
/// This function creates a new [ion::Context] for use within the future.
///
/// # Safety
/// Rooted values must be dropped in LIFO order. The [mozjs::rust::RootedGuard] type
/// guarantees LIFO ordering by unrooting values as it is dropped. However, the [ion]
/// types do not replicate this behavior, instead relying on [ion::Context] to unroot
/// values in order once it's dropped.
///
/// While this works most of the time, in the presence of futures, it's easy to hold
/// a context across an await point. While the future is waiting, other code will run
/// and root new values, thus violating the LIFO ordering.
///
/// To ensure safe usage of JS values in futures, all contexts must be dropped before
/// each await point and a new context created. [ion::future::PromiseFuture] already
/// ensures this be receiving the context by value and returning a new one.
///
/// To make sure this requirement is not violated when awaiting native futures, it is
/// recommended to use the [ion::Context::await_native<Fut>()] method. It is
/// recommended to use this method in any future that has access to an [ion::Context].
///
/// To hold values across await points, use [ion::Heap] which keeps a pointer to the
/// values on the heap. You can root the heap value in the new context using the
/// [ion::Heap::root()] method.
pub unsafe fn future_to_promise<'cx, F, Fut, O, E>(cx: &'cx Context, callback: F) -> Option<Promise>
where
	F: (FnOnce(Context) -> Fut) + 'static,
	Fut: Future<Output = Result<O, E>> + 'static,
	O: for<'cx2> IntoValue<'cx2> + 'static,
	E: for<'cx2> IntoValue<'cx2> + 'static,
{
	let promise = Promise::new(cx);
	let object = promise.root(cx).handle().get();
	let cx2 = unsafe { Context::new_unchecked(cx.as_ptr()) };

	let handle = spawn_local(async move {
		let result: Result<BoxedIntoValue, BoxedIntoValue> = match callback(cx2).await {
			Ok(o) => Ok(Box::new(o)),
			Err(e) => Err(Box::new(e)),
		};
		(result, object)
	});

	let event_loop = unsafe { &(*cx.get_private().as_ptr()).event_loop };
	event_loop.futures.as_ref().map(|futures| {
		futures.enqueue(handle);
		promise
	})
}
