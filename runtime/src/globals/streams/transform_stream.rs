use ion::{
	class::Reflector,
	Heap, js_class, Result, Error, ErrorKind, Context, Object, Value, Function, ClassDefinition, ResultExc, Exception,
	conversions::{FromValue, ToValue},
	Promise, PromiseFuture, TracedHeap,
	flags::PropertyFlags,
};
use mozjs::{
	jsapi::{
		JSObject, JSFunction, ReadableStreamGetController, WritableStreamGetController,
		CheckReadableStreamControllerCanCloseOrEnqueue, ReadableStreamEnqueue, JS_GetPendingException,
		JS_ClearPendingException, ReadableStreamGetStoredError, ReadableStreamClose, JS_ReportErrorLatin1,
		ReadableStreamError, WritableStreamGetState, WritableStreamState, WritableStreamError,
		ReadableStreamGetDesiredSize, NewWritableDefaultStreamObject, HandleObject, HandleFunction,
		ReadableStreamIsErrored, NewReadableDefaultStreamObject,
	},
	c_str,
	jsval::JSVal,
};

use crate::{globals::streams::native_stream_sink::NativeStreamSink, promise::future_to_promise};

use super::{native_stream_sink::NativeStreamSinkCallbacks, NativeStreamSourceCallbacks, NativeStreamSource};

// TODO: back-pressure

#[derive(FromValue)]
pub struct Transformer<'cx> {
	start: Option<Function<'cx>>,
	transform: Option<Function<'cx>>,
	flush: Option<Function<'cx>>,
	cancel: Option<Function<'cx>>,
}

// Needed to store the transformer instance and callbacks for use in the controller.
#[derive(Traceable)]
pub enum HeapTransformer {
	Null,
	Object {
		instance: Heap<*mut JSObject>,
		start: Option<Heap<*mut JSFunction>>,
		transform: Option<Heap<*mut JSFunction>>,
		flush: Option<Heap<*mut JSFunction>>,
		cancel: Option<Heap<*mut JSFunction>>,
	},
}

impl HeapTransformer {
	fn from_transformer(cx: &Context, transformer_object: Option<Object>) -> Result<Self> {
		match transformer_object {
			Some(transformer_object) => {
				let transformer = Transformer::from_value(cx, &transformer_object.as_value(cx), false, ())?;
				Ok(Self::Object {
					instance: Heap::from_local(&transformer_object),
					start: transformer.start.map(|f| Heap::from_local(&f)),
					transform: transformer.transform.map(|f| Heap::from_local(&f)),
					flush: transformer.flush.map(|f| Heap::from_local(&f)),
					cancel: transformer.cancel.map(|f| Heap::from_local(&f)),
				})
			}

			None => Ok(Self::Null),
		}
	}

	fn start_function<'cx>(&self, cx: &'cx Context) -> Option<(Object<'cx>, Function<'cx>)> {
		match self {
			Self::Null | Self::Object { start: None, .. } => None,
			Self::Object { instance, start: Some(start), .. } => {
				Some((instance.root(cx).into(), start.root(cx).into()))
			}
		}
	}

	fn transform_function<'cx>(&self, cx: &'cx Context) -> Option<(Object<'cx>, Function<'cx>)> {
		match self {
			Self::Null | Self::Object { transform: None, .. } => None,
			Self::Object { instance, transform: Some(transform), .. } => {
				Some((instance.root(cx).into(), transform.root(cx).into()))
			}
		}
	}

	fn flush_function<'cx>(&self, cx: &'cx Context) -> Option<(Object<'cx>, Function<'cx>)> {
		match self {
			Self::Null | Self::Object { flush: None, .. } => None,
			Self::Object { instance, flush: Some(flush), .. } => {
				Some((instance.root(cx).into(), flush.root(cx).into()))
			}
		}
	}

	fn cancel_function<'cx>(&self, cx: &'cx Context) -> Option<(Object<'cx>, Function<'cx>)> {
		match self {
			Self::Null | Self::Object { cancel: None, .. } => None,
			Self::Object { instance, cancel: Some(cancel), .. } => {
				Some((instance.root(cx).into(), cancel.root(cx).into()))
			}
		}
	}
}

#[js_class]
pub struct TransformStreamDefaultController {
	reflector: Reflector,
	stream: Heap<*mut JSObject>,
	transformer: HeapTransformer,
}

impl TransformStreamDefaultController {
	pub fn from_heap<'cx>(cx: &'cx Context, heap: &Heap<*mut JSObject>) -> &'cx Self {
		<Self as ClassDefinition>::get_private(&heap.root(cx).into())
	}

	pub fn from_heap_mut<'cx>(cx: &'cx Context, heap: &Heap<*mut JSObject>) -> &'cx mut Self {
		<Self as ClassDefinition>::get_mut_private(&mut heap.root(cx).into())
	}

	fn new(stream: &Object, transformer: HeapTransformer) -> Self {
		Self {
			reflector: Default::default(),
			stream: Heap::from_local(stream),
			transformer,
		}
	}
}

#[js_class]
impl TransformStreamDefaultController {
	#[ion(constructor)]
	pub fn constructor() -> Result<TransformStreamDefaultController> {
		Err(Error::new("Cannot construct this object", ErrorKind::Syntax))
	}

	#[ion(get, name = "desiredSize")]
	pub fn get_desired_size(&self, cx: &Context) -> Result<JSVal> {
		let readable = TransformStream::from_heap(cx, &self.stream).readable.root(cx);
		let mut value = 0.0;
		let mut has_value = false;
		if !unsafe {
			ReadableStreamGetDesiredSize(
				cx.as_ptr(),
				readable.get(),
				&mut has_value as *mut _,
				&mut value as *mut _,
			)
		} {
			return Err(Error::none());
		}

		if has_value {
			Ok(Value::f64(cx, value).get())
		} else {
			Ok(Value::null(cx).get())
		}
	}

	pub fn enqueue(&self, cx: &Context, chunk: Value) -> ResultExc<()> {
		let stream = TransformStream::from_heap(cx, &self.stream);
		let readable = stream.readable.root(cx);
		let controller = stream.get_readable_controller(cx);
		unsafe {
			if !CheckReadableStreamControllerCanCloseOrEnqueue(
				cx.as_ptr(),
				controller.handle().into(),
				c_str!("enqueue"),
			) {
				return Err(Exception::Error(Error::new(
					"Readable stream is already closed",
					ErrorKind::Type,
				)));
			}

			if !ReadableStreamEnqueue(cx.as_ptr(), readable.handle().into(), chunk.handle().into()) {
				let mut error_value = Value::undefined(cx);
				if !JS_GetPendingException(cx.as_ptr(), error_value.handle_mut().into()) {
					return Err(Exception::Error(Error::none()));
				}
				JS_ClearPendingException(cx.as_ptr());

				stream.error_writable_and_unblock_write(cx, &error_value)?;

				let stored_error = ReadableStreamGetStoredError(cx.as_ptr(), readable.handle().into());
				return Err(Exception::Other(stored_error));
			}
		}

		Ok(())
	}

	pub fn error(&self, cx: &Context, e: Value) -> Result<()> {
		TransformStream::from_heap(cx, &self.stream).error(cx, &e)
	}

	pub fn terminate(&self, cx: &Context) -> Result<()> {
		let stream = TransformStream::from_heap(cx, &self.stream);
		let readable = stream.readable.root(cx);
		let controller = stream.get_readable_controller(cx);

		unsafe {
			if CheckReadableStreamControllerCanCloseOrEnqueue(cx.as_ptr(), controller.handle().into(), c_str!("close"))
			{
				if !ReadableStreamClose(cx.as_ptr(), readable.handle().into()) {
					return Err(Error::none());
				}
			} else {
				JS_ClearPendingException(cx.as_ptr());
			}

			let mut error = Value::undefined(cx);
			JS_ReportErrorLatin1(cx.as_ptr(), c_str!("TransformStream was terminated"));
			if !JS_GetPendingException(cx.as_ptr(), error.handle_mut().into()) {
				return Err(Error::none());
			}
			JS_ClearPendingException(cx.as_ptr());

			stream.error_writable_and_unblock_write(cx, &error)?;
		}

		Ok(())
	}

	pub fn clear_algorithms(&mut self) {
		match self.transformer {
			HeapTransformer::Object { ref mut transform, ref mut flush, .. } => {
				*transform = None;
				*flush = None;
			}
			HeapTransformer::Null => (),
		}
	}
}

struct Source {
	stream: Heap<*mut JSObject>,
	start_promise: Promise,
}

impl NativeStreamSourceCallbacks for Source {
	fn start<'cx>(&self, cx: &'cx Context, _controller: Object<'cx>) -> ResultExc<Value<'cx>> {
		let mut res = Value::undefined(cx);
		self.start_promise.to_value(cx, &mut res);
		Ok(res)
	}

	fn pull<'cx>(&self, cx: &'cx Context, _controller: Object<'cx>) -> ResultExc<Promise> {
		// todo: implement backpressure
		Ok(Promise::new_resolved(cx, Value::undefined(cx)))
	}

	fn cancel<'cx>(&self, cx: &'cx Context, reason: Value) -> ResultExc<Promise> {
		let ts = TransformStream::from_heap(cx, &self.stream);
		ts.error_writable_and_unblock_write(cx, &reason)?;
		Ok(Promise::new_resolved(cx, Value::undefined(cx)))
	}
}

struct Sink {
	// Note that this is not traced, since the sink can only exist if the owning stream is still alive
	stream: Heap<*mut JSObject>,
	start_promise: Promise,
}

impl NativeStreamSinkCallbacks for Sink {
	fn start<'cx>(&self, cx: &'cx Context, _controller: Object<'cx>) -> ResultExc<Value<'cx>> {
		let mut res = Value::undefined(cx);
		self.start_promise.to_value(cx, &mut res);
		Ok(res)
	}

	fn write<'cx>(&self, cx: &'cx Context, chunk: Value, _controller: Object) -> ResultExc<Promise> {
		let ts = TransformStream::from_heap(cx, &self.stream);
		let writable = ts.writable.root(cx);

		if unsafe { WritableStreamGetState(cx.as_ptr(), writable.handle().into()) } != WritableStreamState::Writable {
			return Err(Exception::Error(Error::new(
				"Writable half of TransformStream must be in writable state",
				ErrorKind::Normal,
			)));
		}

		let controller = ts.get_controller(cx);
		let controller_object = Object::from(ts.controller.root(cx)).as_value(cx);

		let promise = match controller.transformer.transform_function(cx) {
			None => {
				controller.enqueue(cx, chunk)?;
				Promise::new_resolved(cx, Value::undefined(cx))
			}

			Some((o, f)) => match f.call(&cx, &o, &[chunk, controller_object]) {
				Err(e) => Promise::new_rejected(
					cx,
					e.map(|e| e.exception).unwrap_or_else(|| {
						Exception::Error(Error::new("Call to transformer.transform failed", ErrorKind::Normal))
					}),
				),
				Ok(val) => {
					if !val.get().is_object() {
						Promise::new_resolved(cx, Value::undefined(cx))
					} else {
						match Promise::from(val.to_object(&cx).into_local()) {
							// The flush algorithm (erroneously) didn't return a promise
							None => Promise::new_resolved(cx, Value::undefined(cx)),
							Some(p) => p,
						}
					}
				}
			},
		};

		let ts_heap = self.stream.clone();

		promise.add_reactions(
			cx,
			None,
			Some(Function::from_closure(
				cx,
				"__TransformStreamSinkWriteCallbackFailed",
				Box::new(move |args| {
					let mut accessor = args.access();
					let reason = accessor.arg::<Value>(false, ()).ok_or_else(|| {
						Exception::Error(Error::new("Bad arguments to promise.reject", ErrorKind::Internal))
					})??;

					let ts = TransformStream::from_heap(args.cx(), &ts_heap);
					ts.error(args.cx(), &reason)?;

					Err(Exception::Other(reason.get()))
				}),
				1,
				PropertyFlags::empty(),
			)),
		);

		Ok(promise)
	}

	fn close<'cx>(&self, cx: &'cx Context) -> ResultExc<Promise> {
		let stream = self.stream.clone();

		unsafe {
			Ok(future_to_promise(cx, move |cx| async move {
				let ts = TransformStream::from_heap(&cx, &stream);
				let controller_object = Object::from(ts.controller.root(&cx));
				let controller = ts.get_controller(&cx);
				let cx = match controller.transformer.flush_function(&cx) {
					// No flush algorithm, carry on.
					None => cx,

					// Run the flush algorithm
					Some((o, f)) => {
						let flush_result = match f.call(&cx, &o, &[controller_object.as_value(&cx)]) {
							Err(_) => return Err(Exception::Error(Error::none())),
							Ok(f) => f,
						};

						if !flush_result.get().is_object() {
							// The flush algorithm (erroneously) didn't return an object
							cx
						} else {
							match Promise::from(flush_result.to_object(&cx).into_local()) {
								// The flush algorithm (erroneously) didn't return a promise
								None => cx,
								Some(p) => {
									// Wait for the promise to run...
									let (cx, promise_result) = PromiseFuture::new(cx, &p).await;
									match promise_result {
										Err(e) => {
											// ... if it failed, fail the entire process
											let ts = TransformStream::from_heap(&cx, &stream);
											ts.error(&cx, &cx.root_value(e).into())?;

											let readable = ts.readable.root(&cx);
											let error =
												ReadableStreamGetStoredError(cx.as_ptr(), readable.handle().into());

											return Err(Exception::Other(error));
										}
										// ... it ran successfully, carry on
										Ok(_) => cx,
									}
								}
							}
						}
					}
				};

				let ts = TransformStream::from_heap(&cx, &stream);

				let controller = ts.get_controller_mut(&cx);
				controller.clear_algorithms();

				let readable = ts.readable.root(&cx);
				let mut readable_errored = false;
				if !ReadableStreamIsErrored(cx.as_ptr(), readable.handle().into(), &mut readable_errored) {
					return Err(Exception::Error(Error::none()));
				}

				if readable_errored {
					let e = ReadableStreamGetStoredError(cx.as_ptr(), readable.handle().into());
					return Err(Exception::Other(e));
				}

				if CheckReadableStreamControllerCanCloseOrEnqueue(
					cx.as_ptr(),
					ts.get_readable_controller(&cx).handle().into(),
					c_str!("close"),
				) {
					if !ReadableStreamClose(cx.as_ptr(), readable.handle().into()) {
						return Err(Exception::Error(Error::none()));
					}
				} else {
					JS_ClearPendingException(cx.as_ptr());
				}

				Ok(())
			})
			.expect("future queue must be initialized"))
		}
	}

	fn abort<'cx>(&self, cx: &'cx Context, reason: Value) -> ResultExc<Promise> {
		let ts = TransformStream::from_heap(cx, &self.stream);
		if let Err(e) = ts.error(cx, &reason) {
			return Ok(Promise::new_rejected(cx, e));
		}

		match ts.get_controller(cx).transformer.cancel_function(cx) {
			None => Ok(Promise::new_resolved(cx, Value::undefined(cx))),
			Some((o, f)) => {
				let result = f.call(cx, &o, &[reason]);
				match result {
					Err(_) => Ok(Promise::new_rejected_with_pending_exception(cx)),
					Ok(v) if v.get().is_object() => match Promise::from(v.to_object(cx).into_local()) {
						Some(p) => Ok(p),
						None => Ok(Promise::new_resolved(cx, Value::undefined(cx))),
					},
					Ok(_) => Ok(Promise::new_resolved(cx, Value::undefined(cx))),
				}
			}
		}
	}
}

#[js_class]
pub struct TransformStream {
	reflector: Reflector,

	controller: Heap<*mut JSObject>,

	readable: Heap<*mut JSObject>,
	writable: Heap<*mut JSObject>,
}

impl TransformStream {
	pub fn from_heap<'cx>(cx: &'cx Context, heap: &Heap<*mut JSObject>) -> &'cx Self {
		<Self as ClassDefinition>::get_private(&heap.root(cx).into())
	}

	pub fn get_controller<'cx>(&self, cx: &'cx Context) -> &'cx TransformStreamDefaultController {
		TransformStreamDefaultController::from_heap(cx, &self.controller)
	}

	pub fn get_controller_mut<'cx>(&self, cx: &'cx Context) -> &'cx mut TransformStreamDefaultController {
		TransformStreamDefaultController::from_heap_mut(cx, &self.controller)
	}

	pub fn get_controller_object<'cx>(&self, cx: &'cx Context) -> Object<'cx> {
		self.controller.root(cx).into()
	}

	pub fn get_readable_controller<'cx>(&self, cx: &'cx Context) -> Object<'cx> {
		// TODO: Implement ion wrapper type for the controller
		Object::from(
			cx.root_object(unsafe { ReadableStreamGetController(cx.as_ptr(), self.readable.root(cx).handle().into()) }),
		)
	}

	pub fn get_writable_controller<'cx>(&self, cx: &'cx Context) -> Object<'cx> {
		Object::from(
			cx.root_object(unsafe { WritableStreamGetController(cx.as_ptr(), self.writable.root(cx).handle().into()) }),
		)
	}

	pub fn error(&self, cx: &Context, e: &Value) -> Result<()> {
		unsafe {
			if !ReadableStreamError(cx.as_ptr(), self.readable.root(cx).handle().into(), e.handle().into()) {
				return Err(Error::none());
			}
		}

		self.error_writable_and_unblock_write(cx, e)
	}

	pub fn error_writable_and_unblock_write(&self, cx: &Context, e: &Value) -> Result<()> {
		TransformStreamDefaultController::from_heap_mut(cx, &self.controller).clear_algorithms();

		let writable = self.writable.root(cx);

		unsafe {
			if WritableStreamGetState(cx.as_ptr(), writable.handle().into()) == WritableStreamState::Writable {
				if !WritableStreamError(cx.as_ptr(), writable.handle().into(), e.handle().into()) {
					return Err(Error::none());
				}
			}
		}

		Ok(())
	}
}

const NULL_FUNCTION: *mut JSFunction = 0 as *mut JSFunction;

#[js_class]
impl TransformStream {
	#[ion(constructor)]
	pub fn constructor<'cx>(
		cx: &'cx Context, #[ion(this)] this: &Object<'cx>, transformer_object: Option<Object<'cx>>,
	) -> Result<TransformStream> {
		let transformer = HeapTransformer::from_transformer(cx, transformer_object)?;

		let controller =
			ClassDefinition::new_object(cx, Box::new(TransformStreamDefaultController::new(this, transformer)));

		// For use in the start promise, below
		let controller_heap = TracedHeap::new(controller);

		// We need to turn this into a promise so it gets run later as part of the event loop, once
		// construction of the TransformStream is complete. Otherwise, if the transformer.start_function
		// accesses this stream, the private slot will not be set and everything will fail.
		let start_promise = unsafe {
			future_to_promise(cx, move |cx| async move {
				let controller = TransformStreamDefaultController::get_private(&controller_heap.root(&cx).into());
				let controller_value = Object::from(controller_heap.root(&cx)).as_value(&cx);
				match controller.transformer.start_function(&cx) {
					Some((o, f)) => match f.call(&cx, &o, &[controller_value]) {
						Ok(val) => Ok(val.get()),
						Err(_) => Err(Exception::Error(Error::none())),
					},
					None => Ok(mozjs::jsval::UndefinedValue()),
				}
			})
			.expect("future queue must be initialized")
		};

		let sink = Sink {
			stream: Heap::from_local(&this),
			start_promise: unsafe { Promise::from_unchecked(start_promise.root(cx)) },
		};
		let sink_obj = cx.root_object(NativeStreamSink::new_object(
			cx,
			Box::new(NativeStreamSink::new(Box::new(sink))),
		));

		let writable = unsafe {
			cx.root_object(NewWritableDefaultStreamObject(
				cx.as_ptr(),
				sink_obj.handle().into(),
				HandleFunction::from_marked_location(&NULL_FUNCTION),
				1.0,
				HandleObject::null(),
			))
		};

		if writable.get().is_null() {
			return Err(Error::new(
				"Failed to create writable half of stream",
				ErrorKind::Normal,
			));
		}

		let source = Source {
			stream: Heap::from_local(&this),
			start_promise,
		};
		let source_obj = cx.root_object(NativeStreamSource::new_object(
			cx,
			Box::new(NativeStreamSource::new(Box::new(source))),
		));

		let readable = unsafe {
			cx.root_object(NewReadableDefaultStreamObject(
				cx.as_ptr(),
				source_obj.handle().into(),
				HandleFunction::from_marked_location(&NULL_FUNCTION),
				1.0,
				HandleObject::null(),
			))
		};

		if readable.get().is_null() {
			return Err(Error::new(
				"Failed to create readable half of stream",
				ErrorKind::Normal,
			));
		}

		Ok(Self {
			reflector: Default::default(),
			controller: Heap::new(controller),
			readable: Heap::from_local(&readable),
			writable: Heap::from_local(&writable),
		})
	}

	#[ion(get)]
	pub fn get_readable(&self) -> *mut JSObject {
		self.readable.get()
	}

	#[ion(get)]
	pub fn get_writable(&self) -> *mut JSObject {
		self.writable.get()
	}
}
