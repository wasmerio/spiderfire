/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use mozjs::gc::Traceable;
use mozjs::jsapi::{Heap, Handle, JSContext, IsDetachedArrayBufferObject, JS_NewUint8ArrayWithBuffer, JSObject, JSTracer, NewArrayBufferWithContents, StealArrayBufferContents};
use mozjs::jsval::ObjectValue;
pub use mozjs::rust::IntoHandle;
use mozjs::typedarray::{ArrayBuffer, CreateWith};
pub use byte_stream::ByteStreamController;
pub use default::DefaultController;
use ion::{Context, Exception, Function, Local, Object, Promise, Value, Result, ClassInitialiser, Error, ErrorKind, ResultExc};
use ion::conversions::{FromValue, ToValue};
use crate::globals::streams::readable::reader::{Reader, ReadResult, ReaderKind, ByobReader};
use crate::globals::streams::readable::State;
use crate::globals::streams::readable::stream::ReadableStream;

pub(crate) struct PullIntoDescriptor {
	pub(crate) buffer: Heap<*mut JSObject>,
	pub(crate) offset: usize,
	pub(crate) length: usize,
	pub(crate) filled: usize,
	pub(crate) element: usize,
	pub(crate) constructor: unsafe extern "C" fn (*mut JSContext, Handle<*mut JSObject>, usize, i64) -> *mut JSObject,
	pub(crate) kind: ReaderKind,
}

impl PullIntoDescriptor {
	pub(crate) fn construct<'cx>(&self, cx: &'cx Context) -> ResultExc<Local<'cx, *mut JSObject>> {
		unsafe {
			let constructor = self.constructor;

			let array = constructor(**cx, self.buffer.handle(), self.offset, (self.filled / self.element) as i64);
			if !array.is_null() {
				Ok(cx.root_object(array))
			} else if let Some(exception) = Exception::new(cx) {
				Err(exception)
			} else {
				Err(Error::new("Failed to Initialise Array Buffer", ErrorKind::Normal).into())
			}
		}
	}

	pub(crate) fn commit(&mut self, cx: &Context, reader: &mut ByobReader, state: State) -> ResultExc<()> {
		let mut done = false;

		let buffer = ArrayBuffer::from(self.buffer.get()).unwrap();
		if state == State::Closed {
			done = true;
		}
		let (object, _) = transfer_array_buffer(cx, buffer, false)?;

		self.buffer.set(*object);

		let view = unsafe { self.construct(cx)?.as_value(cx) };
		let result = unsafe { ReadResult { value: Some(**view), done }.as_value(cx) };

		let request = reader.requests.pop_front().unwrap();
		let request = Promise::from(unsafe { Local::from_raw_handle(request.handle()) }).unwrap();

		if !done {
			request.resolve(cx, &result);
		} else {
			request.reject(cx, &result);
		}
		Ok(())
	}
}

unsafe impl Traceable for PullIntoDescriptor {
	unsafe fn trace(&self, trc: *mut JSTracer) {
		self.buffer.trace(trc);
	}
}

pub(crate) fn transfer_array_buffer<'cx>(cx: &'cx Context, buffer: ArrayBuffer, shared: bool) -> Result<(Local<'cx, *mut JSObject>, ArrayBuffer)> {
	unsafe {
		let object = cx.root_object(*buffer.underlying_object());
		if IsDetachedArrayBufferObject(*object) || shared {
			return Err(Error::new("Chunk must not be detached and not shared.", ErrorKind::Type));
		}

		let bytes = buffer.len();
		let data = StealArrayBufferContents(**cx, object.handle().into());
		let buffer = cx.root_object(NewArrayBufferWithContents(**cx, bytes, data));
		let buffer_value = buffer.as_value(cx);
		ArrayBuffer::from_value(cx, &buffer_value, true, ()).map(move |buf| (buffer, buf))
	}
}

pub enum Controller {
	Default(&'static mut DefaultController),
	ByteStream(&'static mut ByteStreamController),
}

impl Controller {
	pub fn start(&self, cx: &Context, object: &Box<Heap<*mut JSObject>>) {
		let inner = match self {
			Controller::Default(controller) => controller.start.as_ref().map(|start| (start, controller.underlying_source.as_ref())),
			Controller::ByteStream(controller) => controller.start.as_ref().map(|start| (start, Some(&controller.underlying_source))),
		};
		if let Some((start, underlying_source)) = inner {
			let start = Function::from(unsafe { Local::from_raw_handle(start.handle()) });
			let underlying_source = underlying_source
				.map(|s| Object::from(unsafe { Local::from_raw_handle(s.handle()) }))
				.unwrap_or_else(|| Object::null(cx));
			let mut value = Value::null(cx);
			value.handle_mut().set(ObjectValue(object.get()));
			let result = start.call(cx, &underlying_source, &[value]).map(|v| **v);

			let obj = object.get();
			let handle = unsafe { object.handle() };

			let mut promise = Promise::new_with_executor(cx, move |cx, resolve, reject| {
				let null = Object::null(cx);
				match result {
					Ok(value) => {
						let value = Value::from(cx.root_value(value));
						let _ = resolve.call(cx, &null, &[value]);
					}
					Err(Some(report)) => {
						let value = unsafe { report.exception.as_value(cx) };
						let _ = reject.call(cx, &null, &[value]);
					}
					Err(None) => unreachable!(),
				}
				Ok(())
			}).unwrap();
			match self {
				Controller::Default(_) => {
					promise.add_reactions(
						cx,
						move |cx, _| {
							let object = Object::from(unsafe { Local::from_raw_handle(handle) });
							let controller = DefaultController::get_private(&object);
							controller.started = true;
							let res = controller.pull_if_needed(cx);
							unsafe { Context::unroot_persistent_object(obj) };
							res.map(|_| Value::undefined(cx))
						},
						move |cx, error| {
							let object = Object::from(unsafe { Local::from_raw_handle(handle) });
							let controller = DefaultController::get_private(&object);
							let res = controller.error_internal(cx, error);
							unsafe { Context::unroot_persistent_object(obj) };
							res.map(|_| Value::undefined(cx)).map_err(Into::into)
						},
					);
				}
				Controller::ByteStream(_) => {
					promise.add_reactions(
						cx,
						move |cx, _| {
							let object = Object::from(unsafe { Local::from_raw_handle(handle) });
							let controller = ByteStreamController::get_private(&object);
							controller.started = true;
							let res = controller.pull_if_needed(cx);
							unsafe { Context::unroot_persistent_object(obj) };
							res.map(|_| Value::undefined(cx))
						},
						move |cx, error| {
							let object = Object::from(unsafe { Local::from_raw_handle(handle) });
							let controller = ByteStreamController::get_private(&object);
							let res = controller.error_internal(cx, error);
							unsafe { Context::unroot_persistent_object(obj) };
							res.map(|_| Value::undefined(cx)).map_err(Into::into)
						},
					);
				}
			}
		}
	}

	pub fn cancel<'cx: 'v, 'v>(
		&mut self, cx: &'cx Context, reason: Option<Value<'v>>, object: &Box<Heap<*mut JSObject>>,
	) -> ResultExc<Promise<'cx>> {
		let cancel = match self {
			Controller::Default(controller) => {
				controller.queue.clear();
				controller.queue_size = 0;

				controller.pull = None;
				controller.size = None;
				controller.cancel.take()
			}
			Controller::ByteStream(controller) => {
				controller.pending_descriptors.clear();
				controller.queue.clear();
				controller.queue_size = 0;

				controller.pull = None;
				controller.cancel.take()
			}
		};
		let mut promise = Promise::new(cx);
		if let Some(cancel) = &cancel {
			let cancel = Function::from(unsafe { Local::from_raw_handle(cancel.handle()) });
			let this = Object::from(unsafe { Local::from_raw_handle(object.handle()) });
			let reason = reason.unwrap_or_else(|| Value::undefined(cx));
			let value = cancel.call(cx, &this, &[reason]).map_err(|report| report.unwrap().exception)?;
			if let Ok(mut result) = unsafe { Promise::from_value(cx, &value, true, ()) } {
				result.then(cx, |cx, _| Ok(Value::undefined(cx)));
				promise = result;
			} else {
				promise.resolve(cx, &Value::undefined(cx));
			}
		}
		Ok(promise)
	}

	pub fn pull(&mut self, cx: &Context, request: &Promise) -> ResultExc<()> {
		match self {
			Controller::Default(controller) => {
				let stream = Object::from(unsafe { Local::from_raw_handle(controller.stream.handle()) });
				let stream = ReadableStream::get_private(&stream);

				if let Some((chunk, _)) = controller.queue.pop_front() {
					if controller.close_requested && controller.queue.is_empty() {
						controller.pull = None;
						controller.cancel = None;
						controller.size = None;

						stream.close(cx)?;
					} else {
						controller.pull_if_needed(cx)?;
					}
					let result = ReadResult { value: Some(chunk.get()), done: false };
					request.resolve(cx, unsafe { &result.as_value(cx) });
				} else {
					match &mut stream.get_reader() {
						Some(Reader::Default(reader)) => {
							if stream.state != State::Readable {
								return Err(Error::new("Cannot Add Read Request to Read Queue", None).into());
							} else {
								reader.requests.push_back(Heap::default());
								reader.requests[reader.requests.len() - 1].set(***request);
							}
						}
						_ => return Ok(()),
					}
					controller.pull_if_needed(cx)?;
				}
			}
			Controller::ByteStream(controller) => {
				let stream = Object::from(unsafe { Local::from_raw_handle(controller.stream.handle()) });
				let stream = ReadableStream::get_private(&stream);

				if stream.reader_kind != ReaderKind::Default {
					return Err(Error::new("Reader should have default reader.", ErrorKind::Type).into());
				}
				if controller.queue_size > 0 {
					let (buffer, offset, length) = controller.queue.pop_front().unwrap();
					controller.queue_size -= length;

					if controller.queue_size == 0 && controller.close_requested {
						controller.close(cx)?;
					} else {
						controller.pull_if_needed(cx)?;
					}

					let value = cx.root_value(ObjectValue(unsafe { JS_NewUint8ArrayWithBuffer(**cx, buffer.handle(), offset, length as i64) }));
					let result = ReadResult { value: Some(*value), done: false };
					request.resolve(cx, unsafe { &result.as_value(cx) });
				} else {
					if controller.auto_allocate_chunk_size != 0 {
						let mut object = Object::new(cx);
						unsafe {
							if ArrayBuffer::create(**cx, CreateWith::Length(controller.auto_allocate_chunk_size), object.handle_mut()).is_err() {
								controller.error_internal(cx, &Exception::new(cx).unwrap().as_value(cx))?;
								return Ok(());
							}
						}
						let descriptor = PullIntoDescriptor {
							buffer: Heap::default(),
							offset: 0,
							length: controller.auto_allocate_chunk_size,
							filled: 0,
							element: 1,
							constructor: JS_NewUint8ArrayWithBuffer,
							kind: ReaderKind::Default,
						};
						controller.pending_descriptors.push_back(descriptor);
						controller.pending_descriptors[controller.pending_descriptors.len() - 1].buffer.set(**object);
					}
					match stream.get_reader() {
						Some(Reader::Default(reader)) => {
							reader.requests.push_back(Heap::default());
							reader.requests[reader.requests.len() - 1].set(***request);
						}
						_ => {}
					}
					controller.pull_if_needed(cx)?;
				}
			}
		}
		Ok(())
	}

	pub fn release(&mut self) {
		match self {
			Controller::Default(_) => {}
			Controller::ByteStream(controller) => {
				if let Some(descriptor) = controller.pending_descriptors.pop_front() {
					controller.pending_descriptors.clear();
					let buffer = descriptor.buffer.get();

					controller.pending_descriptors.push_back(descriptor);
					controller.pending_descriptors[0].buffer.set(buffer);
				}
			}
		}
	}
}

#[js_class]
mod default {
	use mozjs::jsapi::{Heap, JSFunction, JSObject, JSTracer};
	use mozjs::jsval::{DoubleValue, Int32Value, NullValue, JSVal};
	use mozjs::gc::Traceable;
	use ion::{ClassInitialiser, Context, Function, Object, Local, Promise, Error, ErrorKind, Result, Value, ResultExc};
	use ion::conversions::{FromValue, ToValue};
	use crate::globals::streams::readable::{QueueingStrategy, UnderlyingSource, State};
	use crate::globals::streams::readable::stream::ReadableStream;
	use crate::globals::streams::readable::reader::{Reader, ReadResult};
	use std::mem::transmute;
	use std::collections::vec_deque::VecDeque;
	use mozjs::conversions::ConversionBehavior;

	#[ion(no_constructor, name = "ReadableStreamDefaultController")]
	pub struct DefaultController {
		pub(crate) underlying_source: Option<Box<Heap<*mut JSObject>>>,
		pub(crate) start: Option<Box<Heap<*mut JSFunction>>>,
		pub(crate) pull: Option<Box<Heap<*mut JSFunction>>>,
		pub(crate) cancel: Option<Box<Heap<*mut JSFunction>>>,
		pub(crate) size: Option<Box<Heap<*mut JSFunction>>>,

		pub(crate) stream: Box<Heap<*mut JSObject>>,

		high_water_mark: f64,

		pub(crate) started: bool,
		pub(crate) pulling: bool,
		pub(crate) pull_again: bool,
		pub(crate) close_requested: bool,

		pub(crate) queue: VecDeque<(Heap<JSVal>, u64)>,
		pub(crate) queue_size: u64,
	}

	impl DefaultController {
		pub(crate) fn initialise(
			cx: &Context, stream: &Object, source_object: Option<&Object>, source: &UnderlyingSource, strategy: &QueueingStrategy,
			high_water_mark: f64,
		) -> (Box<Heap<*mut JSObject>>, &'static mut DefaultController) {
			let source_object = source_object.map(|o| Heap::boxed(***o));
			let start = source.start.as_ref().map(|s| Heap::boxed(***s));
			let pull = source.pull.as_ref().map(|p| Heap::boxed(***p));
			let cancel = source.cancel.as_ref().map(|c| Heap::boxed(***c));
			let size = strategy.size.as_ref().map(|s| Heap::boxed(***s));

			let controller = DefaultController {
				underlying_source: source_object,
				start,
				pull,
				cancel,
				size,
				stream: Heap::boxed(***stream),

				high_water_mark,

				started: false,
				pulling: false,
				pull_again: false,
				close_requested: false,

				queue: VecDeque::new(),
				queue_size: 0,
			};

			let heap = Heap::boxed(DefaultController::new_object(cx, controller));
			let object = Object::from(unsafe { Local::from_raw_handle(heap.handle()) });
			let controller = unsafe { transmute(DefaultController::get_private(&object)) };
			(heap, controller)
		}

		pub(crate) fn get_state(&self) -> State {
			let stream = Object::from(unsafe { Local::from_raw_handle(self.stream.handle()) });
			let stream = ReadableStream::get_private(&stream);
			stream.state
		}

		pub(crate) fn can_close_or_enqueue(&self) -> bool {
			self.get_state() == State::Readable && !self.close_requested
		}

		pub(crate) fn should_call_pull(&self) -> bool {
			if !self.can_close_or_enqueue() || !self.started {
				return false;
			}
			let stream = Object::from(unsafe { Local::from_raw_handle(self.stream.handle()) });
			let stream = ReadableStream::get_private(&stream);
			if let Some(Reader::Default(reader)) = &mut stream.get_reader() {
				if reader.requests.len() > 0 {
					return true;
				}
			}
			self.get_state() == State::Readable && self.high_water_mark > self.queue_size as f64
		}

		pub(crate) fn pull_if_needed(&mut self, cx: &Context) -> ResultExc<()> {
			if !self.should_call_pull() {
				return Ok(());
			}
			if self.pulling {
				self.pull_again = true;
				return Ok(());
			}

			self.pulling = true;
			let stream = Object::from(unsafe { Local::from_raw_handle(self.stream.handle()) });
			let stream = ReadableStream::get_private(&stream);

			if let Some(pull) = &self.pull {
				let pull = Function::from(unsafe { Local::from_raw_handle(pull.handle()) });
				let this = self
					.underlying_source
					.as_ref()
					.map(|s| Object::from(unsafe { Local::from_raw_handle(s.handle()) }))
					.unwrap_or_else(|| Object::null(cx));
				let result = pull
					.call(cx, &this, unsafe { &[stream.controller_object.get().as_value(cx)] })
					.map_err(|report| report.unwrap().exception)?;
				let handle = unsafe { stream.controller_object.handle() };

				let mut promise = match unsafe { Promise::from_value(cx, &result, true, ()) } {
					Ok(promise) => promise,
					Err(_) => Promise::new(cx)
				};
				promise.add_reactions(
					cx,
					move |cx, _| {
						let object = Object::from(unsafe { Local::from_raw_handle(handle) });
						let controller = DefaultController::get_private(&object);
						controller.pulling = false;
						let mut res = Ok(());
						if controller.pull_again {
							controller.pull_again = false;
							res = controller.pull_if_needed(cx);
						}
						res.map(|_| Value::undefined(cx))
					},
					move |cx, error| {
						let object = Object::from(unsafe { Local::from_raw_handle(handle) });
						let controller = DefaultController::get_private(&object);
						let res = controller.error_internal(cx, error);
						res.map(|_| Value::undefined(cx)).map_err(Into::into)
					},
				);
			}
			Ok(())
		}

		pub(crate) fn error_internal(&mut self, cx: &Context, error: &Value) -> Result<()> {
			if self.get_state() == State::Readable {
				self.queue.clear();
				self.queue_size = 0;
				self.pull = None;
				self.cancel = None;
				self.size = None;

				let stream = Object::from(unsafe { Local::from_raw_handle(self.stream.handle()) });
				let stream = ReadableStream::get_private(&stream);
				stream.error(cx, error)
			} else {
				Ok(())
			}
		}

		#[ion(get)]
		pub fn get_desired_size(&self) -> JSVal {
			match self.get_state() {
				State::Readable => DoubleValue(self.high_water_mark - self.queue_size as f64),
				State::Closed => Int32Value(0),
				State::Errored => NullValue(),
			}
		}

		pub fn close(&mut self, cx: &Context) -> Result<()> {
			if self.can_close_or_enqueue() {
				if self.queue.is_empty() {
					self.close_requested = true;
				}
				self.pull = None;
				self.cancel = None;
				self.size = None;

				let stream = Object::from(unsafe { Local::from_raw_handle(self.stream.handle()) });
				let stream = ReadableStream::get_private(&stream);
				stream.close(cx)
			} else {
				Err(Error::new("Cannot Close Stream", ErrorKind::Type))
			}
		}

		pub fn enqueue(&mut self, cx: &Context, chunk: Value) -> ResultExc<()> {
			if self.can_close_or_enqueue() {
				let stream = Object::from(unsafe { Local::from_raw_handle(self.stream.handle()) });
				let stream = ReadableStream::get_private(&stream);
				if let Some(Reader::Default(reader)) = &mut stream.get_reader() {
					if let Some(request) = reader.requests.pop_front() {
						let request = Promise::from(unsafe { Local::from_raw_handle(request.handle()) }).unwrap();
						let req = ReadResult { value: Some(**chunk), done: false };
						request.resolve(cx, unsafe { &req.as_value(cx) });
						return Ok(());
					}
				}
				let args = &[chunk];
				let result = self
					.size
					.as_ref()
					.map(|size| {
						let size = Function::from(unsafe { Local::from_raw_handle(size.handle()) });
						size.call(cx, &Object::null(cx), args)
					})
					.unwrap_or_else(|| Ok(Value::i32(cx, 1)));
				match result {
					Ok(size) => {
						let size = unsafe { u64::from_value(cx, &size, false, ConversionBehavior::EnforceRange) };
						match size {
							Ok(size) => {
								self.queue.push_back((*Heap::boxed(**args[0]), size));
								self.queue_size += size;
								self.pull_if_needed(cx)?;
							}
							Err(error) => {
								self.error_internal(cx, unsafe { &error.as_value(cx) })?;
							}
						}
					}
					Err(Some(report)) => {
						self.error_internal(cx, unsafe { &report.exception.as_value(cx) })?;
					}
					Err(None) => unreachable!(),
				}
				Ok(())
			} else {
				Err(Error::new("Cannot Enqueue to Stream", ErrorKind::Type).into())
			}
		}

		pub fn error<'cx>(&mut self, cx: &'cx Context, error: Option<Value<'cx>>) -> Result<()> {
			self.error_internal(cx, &error.unwrap_or_else(|| Value::undefined(cx)))
		}
	}

	unsafe impl Traceable for DefaultController {
		unsafe fn trace(&self, trc: *mut JSTracer) {
			self.underlying_source.trace(trc);
			self.start.trace(trc);
			self.pull.trace(trc);
			self.cancel.trace(trc);
			self.size.trace(trc);
			self.stream.trace(trc);

			for (chunk, _) in &self.queue {
				chunk.trace(trc);
			}
		}
	}
}

#[js_class]
mod byte_stream {
	use ion::{ClassInitialiser, Context, Error, ErrorKind, Local, Promise, Object, Result, Exception, Value, Function, ResultExc};
	use ion::conversions::{FromValue, ToValue};
	use mozjs::jsapi::{Heap, JSFunction, JSObject, JSTracer, JS_NewUint8ArrayWithBuffer, JS_GetArrayBufferViewByteOffset, IsDetachedArrayBufferObject, ArrayBufferClone, JS_GetArrayBufferViewBuffer, ArrayBufferCopyData};
	use mozjs::jsval::{JSVal, DoubleValue, Int32Value, ObjectValue, NullValue};
	use mozjs::typedarray::{ArrayBuffer, ArrayBufferView};
	use mozjs::gc::Traceable;
	use crate::globals::streams::readable::{State, UnderlyingSource};
	use crate::globals::streams::readable::stream::ReadableStream;
	use crate::globals::streams::readable::controller::{transfer_array_buffer, PullIntoDescriptor};
	use crate::globals::streams::readable::reader::{Reader, ReadResult, ReaderKind};
	use std::mem::transmute;
	use std::collections::vec_deque::VecDeque;

	#[ion(no_constructor, name = "ReadableByteStreamController")]
	pub struct ByteStreamController {
		pub(crate) underlying_source: Box<Heap<*mut JSObject>>,
		pub(crate) start: Option<Box<Heap<*mut JSFunction>>>,
		pub(crate) pull: Option<Box<Heap<*mut JSFunction>>>,
		pub(crate) cancel: Option<Box<Heap<*mut JSFunction>>>,

		pub(crate) stream: Box<Heap<*mut JSObject>>,

		pub(crate) auto_allocate_chunk_size: usize,
		high_water_mark: f64,

		pub(crate) started: bool,
		pub(crate) pulling: bool,
		pub(crate) pull_again: bool,
		pub(crate) close_requested: bool,

		pub(crate) pending_descriptors: VecDeque<PullIntoDescriptor>,
		pub(crate) queue: VecDeque<(Heap<*mut JSObject>, usize, usize)>,
		pub(crate) queue_size: usize,
	}

	impl ByteStreamController {
		pub(crate) fn initialise(
			cx: &Context, stream: &Object, source_object: &Object, source: &UnderlyingSource, high_water_mark: f64,
		) -> Result<(Box<Heap<*mut JSObject>>, &'static mut ByteStreamController)> {
			let source_object = Heap::boxed(***source_object);
			let start = source.start.as_ref().map(|s| Heap::boxed(***s));
			let pull = source.pull.as_ref().map(|p| Heap::boxed(***p));
			let cancel = source.cancel.as_ref().map(|c| Heap::boxed(***c));
			if let Some(auto_allocate_chunk_size) = source.auto_allocate_chunk_size {
				if auto_allocate_chunk_size == 0 {
					return Err(Error::new("autoAllocateChunkSize can not be zero.", ErrorKind::Type));
				}
			}

			let controller = ByteStreamController {
				underlying_source: source_object,
				start,
				pull,
				cancel,

				stream: Heap::boxed(***stream),

				auto_allocate_chunk_size: source.auto_allocate_chunk_size.unwrap_or(0) as usize,
				high_water_mark,

				started: false,
				pulling: false,
				pull_again: false,
				close_requested: false,

				pending_descriptors: VecDeque::new(),
				queue: VecDeque::new(),
				queue_size: 0,
			};

			let heap = Heap::boxed(ByteStreamController::new_object(cx, controller));
			let object = Object::from(unsafe { Local::from_raw_handle(heap.handle()) });
			let controller = unsafe { transmute(ByteStreamController::get_private(&object)) };
			Ok((heap, controller))
		}

		pub(crate) fn get_state(&self) -> State {
			let stream = Object::from(unsafe { Local::from_raw_handle(self.stream.handle()) });
			let stream = ReadableStream::get_private(&stream);
			stream.state
		}

		pub(crate) fn can_close_or_enqueue(&self) -> bool {
			self.get_state() == State::Readable && !self.close_requested
		}

		pub(crate) fn should_call_pull(&self) -> bool {
			if !self.can_close_or_enqueue() || !self.started {
				return false;
			}
			let stream = Object::from(unsafe { Local::from_raw_handle(self.stream.handle()) });
			let stream = ReadableStream::get_private(&stream);
			match stream.get_reader() {
				Some(Reader::Default(reader)) => {
					if reader.requests.len() > 0 {
						return true;
					}
				}
				Some(Reader::Byob(reader)) => {
					if reader.requests.len() > 0 {
						return true;
					}
				}
				None => {},
			}
			self.get_state() == State::Readable && self.high_water_mark > self.queue_size as f64
		}

		pub(crate) fn pull_if_needed(&mut self, cx: &Context) -> ResultExc<()> {
			if self.should_call_pull() {
				if self.pulling {
					self.pull_again = true;
					return Ok(());
				}

				self.pulling = true;
				let stream = Object::from(unsafe { Local::from_raw_handle(self.stream.handle()) });
				let stream = ReadableStream::get_private(&stream);

				if let Some(pull) = &self.pull {
					let pull = Function::from(unsafe { Local::from_raw_handle(pull.handle()) });
					let this = Object::from(unsafe { Local::from_raw_handle(self.underlying_source.handle()) });
					let result = pull
						.call(cx, &this, unsafe { &[stream.controller_object.get().as_value(cx)] })
						.map_err(|report| report.unwrap().exception)?;
					let handle = unsafe { stream.controller_object.handle() };

					let mut promise = match unsafe { Promise::from_value(cx, &result, true, ()) } {
						Ok(promise) => promise,
						Err(_) => Promise::new(cx),
					};
					promise.add_reactions(
						cx,
						move |cx, _| {
							let object = Object::from(unsafe { Local::from_raw_handle(handle) });
							let controller = ByteStreamController::get_private(&object);
							controller.pulling = false;
							let mut res = Ok(());
							if controller.pull_again {
								controller.pull_again = false;
								res = controller.pull_if_needed(cx);
							}
							res.map(|_| Value::undefined(cx))
						},
						move |cx, error| {
							let object = Object::from(unsafe { Local::from_raw_handle(handle) });
							let controller = ByteStreamController::get_private(&object);
							let res = controller.error_internal(cx, error);
							res.map(|_| Value::undefined(cx)).map_err(Into::into)
						},
					);
				}
			}
			Ok(())
		}

		pub(crate) fn fill_pull_into_descriptor(&mut self, cx: &Context, descriptor: &mut PullIntoDescriptor) -> ResultExc<bool> {
			let aligned = descriptor.filled - descriptor.filled % descriptor.element;
			let max_copy = self.queue_size.min(descriptor.length - descriptor.filled);
			let max_aligned = descriptor.filled + max_copy - (descriptor.filled + max_copy) % descriptor.element;

			let ready = max_aligned > aligned;

			let mut remaining = if ready {
				max_aligned - descriptor.filled
			} else {
				max_copy
			};

			while remaining > 0 {
				let mut copy = remaining;
				let mut len = 0;

				if let Some((chunk, offset, length)) = self.queue.get(0) {
					copy = copy.min(*length);
					len = *length;
					unsafe {
						if !ArrayBufferCopyData(
							**cx,
							descriptor.buffer.handle(),
							descriptor.offset + descriptor.filled,
							chunk.handle(),
							*offset,
							copy,
						) {
							return Err(Exception::new(cx).unwrap());
						}
					}
				}
				if copy == len {
					self.queue.pop_front();
				} else {
					if let Some((_, offset, length)) = self.queue.get_mut(0) {
						*offset += copy;
						*length -= copy;
					}
				}
				self.queue_size -= copy;
				descriptor.filled += copy;
				remaining -= copy;
			}

			if !ready {
				// TODO: Assert Queue Size 0, Assert Filled > 0, Assert Filled < Element Size
			}

			Ok(ready)
		}

		pub(crate) fn error_internal(&mut self, cx: &Context, error: &Value) -> Result<()> {
			if self.get_state() == State::Readable {
				self.pending_descriptors.clear();
				self.queue.clear();
				self.queue_size = 0;
				self.pull = None;
				self.cancel = None;

				let stream = Object::from(unsafe { Local::from_raw_handle(self.stream.handle()) });
				let stream = ReadableStream::get_private(&stream);
				stream.error(cx, error)
			} else {
				Ok(())
			}
		}

		#[ion(get)]
		pub fn get_desired_size(&self) -> JSVal {
			match self.get_state() {
				State::Readable => DoubleValue(self.high_water_mark - self.queue_size as f64),
				State::Closed => Int32Value(0),
				State::Errored => NullValue(),
			}
		}

		pub fn close(&mut self, cx: &Context) -> Result<()> {
			if self.can_close_or_enqueue() {
				if self.queue_size > 0 {
					self.close_requested = true;
				}
				if let Some(descriptor) = self.pending_descriptors.get(0) {
					if descriptor.filled > 0 {
						let error = Error::new("Pending Pull Into Not Empty", ErrorKind::Type);
						self.error_internal(cx, &unsafe { error.as_value(cx) })?;
						return Err(error);
					}
				}

				self.pull = None;
				self.cancel = None;
				let stream = Object::from(unsafe { Local::from_raw_handle(self.stream.handle()) });
				let stream = ReadableStream::get_private(&stream);
				stream.close(cx)
			} else {
				Err(Error::new("Cannot Close Byte Stream Controller", ErrorKind::Type))
			}
		}

		pub fn enqueue(&mut self, cx: &Context, chunk: ArrayBufferView) -> ResultExc<()> {
			if chunk.len() == 0 {
				return Err(Error::new("Chunk must contain bytes.", ErrorKind::Type).into());
			}

			let mut shared = false;
			let chunk_object = cx.root_object(unsafe { *chunk.underlying_object() });
			let object = cx.root_object(unsafe { JS_GetArrayBufferViewBuffer(**cx, chunk_object.handle().into(), &mut shared) });
			let buffer = ArrayBuffer::from(*object).unwrap();

			if buffer.len() == 0 {
				return Err(Error::new("Chunk must contain bytes.", ErrorKind::Type).into());
			}

			if self.can_close_or_enqueue() {
				let (object, _) = transfer_array_buffer(cx, buffer, shared)?;
				let offset = unsafe { JS_GetArrayBufferViewByteOffset(*chunk_object) };

				let mut shift = false;
				if let Some(descriptor) = self.pending_descriptors.get_mut(0) {
					if unsafe { IsDetachedArrayBufferObject(descriptor.buffer.get()) } {
						return Err(Error::new("Pull-Into Descriptor Buffer is detached", ErrorKind::Type).into());
					}

					let buffer = ArrayBuffer::from(descriptor.buffer.get()).unwrap();
					let (object, _) = transfer_array_buffer(cx, buffer, false)?;
					descriptor.buffer.set(*object);
					if descriptor.kind == ReaderKind::None && descriptor.filled > 0 {
						let buffer = unsafe { ArrayBufferClone(**cx, object.handle().into(), descriptor.offset, descriptor.length) };
						if let Some(exception) = Exception::new(cx) {
							self.error_internal(cx, &unsafe { exception.as_value(cx) })?;
							return Err(exception);
						}
						self.queue.push_back((Heap::default(), 0, descriptor.length));
						self.queue[self.queue.len() - 1].0.set(buffer);
						self.queue_size += descriptor.length;
						shift = true;
					}
				}

				if shift {
					self.pending_descriptors.pop_front();
				}

				let stream = Object::from(unsafe { Local::from_raw_handle(self.stream.handle()) });
				let stream = ReadableStream::get_private(&stream);

				match stream.get_reader() {
					Some(Reader::Default(reader)) => {
						let mut complete = false;
						while let Some(request) = reader.requests.pop_front() {
							if self.queue_size == 0 {
								self.pending_descriptors.pop_front();
								let value = cx.root_value(ObjectValue(unsafe { JS_NewUint8ArrayWithBuffer(**cx, object.handle().into(), offset, chunk.len() as i64) }));
								let request = Promise::from(unsafe { Local::from_raw_handle(request.handle()) }).unwrap();
								let result = ReadResult { value: Some(*value), done: false };
								request.resolve(cx, unsafe { &result.as_value(cx) });

								complete = true;
								break;
							}
							let (buffer, offset, length) = self.queue.pop_front().unwrap();
							self.queue_size -= length;

							if self.queue_size == 0 && self.close_requested {
								self.close(cx)?;
							} else {
								self.pull_if_needed(cx)?;
							}

							let value = cx.root_value(ObjectValue(unsafe { JS_NewUint8ArrayWithBuffer(**cx, buffer.handle(), offset, length as i64) }));
							let request = Promise::from(unsafe { Local::from_raw_handle(request.handle()) }).unwrap();
							let result = ReadResult { value: Some(*value), done: false };
							request.resolve(cx, unsafe { &result.as_value(cx) });
						}

						if !complete {
							self.queue.push_back((Heap::default(), offset, chunk.len()));
							self.queue[self.queue.len() - 1].0.set(*object);
							self.queue_size += chunk.len();
						}
					}
					Some(Reader::Byob(reader)) => {
						self.queue.push_back((Heap::default(), offset, chunk.len()));
						self.queue[self.queue.len() - 1].0.set(*object);
						self.queue_size += chunk.len();

						while !self.pending_descriptors.is_empty() {
							if self.queue_size == 0 {
								break;
							}

							let mut shift = false;

							let descriptor = self.pending_descriptors.get_mut(0).unwrap() as *mut _;
							if self.fill_pull_into_descriptor(cx, unsafe { &mut *descriptor })? {
								shift = true;
							}

							if shift {
								let mut descriptor = self.pending_descriptors.pop_front().unwrap();
								descriptor.commit(cx, reader, stream.state)?;
							}
						}
					}
					None => {
						self.queue.push_back((Heap::default(), offset, chunk.len()));
						self.queue[self.queue.len() - 1].0.set(*object);
						self.queue_size += chunk.len();
					}
				}
				self.pull_if_needed(cx)
			} else {
				Err(Error::new("Cannot Enqueue to Stream", ErrorKind::Type).into())
			}
		}

		pub fn error<'cx>(&mut self, cx: &'cx Context, error: Option<Value<'cx>>) -> Result<()> {
			self.error_internal(cx, &error.unwrap_or_else(|| Value::undefined(cx)))
		}
	}

	unsafe impl Traceable for ByteStreamController {
		unsafe fn trace(&self, trc: *mut JSTracer) {
			self.underlying_source.trace(trc);
			self.start.trace(trc);
			self.pull.trace(trc);
			self.cancel.trace(trc);
			self.stream.trace(trc);

			self.pending_descriptors.trace(trc);
			for (chunk, _, _) in &self.queue {
				chunk.trace(trc);
			}
		}
	}
}
