/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

pub use controller::{ByobRequest, ByteStreamController, DefaultController};
use ion::conversions::ConversionBehavior;
use ion::Function;
pub use reader::{ByobReader, DefaultReader};
pub use stream::ReadableStream;

mod controller;
mod reader;

#[derive(Default, FromValue)]
pub struct UnderlyingSource<'cx> {
	start: Option<Function<'cx>>,
	pull: Option<Function<'cx>>,
	cancel: Option<Function<'cx>>,
	#[ion(name = "type")]
	ty: Option<String>,
	#[ion(convert = ConversionBehavior::EnforceRange)]
	auto_allocate_chunk_size: Option<u64>,
}

#[derive(Default, FromValue)]
pub struct QueueingStrategy<'cx> {
	high_water_mark: Option<f64>,
	size: Option<Function<'cx>>,
}

#[derive(Default, FromValue)]
pub struct ReaderOptions {
	mode: Option<String>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum State {
	Readable,
	Closed,
	Errored,
}

#[js_class]
mod stream {
	use std::ffi::c_void;
	use std::mem::transmute;

	use mozjs::gc::Traceable;
	use mozjs::jsapi::{Heap, JS_SetReservedSlot, JSObject, JSTracer};
	use mozjs::jsval::{JSVal, PrivateValue};

	use ion::{ClassInitialiser, Context, Error, ErrorKind, Local, Object, Promise, Result, ResultExc, Value};
	use ion::conversions::{FromValue, ToValue};

	use crate::globals::streams::readable::{QueueingStrategy, ReaderOptions, State, UnderlyingSource};
	use crate::globals::streams::readable::controller::{ByteStreamController, Controller, DefaultController};
	use crate::globals::streams::readable::reader::{ByobReader, DefaultReader, Reader, ReaderKind, ReadResult};

	pub struct ReadableStream {
		pub(crate) controller: Controller,
		pub(crate) controller_object: Box<Heap<*mut JSObject>>,

		pub(crate) reader_kind: ReaderKind,
		pub(crate) reader_object: Option<Box<Heap<*mut JSObject>>>,

		pub(crate) state: State,
		pub(crate) disturbed: bool,
		pub(crate) error: Option<Box<Heap<JSVal>>>,
	}

	impl ReadableStream {
		#[ion(constructor)]
		pub fn constructor<'cx: 'o, 'o>(
			cx: &'cx Context, #[ion(this)] this: &Object, underlying_source: Option<Object<'o>>, strategy: Option<QueueingStrategy>,
		) -> ResultExc<()> {
			let strategy = strategy.unwrap_or_default();
			let mut source = None;

			let stream = underlying_source
				.as_ref()
				.map(|underlying_source| {
					let mut source_value = Value::null(cx);
					unsafe {
						underlying_source.to_value(cx, &mut source_value);
					}
					source = unsafe { Some(UnderlyingSource::from_value(cx, &source_value, false, ())?) };
					let source = source.as_ref().unwrap();
					if source.ty.as_deref() == Some("bytes") {
						if strategy.size.is_some() {
							return Err(Error::new("Implementation preserved member 'size'", ErrorKind::Range));
						}
						if let Some(high_water_mark) = strategy.high_water_mark {
							if high_water_mark.is_nan() {
								return Err(Error::new("highWaterMark cannot be NaN", ErrorKind::Range));
							} else if high_water_mark < 0.0 {
								return Err(Error::new("highWaterMark must be non-negative", ErrorKind::Range));
							}
						}
						let high_water_mark = strategy.high_water_mark.unwrap_or(0.0);

						let (heap, controller) = ByteStreamController::initialise(cx, this, &underlying_source, source, high_water_mark)?;

						Ok(Some(ReadableStream {
							controller: Controller::ByteStream(controller),
							controller_object: heap,

							reader_kind: ReaderKind::None,
							reader_object: None,

							state: State::Readable,
							disturbed: false,
							error: None,
						}))
					} else if source.ty.is_some() {
						Err(Error::new("Type of Underlying Source must be 'bytes' or not exist.", ErrorKind::Type))
					} else {
						Ok(None)
					}
				})
				.transpose()?
				.flatten();
			let stream = stream.unwrap_or_else(|| {
				let source = source.unwrap_or_default();
				let high_water_mark = strategy.high_water_mark.unwrap_or(1.0);
				let (heap, controller) = DefaultController::initialise(cx, this, underlying_source.as_ref(), &source, &strategy, high_water_mark);

				ReadableStream {
					controller: Controller::Default(controller),
					controller_object: heap,

					reader_kind: ReaderKind::None,
					reader_object: None,

					state: State::Readable,
					disturbed: false,
					error: None,
				}
			});

			let b = Box::new(Some(stream));
			unsafe {
				JS_SetReservedSlot(
					***this,
					ReadableStream::PARENT_PROTOTYPE_CHAIN_LENGTH,
					&PrivateValue(Box::into_raw(b) as *mut c_void),
				)
			};
			let stream = ReadableStream::get_private(this);
			stream.controller.start(cx, &stream.controller_object);
			Ok(())
		}

		pub fn cancel<'cx: 'v, 'v>(&mut self, cx: &'cx Context, reason: Option<Value<'v>>) -> ResultExc<Promise<'cx>> {
			if self.get_locked() {
				Err(Error::new("ReadableStream is locked.", ErrorKind::Type).into())
			} else {
				self.disturbed = true;
				match self.state {
					State::Readable => {
						self.close(cx)?;
						self.controller.cancel(cx, reason, &self.controller_object)
					}
					State::Closed => {
						let promise = Promise::new(cx);
						promise.resolve(cx, &Value::undefined(cx));
						Ok(promise)
					}
					State::Errored => {
						let mut value = Value::null(cx);
						if let Some(error) = &self.error {
							value.handle_mut().set(error.get());
						}
						let promise = Promise::new(cx);
						promise.reject(cx, &value);
						Ok(promise)
					}
				}
			}
		}

		pub fn getReader<'cx>(&mut self, #[ion(this)] this: &Object, cx: &'cx Context, options: Option<ReaderOptions>) -> Result<Object<'cx>> {
			let options = options.unwrap_or_default();
			if let Some(mode) = &options.mode {
				if mode == "byob" {
					let reader = ByobReader::new(cx, &Object::from(cx.root_object(***this)))?;
					let object = Object::from(cx.root_object(ByobReader::new_object(cx, reader)));

					self.reader_kind = ReaderKind::Byob;
					self.reader_object = Some(Heap::boxed(**object));

					Ok(object)
				} else {
					Err(Error::new("Mode must be 'byob' or must not exist.", ErrorKind::Type))
				}
			} else {
				if self.get_locked() {
					return Err(Error::new("New readers cannot be initialised for locked streams.", ErrorKind::Type));
				}

				let reader = DefaultReader::new(cx, &Object::from(cx.root_object(***this)))?;
				let object = Object::from(cx.root_object(DefaultReader::new_object(cx, reader)));

				self.reader_kind = ReaderKind::Default;
				self.reader_object = Some(Heap::boxed(**object));

				Ok(object)
			}
		}

		#[ion(get)]
		pub fn get_locked(&self) -> bool {
			self.reader_kind != ReaderKind::None
		}

		pub(crate) fn close(&mut self, cx: &Context) -> Result<()> {
			if self.state != State::Readable {
				return Err(Error::new("Cannot Close Stream", None));
			}

			self.state = State::Closed;
			let (requests, closed) = match self.get_reader() {
				Some(Reader::Default(reader)) => (&mut reader.requests, &reader.closed),
				Some(Reader::Byob(reader)) => (&mut reader.requests, &reader.closed),
				None => return Ok(()),
			};

			let closed = Promise::from(unsafe { Local::from_raw_handle(closed.handle()) }).unwrap();
			closed.resolve(cx, &Value::undefined(cx));

			for request in &*requests {
				let request = Promise::from(unsafe { Local::from_raw_handle(request.handle()) }).unwrap();
				let req = ReadResult { value: None, done: true };
				request.resolve(cx, unsafe { &req.as_value(cx) });
			}
			requests.clear();

			Ok(())
		}

		pub(crate) fn error(&mut self, cx: &Context, error: &Value) -> Result<()> {
			if self.state != State::Readable {
				return Err(Error::new("Cannot Error Stream", None));
			}
			self.state = State::Errored;
			self.error = Some(Heap::boxed(***error));
			let (requests, closed) = match self.get_reader() {
				Some(Reader::Default(reader)) => (&mut reader.requests, &reader.closed),
				Some(Reader::Byob(reader)) => (&mut reader.requests, &reader.closed),
				None => return Ok(()),
			};

			let closed = Promise::from(unsafe { Local::from_raw_handle(closed.handle()) }).unwrap();
			closed.reject(cx, error);
			for request in &*requests {
				let request = Promise::from(unsafe { Local::from_raw_handle(request.handle()) }).unwrap();
				request.reject(cx, &error);
			}
			requests.clear();

			Ok(())
		}

		pub(crate) fn get_reader(&self) -> Option<Reader> {
			match self.reader_kind {
				ReaderKind::None => None,
				ReaderKind::Default => {
					let reader = Object::from(unsafe { Local::from_raw_handle(self.reader_object.as_ref().unwrap().handle()) });
					let reader = unsafe { transmute(DefaultReader::get_private(&reader)) };
					Some(Reader::Default(reader))
				}
				ReaderKind::Byob => {
					let reader = Object::from(unsafe { Local::from_raw_handle(self.reader_object.as_ref().unwrap().handle()) });
					let reader = unsafe { transmute(ByobReader::get_private(&reader)) };
					Some(Reader::Byob(reader))
				}
			}
		}
	}

	unsafe impl Traceable for ReadableStream {
		unsafe fn trace(&self, trc: *mut JSTracer) {
			self.controller_object.trace(trc);
			self.reader_object.trace(trc);
			self.error.trace(trc);
		}
	}
}
