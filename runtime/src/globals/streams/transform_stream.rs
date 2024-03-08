use ion::{
	class::{NativeObject, Reflector},
	conversions::{FromValue, ToValue},
	flags::PropertyFlags,
	function::Opt,
	js_class, ClassDefinition, Context, Error, ErrorKind, Exception, Function, Heap, Object, Promise, PromiseFuture,
	Result, ResultExc, TracedHeap, Value,
};
use mozjs::{
	jsapi::{
		JSObject, JSFunction, ReadableStreamGetController, WritableStreamGetController,
		CheckReadableStreamControllerCanCloseOrEnqueue, ReadableStreamEnqueue, JS_GetPendingException,
		JS_ClearPendingException, ReadableStreamGetStoredError, ReadableStreamClose, JS_ReportErrorLatin1,
		ReadableStreamError, WritableStreamGetState, WritableStreamState, WritableStreamError,
		ReadableStreamGetDesiredSize, NewWritableDefaultStreamObject, HandleObject, HandleFunction,
		ReadableStreamIsErrored,
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
		<Self as ClassDefinition>::get_private(cx, &heap.root(cx).into()).unwrap()
	}

	pub fn from_heap_mut<'cx>(cx: &'cx Context, heap: &Heap<*mut JSObject>) -> &'cx mut Self {
		<Self as ClassDefinition>::get_mut_private(cx, &heap.root(cx).into()).unwrap()
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
		let stream = TransformStream::from_heap_mut(cx, &self.stream);
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

	pub fn error(&self, cx: &Context, Opt(e): Opt<Value>) -> Result<()> {
		let e = e.unwrap_or_else(|| Value::undefined(cx));
		TransformStream::from_heap_mut(cx, &self.stream).error(cx, &e)
	}

	pub fn terminate(&self, cx: &Context) -> Result<()> {
		let stream = TransformStream::from_heap_mut(cx, &self.stream);
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
	stream: TracedHeap<*mut JSObject>,
	start_promise: Promise,
}

impl NativeStreamSourceCallbacks for Source {
	fn start<'cx>(
		&self, _source: &'cx NativeStreamSource, cx: &'cx Context, _controller: Object<'cx>,
	) -> ResultExc<Value<'cx>> {
		let mut res = Value::undefined(cx);
		self.start_promise.to_value(cx, &mut res);
		Ok(res)
	}

	fn pull<'cx>(
		&self, _source: &'cx NativeStreamSource, cx: &'cx Context, _controller: Object<'cx>,
	) -> ResultExc<Promise> {
		// todo: implement backpressure
		Ok(Promise::resolved(cx, Value::undefined(cx)))
	}

	fn cancel(self: Box<Self>, cx: &Context, reason: Value) -> ResultExc<Promise> {
		let ts = TransformStream::from_traced_heap_mut(cx, &self.stream);

		let reason_heap = TracedHeap::from_local(&reason);
		let ts_heap1 = self.stream.clone();
		let ts_heap2 = self.stream.clone();

		let (finish_promise, just_created) = ts.get_or_create_finish_promise(cx, reason);

		if just_created {
			finish_promise.add_reactions(
				cx,
				Some(Function::from_closure(
					cx,
					"",
					Box::new(move |args| {
						let cx = args.cx();
						let ts = TransformStream::from_traced_heap_mut(cx, &ts_heap1);
						_ = ts.error_writable_and_unblock_write(cx, &reason_heap.root(cx).into());
						Ok(Value::undefined(cx))
					}),
					1,
					PropertyFlags::empty(),
				)),
				Some(Function::from_closure(
					cx,
					"",
					Box::new(move |args| {
						let cx = args.cx();
						let error = args.access().value();
						let ts = TransformStream::from_traced_heap_mut(cx, &ts_heap2);
						_ = ts.error_writable_and_unblock_write(cx, &error);
						Ok(Value::undefined(cx))
					}),
					1,
					PropertyFlags::empty(),
				)),
			);
		}

		Ok(finish_promise)
	}
}

struct Sink {
	stream: TracedHeap<*mut JSObject>,
	start_promise: Promise,
}

impl NativeStreamSinkCallbacks for Sink {
	fn start<'cx>(&self, cx: &'cx Context, _controller: Object<'cx>) -> ResultExc<Value<'cx>> {
		let mut res = Value::undefined(cx);
		self.start_promise.to_value(cx, &mut res);
		Ok(res)
	}

	fn write(&self, cx: &Context, chunk: Value, _controller: Object) -> ResultExc<Promise> {
		let ts = TransformStream::from_traced_heap(cx, &self.stream);
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
				Promise::resolved(cx, Value::undefined(cx))
			}

			Some((o, f)) => match f.call(cx, &o, &[chunk, controller_object]) {
				Err(e) => Promise::rejected(
					cx,
					e.map(|e| e.exception).unwrap_or_else(|| {
						Exception::Error(Error::new("Call to transformer.transform failed", ErrorKind::Normal))
					}),
				),
				Ok(val) => {
					if !val.get().is_object() {
						Promise::resolved(cx, Value::undefined(cx))
					} else {
						match Promise::from(val.to_object(cx).into_local()) {
							// The flush algorithm (erroneously) didn't return a promise
							None => Promise::resolved(cx, Value::undefined(cx)),
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
					if accessor.is_empty() {
						return Err(Exception::Error(Error::new(
							"Bad arguments to promise.reject",
							ErrorKind::Internal,
						)));
					}

					let reason = accessor.value();

					let ts = TransformStream::from_traced_heap_mut(args.cx(), &ts_heap);
					ts.error(args.cx(), &reason)?;

					Err(Exception::Other(reason.get()))
				}),
				1,
				PropertyFlags::empty(),
			)),
		);

		Ok(promise)
	}

	fn close(&self, cx: &Context) -> ResultExc<Promise> {
		let ts = TransformStream::from_traced_heap_mut(cx, &self.stream);
		if let Some(ref p) = ts.finish_promise {
			return Ok(p.clone());
		}

		let stream = self.stream.clone();

		let promise = unsafe {
			future_to_promise(cx, move |cx| async move {
				let ts = TransformStream::from_traced_heap(&cx, &stream);
				let controller_object = Object::from(ts.controller.root(&cx));
				let controller = ts.get_controller(&cx);
				let cx = match controller.transformer.flush_function(&cx) {
					// No flush algorithm, carry on.
					None => cx,

					// Run the flush algorithm
					Some((o, f)) => {
						let flush_result = match f.call(&cx, &o, &[controller_object.as_value(&cx)]) {
							Err(Some(e)) => {
								ReadableStreamError(
									cx.as_ptr(),
									ts.readable.root(&cx).handle().into(),
									e.exception.as_value(&cx).handle().into(),
								);
								return Err(e.exception);
							}
							Err(None) => return Err(Error::none().into()),
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
											let e = e.root(&cx).into();

											// ... if it failed, fail the entire process
											let ts = TransformStream::from_traced_heap_mut(&cx, &stream);
											ts.error(&cx, &e)?;

											ReadableStreamError(
												cx.as_ptr(),
												ts.readable.root(&cx).handle().into(),
												e.handle().into(),
											);

											return Err(Exception::Other(e.get()));
										}
										// ... it ran successfully, carry on
										Ok(_) => cx,
									}
								}
							}
						}
					}
				};

				let ts = TransformStream::from_traced_heap(&cx, &stream);

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
			.expect("future queue must be initialized")
		};
		ts.finish_promise = Some(promise.clone());
		Ok(promise)
	}

	fn abort(&self, cx: &Context, reason: Value) -> ResultExc<Promise> {
		let ts = TransformStream::from_traced_heap_mut(cx, &self.stream);

		let reason_heap = TracedHeap::from_local(&reason);
		let ts_heap1 = self.stream.clone();
		let ts_heap2 = self.stream.clone();

		let (finish_promise, just_created) = ts.get_or_create_finish_promise(cx, reason);

		if just_created {
			finish_promise.add_reactions(
				cx,
				Some(Function::from_closure(
					cx,
					"",
					Box::new(move |args| {
						let cx = args.cx();
						let ts = TransformStream::from_traced_heap_mut(cx, &ts_heap1);
						_ = ts.error(cx, &reason_heap.root(cx).into());
						Ok(Value::undefined(cx))
					}),
					1,
					PropertyFlags::empty(),
				)),
				Some(Function::from_closure(
					cx,
					"",
					Box::new(move |args| {
						let cx = args.cx();
						let error = args.access().value();
						let ts = TransformStream::from_traced_heap_mut(cx, &ts_heap2);
						_ = ts.error(cx, &error);
						Ok(Value::undefined(cx))
					}),
					1,
					PropertyFlags::empty(),
				)),
			);
		}

		Ok(finish_promise)
	}
}

#[js_class]
pub struct TransformStream {
	reflector: Reflector,

	#[trace(no_trace)]
	start_promise: Promise,

	controller: Heap<*mut JSObject>,

	readable: Heap<*mut JSObject>,
	writable: Heap<*mut JSObject>,

	#[trace(no_trace)]
	finish_promise: Option<Promise>,
	error: Option<Heap<JSVal>>,
}

impl TransformStream {
	pub fn from_heap<'cx>(cx: &'cx Context, heap: &Heap<*mut JSObject>) -> &'cx Self {
		<Self as ClassDefinition>::get_private(cx, &heap.root(cx).into()).unwrap()
	}

	pub fn from_heap_mut<'cx>(cx: &'cx Context, heap: &Heap<*mut JSObject>) -> &'cx mut Self {
		<Self as ClassDefinition>::get_mut_private(cx, &heap.root(cx).into()).unwrap()
	}

	pub fn from_traced_heap<'cx>(cx: &'cx Context, heap: &TracedHeap<*mut JSObject>) -> &'cx Self {
		<Self as ClassDefinition>::get_private(cx, &heap.root(cx).into()).unwrap()
	}

	pub fn from_traced_heap_mut<'cx>(cx: &'cx Context, heap: &TracedHeap<*mut JSObject>) -> &'cx mut Self {
		<Self as ClassDefinition>::get_mut_private(cx, &heap.root(cx).into()).unwrap()
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
			cx.root(unsafe { ReadableStreamGetController(cx.as_ptr(), self.readable.root(cx).handle().into()) }),
		)
	}

	pub fn get_writable_controller<'cx>(&self, cx: &'cx Context) -> Object<'cx> {
		Object::from(
			cx.root(unsafe { WritableStreamGetController(cx.as_ptr(), self.writable.root(cx).handle().into()) }),
		)
	}

	pub fn get_or_create_finish_promise(&mut self, cx: &Context, reason: Value) -> (Promise, bool) {
		fn finish_promise_inner(ts: &TransformStream, cx: &Context) -> Promise {
			match ts.error {
				Some(ref e) => Promise::rejected(cx, Value::from(e.root(cx))),
				None => Promise::resolved(cx, Value::undefined(cx)),
			}
		}

		match self.finish_promise {
			Some(ref p) => (p.clone(), false),
			None => {
				let promise = match self.get_controller(cx).transformer.cancel_function(cx) {
					None => finish_promise_inner(self, cx),
					Some((o, f)) => {
						let result = f.call(cx, &o, &[reason]);
						match result {
							Err(Some(e)) => Promise::rejected(cx, e.exception.as_value(cx)),
							Err(None) => Promise::rejected(cx, Value::undefined(cx)),
							Ok(v) if v.get().is_object() => match Promise::from(v.to_object(cx).into_local()) {
								Some(p) => {
									let finish_promise = Promise::new(cx);
									let fp1 = finish_promise.clone();
									let fp2 = finish_promise.clone();
									let this_heap1 = TracedHeap::new(self.reflector().get());
									let this_heap2 = TracedHeap::new(self.reflector().get());
									p.add_reactions(
										cx,
										Some(Function::from_closure(
											cx,
											"",
											Box::new(move |args| {
												let cx = args.cx();
												let this = Self::from_traced_heap(cx, &this_heap1);
												match this.error {
													Some(ref e) => fp1.reject(cx, &e.root(cx).into()),
													None => fp1.resolve(cx, &Value::undefined(cx)),
												};
												Ok(Value::undefined(cx))
											}),
											1,
											PropertyFlags::empty(),
										)),
										Some(Function::from_closure(
											cx,
											"",
											Box::new(move |args| {
												let cx = args.cx();
												let this = Self::from_traced_heap(cx, &this_heap2);
												match this.error {
													Some(ref e) => fp2.reject(cx, &e.root(cx).into()),
													None => fp2.reject(cx, &args.access().value()),
												};
												Ok(Value::undefined(cx))
											}),
											1,
											PropertyFlags::empty(),
										)),
									);
									finish_promise
								}
								None => finish_promise_inner(self, cx),
							},
							Ok(_) => finish_promise_inner(self, cx),
						}
					}
				};
				self.finish_promise = Some(unsafe { Promise::from_unchecked(cx.root(promise.get())) });
				(promise, true)
			}
		}
	}

	pub fn error(&mut self, cx: &Context, e: &Value) -> Result<()> {
		unsafe {
			if !ReadableStreamError(cx.as_ptr(), self.readable.root(cx).handle().into(), e.handle().into()) {
				return Err(Error::none());
			}
		}

		self.error_writable_and_unblock_write(cx, e)
	}

	pub fn error_writable_and_unblock_write(&mut self, cx: &Context, e: &Value) -> Result<()> {
		self.error = Some(Heap::new(e.get()));

		TransformStreamDefaultController::from_heap_mut(cx, &self.controller).clear_algorithms();

		let writable = self.writable.root(cx);

		unsafe {
			if WritableStreamGetState(cx.as_ptr(), writable.handle().into()) == WritableStreamState::Writable
				&& !WritableStreamError(cx.as_ptr(), writable.handle().into(), e.handle().into())
			{
				return Err(Error::none());
			}
		}

		Ok(())
	}

	fn call_start(cx: &Context, this: &mut Object) -> ResultExc<()> {
		let ts = Self::get_private(cx, this).unwrap();
		let controller = TransformStreamDefaultController::get_private(cx, &ts.controller.root(cx).into()).unwrap();
		let controller_value = Object::from(ts.controller.root(cx)).as_value(cx);
		match controller.transformer.start_function(cx) {
			Some((o, f)) => match f.call(cx, &o, &[controller_value]) {
				Ok(val) => {
					ts.start_promise.resolve(cx, &val);
					Ok(())
				}
				Err(Some(e)) => {
					ts.start_promise.reject(cx, &e.exception.as_value(cx));
					Err(e.exception)
				}
				Err(None) => {
					ts.start_promise.reject(cx, &Value::undefined(cx));
					Err(Error::none().into())
				}
			},
			None => {
				ts.start_promise.resolve(cx, &Value::undefined(cx));
				Ok(())
			}
		}
	}
}

#[js_class]
impl TransformStream {
	#[ion(constructor, post_construct = call_start)]
	pub fn constructor<'cx>(
		cx: &'cx Context, #[ion(this)] this: &Object<'cx>, Opt(transformer_object): Opt<Object<'cx>>,
	) -> ResultExc<TransformStream> {
		let transformer = HeapTransformer::from_transformer(cx, transformer_object)?;

		let start_promise = Promise::new(cx);

		let controller =
			ClassDefinition::new_object(cx, Box::new(TransformStreamDefaultController::new(this, transformer)));

		let sink = Sink {
			stream: TracedHeap::from_local(this),
			start_promise: start_promise.clone(),
		};
		let sink_obj = cx.root(NativeStreamSink::new_object(
			cx,
			Box::new(NativeStreamSink::new(Box::new(sink))),
		));

		let writable = unsafe {
			cx.root::<*mut JSObject>(NewWritableDefaultStreamObject(
				cx.as_ptr(),
				sink_obj.handle().into(),
				HandleFunction::from_marked_location(&super::readable_stream_extensions::NULL_FUNCTION),
				1.0,
				HandleObject::null(),
			))
		};

		if writable.get().is_null() {
			return Err(Error::new("Failed to create writable half of stream", ErrorKind::Normal).into());
		}

		let source = Source {
			stream: TracedHeap::from_local(this),
			start_promise: start_promise.clone(),
		};

		let readable = match super::readable_stream_extensions::readable_stream_from_callbacks(cx, Box::new(source)) {
			Some(readable) => readable,
			None => return Err(Error::new("Failed to create readable half of stream", ErrorKind::Normal).into()),
		};

		Ok(Self {
			reflector: Default::default(),
			start_promise,
			controller: Heap::new(controller),
			readable: Heap::new(readable.get()),
			writable: Heap::from_local(&writable),
			finish_promise: None,
			error: None,
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
