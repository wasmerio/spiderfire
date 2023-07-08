/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use mozjs::jsval::{JSVal, UndefinedValue};

pub use byob::ByobReader;
pub use default::DefaultReader;
use ion::{Context, Object, Value};
use ion::conversions::ToValue;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ReaderKind {
	None,
	Default,
	Byob,
}

pub enum Reader {
	Default(&'static mut DefaultReader),
	Byob(&'static mut ByobReader),
}

pub struct ReadResult {
	pub value: Option<JSVal>,
	pub done: bool,
}

impl<'cx> ToValue<'cx> for ReadResult {
	unsafe fn to_value(&self, cx: &'cx Context, value: &mut Value) {
		let mut object = Object::new(cx);
		object.set(cx, "value", &Value::from(cx.root_value(self.value.unwrap_or_else(UndefinedValue))));
		object.set_as(cx, "done", &self.done);
		object.to_value(cx, value);
	}
}

#[js_class]
mod default {
	use std::collections::vec_deque::VecDeque;

	use mozjs::gc::Traceable;
	use mozjs::jsapi::{Heap, JSObject, JSTracer};

	use ion::{ClassInitialiser, Context, Error, ErrorKind, Local, Object, Promise, Result, ResultExc, Value};
	use ion::conversions::ToValue;

	use crate::globals::streams::readable::reader::{ReaderKind, ReadResult};
	use crate::globals::streams::readable::State;
	use crate::globals::streams::readable::stream::ReadableStream;

	#[ion(name = "ReadableStreamDefaultReader")]
	pub struct DefaultReader {
		stream: Option<Box<Heap<*mut JSObject>>>,
		pub(crate) requests: VecDeque<Heap<*mut JSObject>>,
		pub(crate) closed: Box<Heap<*mut JSObject>>,
	}

	impl DefaultReader {
		#[ion(constructor)]
		pub fn constructor(cx: &Context, #[ion(this)] this: &Object, stream_object: Object) -> Result<DefaultReader> {
			let reader = DefaultReader::new(cx, &stream_object)?;

			let stream = ReadableStream::get_private(&stream_object);
			stream.reader_kind = ReaderKind::Default;
			stream.reader_object = Some(Heap::boxed(***this));

			Ok(reader)
		}

		pub(crate) fn new(cx: &Context, stream_object: &Object) -> Result<DefaultReader> {
			if !ReadableStream::instance_of(cx, stream_object, None) {
				return Err(Error::new("Expected ReadableStream", ErrorKind::Type));
			}

			let stream = ReadableStream::get_private(stream_object);
			if stream.get_locked() {
				return Err(Error::new("Cannot create DefaultReader from locked stream.", ErrorKind::Type));
			}

			let closed = Promise::new(cx);
			match stream.state {
				State::Readable => {}
				State::Closed => {
					closed.resolve(cx, &Value::undefined(cx));
				}
				State::Errored => {
					closed.reject(
						cx,
						&stream
							.error
							.as_ref()
							.map(|error| Value::from(unsafe { Local::from_heap(error) }))
							.unwrap_or_else(|| Value::undefined(cx)),
					);
				}
			}

			Ok(DefaultReader {
				stream: Some(Heap::boxed(***stream_object)),
				requests: VecDeque::new(),
				closed: Heap::boxed(**closed),
			})
		}

		pub fn cancel<'cx: 'v, 'v>(&self, cx: &'cx Context, reason: Option<Value<'v>>) -> ResultExc<Promise<'cx>> {
			if let Some(stream) = &self.stream {
				let stream = Object::from(unsafe { Local::from_heap(stream) });
				let stream = ReadableStream::get_private(&stream);
				stream.cancel(cx, reason)
			} else {
				let promise = Promise::new(cx);
				promise.reject(cx, &unsafe {
					Error::new("Reader has already been released.", ErrorKind::Type).as_value(cx)
				});
				Ok(promise)
			}
		}

		pub fn read<'cx>(&mut self, cx: &'cx Context) -> ResultExc<Promise<'cx>> {
			if let Some(stream) = &self.stream {
				let stream = Object::from(unsafe { Local::from_heap(stream) });
				let stream = ReadableStream::get_private(&stream);
				stream.disturbed = true;

				let promise = Promise::new(cx);
				match stream.state {
					State::Readable => stream.controller.pull(cx, &promise)?,
					State::Closed => unsafe {
						let request = ReadResult { value: None, done: true };
						promise.resolve(cx, &request.as_value(cx));
					},
					State::Errored => {
						promise.reject(
							cx,
							&stream
								.error
								.as_ref()
								.map(|error| Value::from(unsafe { Local::from_heap(error) }))
								.unwrap_or_else(|| Value::undefined(cx)),
						);
					}
				}
				Ok(promise)
			} else {
				let promise = Promise::new(cx);
				promise.reject(cx, &unsafe {
					Error::new("Reader has already been released.", ErrorKind::Type).as_value(cx)
				});
				Ok(promise)
			}
		}

		pub fn releaseLock(&mut self, cx: &Context) -> Result<()> {
			if let Some(stream) = &self.stream {
				let stream = Object::from(unsafe { Local::from_heap(stream) });
				let stream = ReadableStream::get_private(&stream);

				let mut closed = Promise::from(unsafe { Local::from_heap(&self.closed) }).unwrap();
				match stream.state {
					State::Readable => {}
					_ => {
						self.closed = Heap::boxed(**Promise::new(cx));
						closed = Promise::from(unsafe { Local::from_heap(&self.closed) }).unwrap();
					}
				}
				closed.reject(cx, unsafe { &Error::new("Released Reader", ErrorKind::Type).as_value(cx) });

				stream.reader_kind = ReaderKind::None;
				stream.reader_object = None;

				stream.controller.release();

				while let Some(request) = self.requests.pop_front() {
					let request = Promise::from(unsafe { Local::from_heap(&request) }).unwrap();
					request.reject(cx, &unsafe { Error::new("Reader has been released.", ErrorKind::Type).as_value(cx) });
				}
			} else {
				return Err(Error::new("Reader has already been released.", ErrorKind::Type));
			}
			self.stream = None;
			Ok(())
		}

		#[ion(get)]
		pub fn get_closed(&self) -> *mut JSObject {
			self.closed.get()
		}
	}

	unsafe impl Traceable for DefaultReader {
		unsafe fn trace(&self, trc: *mut JSTracer) {
			self.stream.trace(trc);
			self.requests.trace(trc);
			self.closed.trace(trc);
		}
	}
}

#[js_class]
mod byob {
	use std::collections::vec_deque::VecDeque;
	use std::mem::transmute;

	use mozjs::gc::Traceable;
	use mozjs::jsapi::{
		Heap, IsDetachedArrayBufferObject, JS_GetArrayBufferViewBuffer, JS_GetArrayBufferViewByteOffset, JS_GetArrayBufferViewType,
		JS_IsTypedArrayObject, JS_NewDataView, JSObject, JSTracer,
	};
	use mozjs::typedarray::{ArrayBuffer, ArrayBufferView};

	use ion::{ClassInitialiser, Context, Error, ErrorKind, Local, Object, Promise, Result, ResultExc, Value};
	use ion::conversions::ToValue;
	use ion::typedarray::{type_to_constructor, type_to_element_size};

	use crate::globals::streams::readable::{ReadableStream, State};
	use crate::globals::streams::readable::controller::{Controller, PullIntoDescriptor, transfer_array_buffer};
	use crate::globals::streams::readable::reader::{ReaderKind, ReadResult};

	#[ion(name = "ReadableStreamBYOBReader")]
	pub struct ByobReader {
		stream: Option<Box<Heap<*mut JSObject>>>,
		pub(crate) requests: VecDeque<Heap<*mut JSObject>>,
		pub(crate) closed: Box<Heap<*mut JSObject>>,
	}

	impl ByobReader {
		#[ion(constructor)]
		pub fn constructor(cx: &Context, #[ion(this)] this: &Object, stream_object: Object) -> Result<ByobReader> {
			let reader = ByobReader::new(cx, &stream_object)?;

			let stream = ReadableStream::get_private(&stream_object);
			stream.reader_kind = ReaderKind::Byob;
			stream.reader_object = Some(Heap::boxed(***this));

			Ok(reader)
		}

		pub(crate) fn new(cx: &Context, stream_object: &Object) -> Result<ByobReader> {
			if !ReadableStream::instance_of(cx, stream_object, None) {
				return Err(Error::new("Expected ReadableStream", ErrorKind::Type));
			}

			let stream = ReadableStream::get_private(stream_object);
			if stream.get_locked() {
				return Err(Error::new("Cannot create BYOBReader from locked stream.", ErrorKind::Type));
			}

			if let Controller::Default(_) = &stream.controller {
				return Err(Error::new("Cannot create BYOBReader from DefaultController", ErrorKind::Type));
			}

			let closed = Promise::new(cx);
			match stream.state {
				State::Readable => {}
				State::Closed => {
					closed.resolve(cx, &Value::undefined(cx));
				}
				State::Errored => {
					closed.reject(
						cx,
						&stream
							.error
							.as_ref()
							.map(|error| Value::from(unsafe { Local::from_heap(error) }))
							.unwrap_or_else(|| Value::undefined(cx)),
					);
				}
			}

			Ok(ByobReader {
				stream: Some(Heap::boxed(***stream_object)),
				requests: VecDeque::new(),
				closed: Heap::boxed(**closed),
			})
		}

		pub fn cancel<'cx: 'v, 'v>(&self, cx: &'cx Context, reason: Option<Value<'v>>) -> ResultExc<Promise<'cx>> {
			if let Some(stream) = &self.stream {
				let stream = Object::from(unsafe { Local::from_heap(stream) });
				let stream = ReadableStream::get_private(&stream);
				stream.cancel(cx, reason)
			} else {
				let promise = Promise::new(cx);
				promise.reject(cx, &unsafe {
					Error::new("Reader has already been released.", ErrorKind::Type).as_value(cx)
				});
				Ok(promise)
			}
		}

		pub fn read<'cx>(&mut self, cx: &'cx Context, view: ArrayBufferView) -> ResultExc<Promise<'cx>> {
			if let Some(stream) = &self.stream {
				if view.len() == 0 {
					return Err(Error::new("Buffer must contain bytes.", ErrorKind::Type).into());
				}

				let mut shared = false;
				let view_object = cx.root_object(unsafe { *view.underlying_object() });
				let object = cx.root_object(unsafe { JS_GetArrayBufferViewBuffer(**cx, view_object.handle().into(), &mut shared) });
				let buffer = ArrayBuffer::from(*object).unwrap();

				if buffer.len() == 0 {
					return Err(Error::new("Buffer must contain bytes.", ErrorKind::Type).into());
				}

				if unsafe { IsDetachedArrayBufferObject(*object) } {
					return Err(Error::new("ArrayBuffer must not be detached.", ErrorKind::Type).into());
				}

				let request = Promise::new(cx);

				let stream = Object::from(unsafe { Local::from_heap(stream) });
				let stream = ReadableStream::get_private(&stream);
				stream.disturbed = true;
				if stream.state == State::Errored {
					let error = stream
						.error
						.as_ref()
						.map(|error| Value::from(unsafe { Local::from_heap(error) }))
						.unwrap_or_else(|| Value::undefined(cx));
					request.reject(cx, &error);
					return Ok(request);
				}

				let (constructor, element_size) = unsafe {
					if JS_IsTypedArrayObject(*view_object) {
						let ty = JS_GetArrayBufferViewType(*view_object);
						(type_to_constructor(ty), type_to_element_size(ty))
					} else {
						(transmute(JS_NewDataView as usize), 1)
					}
				};

				let offset = unsafe { JS_GetArrayBufferViewByteOffset(*view_object) };
				match transfer_array_buffer(cx, buffer, shared) {
					Ok((object, buffer)) => {
						let mut descriptor = PullIntoDescriptor {
							buffer: Heap::default(),
							offset,
							length: view.len() * element_size,
							filled: 0,
							element: element_size,
							constructor,
							kind: ReaderKind::Byob,
						};
						descriptor.buffer.set(*object);

						if let Controller::ByteStream(controller) = &mut stream.controller {
							if !controller.pending_descriptors.is_empty() {
								controller.pending_descriptors.push_back(descriptor);
								controller.pending_descriptors[controller.pending_descriptors.len() - 1]
									.buffer
									.set(*object);

								if stream.state == State::Readable {
									self.requests.push_back(Heap::default());
									self.requests[self.requests.len() - 1].set(**request);
								}
								return Ok(request);
							} else if stream.state == State::Closed {
								let empty = unsafe { descriptor.construct(cx)?.as_value(cx) };

								let result = ReadResult { value: Some(**empty), done: true };
								request.resolve(cx, &unsafe { result.as_value(cx) });
								return Ok(request);
							} else if controller.queue_size > 0 {
								if controller.fill_pull_into_descriptor(cx, &mut descriptor)? {
									let (object, _) = transfer_array_buffer(cx, buffer, false)?;
									descriptor.buffer.set(*object);
									let view = unsafe { descriptor.construct(cx)?.as_value(cx) };

									if controller.queue_size == 0 && controller.close_requested {
										controller.close(cx)?;
									} else {
										controller.pull_if_needed(cx)?;
									}

									let result = ReadResult { value: Some(**view), done: false };
									request.resolve(cx, &unsafe { result.as_value(cx) });
									return Ok(request);
								} else if controller.close_requested {
									let error = Error::new("Stream closed by request.", ErrorKind::Type);
									request.reject(cx, &unsafe { error.as_value(cx) });
									return Ok(request);
								}
							}

							controller.pending_descriptors.push_back(descriptor);
							controller.pending_descriptors[controller.pending_descriptors.len() - 1]
								.buffer
								.set(*object);

							if stream.state == State::Readable {
								self.requests.push_back(Heap::default());
								self.requests[self.requests.len() - 1].set(**request);
							}

							controller.pull_if_needed(cx)?;
						}
					}
					Err(error) => {
						request.reject(cx, &unsafe { error.as_value(cx) });
					}
				}

				Ok(request)
			} else {
				let promise = Promise::new(cx);
				promise.reject(cx, &unsafe {
					Error::new("Reader has already been released.", ErrorKind::Type).as_value(cx)
				});
				Ok(promise)
			}
		}

		pub fn releaseLock(&mut self, cx: &Context) -> Result<()> {
			if let Some(stream) = &self.stream {
				let stream = Object::from(unsafe { Local::from_heap(stream) });
				let stream = ReadableStream::get_private(&stream);

				let mut closed = Promise::from(unsafe { Local::from_heap(&self.closed) }).unwrap();
				match stream.state {
					State::Readable => {}
					_ => {
						self.closed = Heap::boxed(**Promise::new(cx));
						closed = Promise::from(unsafe { Local::from_heap(&self.closed) }).unwrap();
					}
				}
				closed.reject(cx, unsafe { &Error::new("Released Reader", ErrorKind::Type).as_value(cx) });

				stream.reader_kind = ReaderKind::None;
				stream.reader_object = None;

				stream.controller.release();

				while let Some(request) = self.requests.pop_front() {
					let request = Promise::from(unsafe { Local::from_heap(&request) }).unwrap();
					request.reject(cx, &unsafe { Error::new("Reader has been released.", ErrorKind::Type).as_value(cx) });
				}
			} else {
				return Err(Error::new("Reader has already been released.", ErrorKind::Type));
			}
			self.stream = None;
			Ok(())
		}

		#[ion(get)]
		pub fn get_closed(&self) -> *mut JSObject {
			self.closed.get()
		}
	}

	unsafe impl Traceable for ByobReader {
		unsafe fn trace(&self, trc: *mut JSTracer) {
			self.stream.trace(trc);
			self.requests.trace(trc);
			self.closed.trace(trc);
		}
	}
}
