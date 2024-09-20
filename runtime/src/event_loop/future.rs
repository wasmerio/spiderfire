/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::task;
use std::task::Poll;

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use mozjs::jsapi::JSObject;
use tokio::task::JoinHandle;

use ion::{Context, Error, ErrorKind, ErrorReport, Promise, ThrowException, Value, TracedHeap};
use ion::conversions::BoxedIntoValue;

use super::{EventLoop, EventLoopPollResult};

type FutureOutput = (Result<BoxedIntoValue, BoxedIntoValue>, TracedHeap<*mut JSObject>);

pub struct FutureQueue {
	queue: Option<FuturesUnordered<JoinHandle<FutureOutput>>>,
}

impl Default for FutureQueue {
	fn default() -> Self {
		Self { queue: Some(Default::default()) }
	}
}

impl FutureQueue {
	pub fn poll_futures(
		&mut self, cx: &Context, wcx: &mut task::Context,
	) -> Result<EventLoopPollResult, Option<ErrorReport>> {
		let mut results = Vec::new();

		let queue = self.get_queue_mut();
		while let Poll::Ready(Some(item)) = queue.poll_next_unpin(wcx) {
			match item {
				Ok(item) => results.push(item),
				Err(error) => {
					Error::new(error.to_string(), ErrorKind::Normal).throw(cx);
					return Err(None);
				}
			}
		}

		let result = EventLoopPollResult::from_bool(!results.is_empty());

		for (result, promise) in results {
			let mut value = Value::undefined(cx);
			let promise = Promise::from(promise.root(cx)).unwrap();

			let result = match result {
				Ok(o) => {
					o.into_value(cx, &mut value);
					promise.resolve(cx, &value)
				}
				Err(e) => {
					e.into_value(cx, &mut value);
					promise.reject(cx, &value)
				}
			};

			if !result {
				return Err(ErrorReport::new_with_exception_stack(cx).unwrap());
			}
		}

		Ok(result)
	}

	pub fn enqueue(&self, cx: &Context, handle: JoinHandle<FutureOutput>) {
		self.get_queue().push(handle);
		EventLoop::from_context(cx).wake();
	}

	pub fn is_empty(&self) -> bool {
		self.get_queue().is_empty()
	}

	fn get_queue(&self) -> &FuturesUnordered<JoinHandle<FutureOutput>> {
		self.queue.as_ref().expect("Future queue was dropped but not recreated")
	}

	fn get_queue_mut(&mut self) -> &mut FuturesUnordered<JoinHandle<FutureOutput>> {
		self.queue.as_mut().expect("Future queue was dropped but not recreated")
	}

	pub fn recreate_queue(&mut self) {
		if self.queue.is_none() {
			self.queue = Some(Default::default());
		}
	}

	pub fn drop_queue(&mut self) {
		if self.queue.is_some() {
			assert!(self.is_empty());
			self.queue = None;
		}
	}
}
