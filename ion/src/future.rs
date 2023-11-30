/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::future::Future;
use std::pin::Pin;
use std::task;
use std::task::Poll;

use futures::channel::mpsc;
use futures::channel::mpsc::Receiver;
use futures::Stream;
use mozjs::jsval::JSVal;
use mozjs_sys::jsapi::JSContext;

use crate::{Context, Function, Promise, Value};
use crate::flags::PropertyFlags;

pub struct PromiseFuture(*mut JSContext, Receiver<Result<JSVal, JSVal>>);

impl PromiseFuture {
	/// See documentation for [`runtime::promise::future_to_promise`].
	pub fn new(cx: Context, promise: &Promise) -> PromiseFuture {
		let (tx, rx) = mpsc::channel(1);

		let mut tx1 = tx;
		let mut tx2 = tx1.clone();

		promise.add_reactions(
			&cx,
			Some(Function::from_closure(
				&cx,
				"",
				Box::new(move |args| {
					let _ = tx1.try_send(Ok(args.value(0).unwrap().get()));
					Ok(Value::undefined(args.cx()))
				}),
				1,
				PropertyFlags::empty(),
			)),
			Some(Function::from_closure(
				&cx,
				"",
				Box::new(move |args| {
					let _ = tx2.try_send(Err(args.value(0).unwrap().get()));
					Ok(Value::undefined(args.cx()))
				}),
				1,
				PropertyFlags::empty(),
			)),
		);

		let cx_ptr = cx.as_ptr();
		drop(cx);

		PromiseFuture(cx_ptr, rx)
	}
}

impl Future for PromiseFuture {
	type Output = (Context, Result<JSVal, JSVal>);

	fn poll(mut self: Pin<&mut Self>, cx: &mut task::Context<'_>) -> Poll<Self::Output> {
		let result = Pin::new(&mut self.1);
		if let Poll::Ready(Some(val)) = result.poll_next(cx) {
			Poll::Ready((unsafe { Context::new_unchecked(self.0) }, val))
		} else {
			Poll::Pending
		}
	}
}
