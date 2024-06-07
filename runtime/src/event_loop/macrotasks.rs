/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::collections::HashMap;
use std::pin::Pin;
use std::{fmt, task};
use std::fmt::{Debug, Formatter};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Duration, Utc};
use futures::Future;
use mozjs::jsapi::JSFunction;
use mozjs::jsval::JSVal;

use ion::{Context, ErrorReport, Function, Object, Value, TracedHeap};

use super::{EventLoop, EventLoopPollResult};

#[allow(clippy::type_complexity)]
pub struct SignalMacrotask {
	callback: Option<Box<dyn FnOnce(&Context)>>,
	terminate: Arc<AtomicBool>,
	scheduled: DateTime<Utc>,
}

impl SignalMacrotask {
	pub fn new(callback: Box<dyn FnOnce(&Context)>, terminate: Arc<AtomicBool>, duration: Duration) -> SignalMacrotask {
		SignalMacrotask {
			callback: Some(callback),
			terminate,
			scheduled: Utc::now() + duration,
		}
	}
}

impl Debug for SignalMacrotask {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.debug_struct("SignalMacrotask")
			.field("terminate", &self.terminate.as_ref())
			.field("scheduled", &self.scheduled)
			.finish()
	}
}

#[derive(Debug)]
pub struct TimerMacrotask {
	callback: TracedHeap<*mut JSFunction>,
	arguments: Vec<TracedHeap<JSVal>>,
	repeat: bool,
	scheduled: DateTime<Utc>,
	duration: Duration,
	nesting: u8,
}

impl TimerMacrotask {
	pub fn new(callback: Function, arguments: &[JSVal], repeat: bool, duration: Duration) -> TimerMacrotask {
		TimerMacrotask {
			callback: TracedHeap::new(callback.get()),
			arguments: arguments.iter().map(|a| TracedHeap::new(*a)).collect(),
			repeat,
			duration,
			scheduled: Utc::now(),
			nesting: 0,
		}
	}

	pub fn reset(&mut self) -> bool {
		if self.repeat {
			self.scheduled = Utc::now();
		}
		self.repeat
	}
}

#[derive(Debug)]
pub struct UserMacrotask {
	callback: TracedHeap<*mut JSFunction>,
	scheduled: DateTime<Utc>,
}

impl UserMacrotask {
	pub fn new(callback: Function) -> UserMacrotask {
		UserMacrotask {
			callback: TracedHeap::new(callback.get()),
			scheduled: Utc::now(),
		}
	}
}

#[derive(Debug)]
pub enum Macrotask {
	Signal(SignalMacrotask),
	Timer(TimerMacrotask),
	User(UserMacrotask),
}

#[derive(Debug, Default)]
pub struct MacrotaskQueue {
	pub(crate) map: HashMap<u32, Macrotask>,
	pub(crate) nesting: u8,
	latest: Option<u32>,
	timer: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl Macrotask {
	pub fn run(&mut self, cx: &Context, nesting: &mut u8) -> Result<(), Option<ErrorReport>> {
		if let Macrotask::Signal(signal) = self {
			if let Some(callback) = signal.callback.take() {
				callback(cx);
			}
			return Ok(());
		}
		let (callback, args, my_nesting) = match &self {
			Macrotask::Timer(timer) => (&timer.callback, timer.arguments.clone(), timer.nesting),
			Macrotask::User(user) => (&user.callback, Vec::new(), 0),
			_ => unreachable!(),
		};

		let prev_nesting = *nesting;
		*nesting = my_nesting;

		let callback = Function::from(callback.root(cx));
		let args: Vec<_> = args.into_iter().map(|value| Value::from(value.root(cx))).collect();

		let res = callback.call(cx, &Object::global(cx), args.as_slice());

		*nesting = prev_nesting;

		res?;
		Ok(())
	}

	pub fn remove(&mut self) -> bool {
		match self {
			Macrotask::Timer(timer) => !timer.reset(),
			_ => true,
		}
	}

	fn terminate(&self) -> bool {
		match self {
			Macrotask::Signal(signal) => signal.terminate.load(Ordering::SeqCst),
			_ => false,
		}
	}

	fn remaining(&self, now: &DateTime<Utc>) -> Duration {
		match self {
			Macrotask::Signal(signal) => signal.scheduled - now,
			Macrotask::Timer(timer) => timer.scheduled + timer.duration - now,
			Macrotask::User(user) => user.scheduled - now,
		}
	}
}

impl MacrotaskQueue {
	pub fn poll_jobs(
		&mut self, cx: &Context, wcx: &mut task::Context,
	) -> Result<EventLoopPollResult, Option<ErrorReport>> {
		let mut result = EventLoopPollResult::NothingToDo;

		while let Some((next, remaining)) = self.find_earliest(&Utc::now()) {
			if remaining <= Duration::zero() {
				result = EventLoopPollResult::DidWork;

				{
					let macrotask = self.map.get_mut(&next);
					if let Some(macrotask) = macrotask {
						macrotask.run(cx, &mut self.nesting)?;
					}
				}

				// The previous reference may be invalidated by running the macrotask.
				let macrotask = self.map.get_mut(&next);
				if let Some(macrotask) = macrotask {
					if macrotask.remove() {
						self.map.remove(&next);
					}
				}
			} else {
				let mut timer = Box::pin(tokio::time::sleep(
					remaining.to_std().expect("Duration should have been greater than zero"),
				));

				// The assumption is that the event loop will be polled until it is empty
				// and it is clearly not empty at this point, so returning a Poll::Pending
				// doesn't really accomplish anything.
				_ = timer.as_mut().poll(wcx);

				self.timer = Some(timer);

				break;
			}
		}

		Ok(result)
	}

	pub fn enqueue(&mut self, cx: &Context, mut macrotask: Macrotask, id: Option<u32>) -> u32 {
		let index = id.unwrap_or_else(|| self.latest.map(|l| l + 1).unwrap_or(0));

		if let Macrotask::Timer(timer) = &mut macrotask {
			timer.nesting = self.nesting.saturating_add(1);
		}

		self.latest = Some(index);
		self.map.insert(index, macrotask);

		// We must wake the task up, if only to register a new timer.
		EventLoop::from_context(cx).wake();

		index
	}

	pub fn remove(&mut self, id: u32) {
		self.map.remove(&id);
	}

	fn find_earliest(&mut self, now: &DateTime<Utc>) -> Option<(u32, Duration)> {
		let mut next: Option<(u32, Duration)> = None;
		let mut to_remove = Vec::new();
		for (id, macrotask) in &self.map {
			if macrotask.terminate() {
				to_remove.push(*id);
				continue;
			}

			let remaining = macrotask.remaining(now);

			match next {
				Some((_, rem)) if rem < remaining => (),
				_ => next = Some((*id, remaining)),
			}
		}

		for id in to_remove.iter_mut() {
			self.map.remove(id);
		}

		next
	}

	pub fn is_empty(&self) -> bool {
		self.map.is_empty()
	}
}
