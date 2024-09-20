/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::collections::VecDeque;
use std::ffi::c_void;
use std::task::{self, Waker};
use std::task::Poll;

use mozjs::jsapi::{Handle, JSContext, JSObject, PromiseRejectionHandlingState};

use ion::{Context, ErrorReport, Local, Promise, TracedHeap};
use ion::format::{Config, format_value};

use crate::ContextExt;
use crate::event_loop::future::FutureQueue;
use crate::event_loop::macrotasks::MacrotaskQueue;
use crate::event_loop::microtasks::MicrotaskQueue;

pub(crate) mod future;
pub(crate) mod macrotasks;
pub(crate) mod microtasks;

pub enum EventLoopPollResult {
	NothingToDo,
	DidWork,
}

impl EventLoopPollResult {
	fn from_bool(did_work: bool) -> Self {
		if did_work {
			Self::DidWork
		} else {
			Self::NothingToDo
		}
	}

	fn did_work(&self) -> bool {
		matches!(self, Self::DidWork)
	}

	fn compound_with(&mut self, other: &Self) {
		*self = if self.did_work() || other.did_work() {
			Self::DidWork
		} else {
			Self::NothingToDo
		};
	}
}

#[derive(Default)]
pub struct EventLoop {
	pub(crate) futures: Option<FutureQueue>,
	pub(crate) microtasks: Option<MicrotaskQueue>,
	pub(crate) macrotasks: Option<MacrotaskQueue>,
	pub(crate) unhandled_rejections: VecDeque<TracedHeap<*mut JSObject>>,
	pub(crate) waker: Option<Waker>,
}

impl EventLoop {
	#[allow(clippy::mut_from_ref)]
	pub(crate) fn from_context(cx: &Context) -> &mut Self {
		cx.get_event_loop()
	}

	pub(crate) fn wake(&mut self) {
		if let Some(waker) = self.waker.take() {
			waker.wake();
		}
	}

	pub(crate) fn run_to_end(&mut self, cx: &Context) -> RunToEnd {
		RunToEnd { event_loop: self, cx: cx.as_ptr() }
	}

	pub(crate) fn step(&mut self, cx: &Context, wcx: &mut task::Context) -> Result<(), Option<ErrorReport>> {
		let res = self.step_inner(cx, wcx);

		match self.waker {
			Some(ref w) if w.will_wake(wcx.waker()) => (),
			_ => self.waker = Some(wcx.waker().clone()),
		}

		// If we were interrupted by an error, there may still be more to do
		if res.is_err() {
			self.wake();
		}

		res
	}

	fn step_inner(&mut self, cx: &Context, wcx: &mut task::Context) -> Result<(), Option<ErrorReport>> {
		let mut poll_result = EventLoopPollResult::NothingToDo;

		if let Some(futures) = &mut self.futures {
			if !futures.is_empty() {
				poll_result.compound_with(&futures.poll_futures(cx, wcx)?);
			}
		}

		if let Some(microtasks) = &mut self.microtasks {
			if !microtasks.is_empty() {
				poll_result.compound_with(&microtasks.run_jobs(cx)?);
			}
		}

		if let Some(macrotasks) = &mut self.macrotasks {
			if !macrotasks.is_empty() {
				poll_result.compound_with(&macrotasks.poll_jobs(cx, wcx)?);
			}
		}

		while let Some(promise) = self.unhandled_rejections.pop_front() {
			let promise = Promise::from(promise.to_local()).unwrap();
			let result = promise.result(cx);
			eprintln!(
				"Unhandled Promise Rejection: {}",
				format_value(cx, Config::default(), &result)
			);
		}

		// TODO: Is it necessary to run the entire event loop again? Just running new
		// microtasks may be enough here.
		if poll_result.did_work() {
			// Make another pass on the event loop, since doing work may lead to new work
			// being enqueued in the event loop
			self.step_inner(cx, wcx)
		} else {
			Ok(())
		}
	}

	pub fn is_empty(&self) -> bool {
		self.microtasks.as_ref().map(|m| m.is_empty()).unwrap_or(true)
			&& self.futures.as_ref().map(|f| f.is_empty()).unwrap_or(true)
			&& self.macrotasks.as_ref().map(|m| m.is_empty()).unwrap_or(true)
	}
}

pub struct RunToEnd<'e> {
	event_loop: &'e mut EventLoop,
	cx: *mut JSContext,
}

impl<'e> futures::Future for RunToEnd<'e> {
	type Output = Result<(), Option<ErrorReport>>;

	fn poll(mut self: std::pin::Pin<&mut Self>, wcx: &mut task::Context<'_>) -> Poll<Self::Output> {
		let cx = unsafe { Context::new_unchecked(self.cx) };
		match self.event_loop.step(&cx, wcx) {
			Err(e) => Poll::Ready(Err(e)),
			Ok(()) if self.event_loop.is_empty() => Poll::Ready(Ok(())),
			Ok(()) => Poll::Pending,
		}
	}
}

pub(crate) unsafe extern "C" fn promise_rejection_tracker_callback(
	cx: *mut JSContext, _: bool, promise: Handle<*mut JSObject>, state: PromiseRejectionHandlingState, _: *mut c_void,
) {
	let cx = unsafe { &Context::new_unchecked(cx) };
	let unhandled = &mut cx.get_event_loop().unhandled_rejections;
	let promise = unsafe { Local::from_raw_handle(promise) };
	match state {
		PromiseRejectionHandlingState::Unhandled => unhandled.push_back(TracedHeap::from_local(&promise)),
		PromiseRejectionHandlingState::Handled => {
			let idx = unhandled.iter().position(|unhandled| unhandled.get() == promise.get());
			if let Some(idx) = idx {
				unhandled.swap_remove_back(idx);
			}
		}
	}
}
