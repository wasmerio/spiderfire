/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::{ptr, task};
use std::future::Future;
use std::pin::{Pin, pin};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::Poll;

use chrono::Duration;
use mozjs::jsapi::JSObject;
use mozjs::jsval::{JSVal, UndefinedValue};
use tokio::sync::watch::{channel, Receiver, Sender};

use ion::{ClassDefinition, Context, Error, ErrorKind, Exception, Object, Result, ResultExc, TracedHeap, Value};
use ion::class::Reflector;
use ion::conversions::{FromValue, ToValue};
use ion::function::{Enforce, Opt};

use crate::ContextExt;
use crate::event_loop::macrotasks::{Macrotask, SignalMacrotask};

#[derive(Clone, Debug, Default)]
pub enum Signal {
	#[default]
	None,
	Abort(TracedHeap<JSVal>),
	Receiver(Receiver<Option<TracedHeap<JSVal>>>),
	Timeout(Receiver<Option<TracedHeap<JSVal>>>, Arc<AtomicBool>),
}

impl Signal {
	pub fn poll(&self) -> SignalFuture {
		SignalFuture { inner: self.clone() }
	}
}

pub struct SignalFuture {
	inner: Signal,
}

impl Future for SignalFuture {
	type Output = JSVal;

	fn poll(mut self: Pin<&mut SignalFuture>, cx: &mut task::Context) -> Poll<JSVal> {
		match &mut self.inner {
			Signal::None => Poll::Pending,
			Signal::Abort(abort) => Poll::Ready(abort.get()),
			Signal::Receiver(receiver) | Signal::Timeout(receiver, _) => {
				if let Some(ref abort) = *receiver.borrow() {
					return Poll::Ready(abort.get());
				}
				let changed = { pin!(receiver.changed()).poll(cx) };
				match changed {
					Poll::Ready(_) => match *receiver.borrow() {
						Some(ref abort) => Poll::Ready(abort.get()),
						None => Poll::Pending,
					},
					Poll::Pending => Poll::Pending,
				}
			}
		}
	}
}

impl Drop for SignalFuture {
	fn drop(&mut self) {
		if let Signal::Timeout(receiver, terminate) = &self.inner {
			if receiver.borrow().is_none() {
				terminate.store(true, Ordering::SeqCst);
			}
		}
	}
}

#[js_class]
pub struct AbortController {
	reflector: Reflector,
	#[trace(no_trace)]
	sender: Sender<Option<TracedHeap<JSVal>>>,
}

#[js_class]
impl AbortController {
	#[ion(constructor)]
	pub fn constructor() -> AbortController {
		let (sender, _) = channel(None);
		AbortController { reflector: Reflector::default(), sender }
	}

	// TODO: Return the same signal object
	#[ion(get)]
	pub fn get_signal(&self, cx: &Context) -> *mut JSObject {
		AbortSignal::new_object(
			cx,
			Box::new(AbortSignal {
				reflector: Reflector::default(),
				signal: Signal::Receiver(self.sender.subscribe()),
			}),
		)
	}

	pub fn abort<'cx>(&self, cx: &'cx Context, Opt(reason): Opt<Value<'cx>>) {
		let reason = reason.unwrap_or_else(|| Error::new("AbortError", None).as_value(cx));
		self.sender.send_replace(Some(TracedHeap::from_local(&reason)));
	}
}

#[js_class]
#[derive(Default)]
pub struct AbortSignal {
	reflector: Reflector,
	#[trace(no_trace)]
	pub(crate) signal: Signal,
}

#[js_class]
impl AbortSignal {
	#[ion(constructor)]
	pub fn constructor() -> Result<AbortSignal> {
		Err(Error::new("AbortSignal has no constructor.", ErrorKind::Type))
	}

	#[ion(get)]
	pub fn get_aborted(&self) -> bool {
		!self.get_reason().is_undefined()
	}

	#[ion(get)]
	pub fn get_reason(&self) -> JSVal {
		match &self.signal {
			Signal::None => UndefinedValue(),
			Signal::Abort(abort) => abort.get(),
			Signal::Receiver(receiver) | Signal::Timeout(receiver, _) => {
				receiver.borrow().as_ref().map(|x| x.get()).unwrap_or_else(UndefinedValue)
			}
		}
	}

	#[ion(name = "throwIfAborted")]
	pub fn throw_if_aborted(&self) -> ResultExc<()> {
		let reason = self.get_reason();
		if reason.is_undefined() {
			Ok(())
		} else {
			Err(Exception::Other(reason))
		}
	}

	pub fn abort<'cx>(cx: &'cx Context, Opt(reason): Opt<Value<'cx>>) -> *mut JSObject {
		let reason = reason.unwrap_or_else(|| Error::new("AbortError", None).as_value(cx));
		AbortSignal::new_object(
			cx,
			Box::new(AbortSignal {
				reflector: Reflector::default(),
				signal: Signal::Abort(TracedHeap::from_local(&reason)),
			}),
		)
	}

	pub fn timeout(cx: &Context, Enforce(time): Enforce<u64>) -> *mut JSObject {
		let (sender, receiver) = channel(None);
		let terminate = Arc::new(AtomicBool::new(false));
		let terminate2 = Arc::clone(&terminate);

		let callback = Box::new(move |cx: &_| {
			let error = Error::new(format!("Timeout Error: {}ms", time), None).as_value(cx).get();
			sender.send_replace(Some(TracedHeap::new(error)));
		});

		let duration = Duration::milliseconds(time as i64);
		let event_loop = unsafe { &mut cx.get_private().event_loop };
		if let Some(queue) = &mut event_loop.macrotasks {
			queue.enqueue(
				cx,
				Macrotask::Signal(SignalMacrotask::new(callback, terminate, duration)),
				None,
			);
			AbortSignal::new_object(
				cx,
				Box::new(AbortSignal {
					reflector: Reflector::default(),
					signal: Signal::Timeout(receiver, terminate2),
				}),
			)
		} else {
			ptr::null_mut()
		}
	}
}

impl<'cx> FromValue<'cx> for AbortSignal {
	type Config = ();
	fn from_value(cx: &'cx Context, value: &Value, strict: bool, _: ()) -> Result<AbortSignal> {
		let object = Object::from_value(cx, value, strict, ())?;
		if AbortSignal::instance_of(cx, &object) {
			Ok(AbortSignal {
				reflector: Reflector::default(),
				signal: AbortSignal::get_private(cx, &object)?.signal.clone(),
			})
		} else {
			Err(Error::new("Expected AbortSignal", ErrorKind::Type))
		}
	}
}

pub fn define(cx: &Context, global: &Object) -> bool {
	AbortController::init_class(cx, global).0 && AbortSignal::init_class(cx, global).0
}
