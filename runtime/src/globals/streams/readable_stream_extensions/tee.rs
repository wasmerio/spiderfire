use std::cell::RefCell;

use ion::{
	class::Reflector,
	conversions::{FromValue, ToValue},
	flags::PropertyFlags,
	typedarray::Uint8Array,
	Array, Context, Error, ErrorKind, Exception, Function, Heap, Object, PermanentHeap, Promise, Result, TracedHeap,
	Value,
};
use mozjs::{
	jsapi::{IsReadableByteStreamController, JSFunction, JSObject, ReadableStreamGetController},
	jsval::JSVal,
};

use crate::{
	globals::streams::{NativeStreamSource, NativeStreamSourceCallbacks},
	promise::future_to_promise,
};

use super::readable_stream_from_callbacks;

thread_local! {
	static STREAM_TEE: RefCell<Option<PermanentHeap<*mut JSFunction>>> = const { RefCell::new(None) };
}

pub(super) fn define(cx: &Context, global: &Object) -> bool {
	let Ok(Some(readable_stream)) = global.get(cx, "ReadableStream") else {
		return false;
	};

	let readable_stream = if readable_stream.get().is_object() {
		readable_stream.to_object(cx)
	} else {
		return false;
	};

	let Ok(Some(readable_stream_prototype)) = readable_stream.get(cx, "prototype") else {
		return false;
	};
	let readable_stream_prototype = readable_stream_prototype.to_object(cx);

	let Ok(Some(tee)) = readable_stream_prototype.get(cx, "tee") else {
		return false;
	};

	let tee = if tee.get().is_object() {
		tee.to_object(cx)
	} else {
		return false;
	};

	let Some(tee_fn) = Function::from_object(cx, &tee) else {
		return false;
	};

	STREAM_TEE.with(move |l| l.replace(Some(PermanentHeap::from_local(&tee_fn))));

	let new_tee_fn = readable_stream_prototype.define_method(cx, "tee", self::tee, 2, PropertyFlags::ENUMERATE);

	!new_tee_fn.get().is_null()
}

#[js_fn]
fn tee<'cx>(cx: &'cx Context, #[ion(this)] this: &Object) -> Result<Value<'cx>> {
	unsafe {
		let controller = ReadableStreamGetController(cx.as_ptr(), this.handle().into());
		if controller.is_null() {
			return Err(Error::none());
		}

		if IsReadableByteStreamController(controller) {
			readable_byte_stream_tee(cx, this)
		} else {
			let tee_fn = Function::from(STREAM_TEE.with(|l| {
				l.borrow()
					.as_ref()
					.expect("The tee function should have been found during initialization")
					.root(cx)
			}));
			tee_fn
				.call(cx, this, &[])
				.map_err(|e| e.map(|e| e.exception.to_error()).unwrap_or_else(Error::none))
		}
	}
}

#[js_class]
struct TeeState {
	reflector: Reflector,

	stream: Heap<*mut JSObject>,
	reader: Heap<*mut JSObject>,
	reading: bool,
	read_again_for_branch_1: bool,
	read_again_for_branch_2: bool,
	canceled_1: bool,
	canceled_2: bool,
	reason1: Heap<JSVal>,
	reason2: Heap<JSVal>,
	branch1: Heap<*mut JSObject>,
	branch2: Heap<*mut JSObject>,
	cancel_promise: Heap<*mut JSObject>,
}

#[js_class]
impl TeeState {
	#[ion(constructor)]
	pub fn constructor() -> Result<TeeState> {
		Err(Error::new("Cannot construct this type", ErrorKind::Type))
	}
}

fn readable_byte_stream_tee<'cx>(cx: &'cx Context, stream: &Object) -> Result<Value<'cx>> {
	// Note: this is a sub-optimal implementation, as it caches the
	// entire contents of the stream in memory.

	let bytes_promise = read_byte_stream(cx, stream);
	let source1 = TeedReadableStreamSource {
		bytes_promise: Promise::from_raw(bytes_promise.get(), cx).unwrap(),
	};
	let stream1 = readable_stream_from_callbacks(cx, Box::new(source1))
		.ok_or_else(|| Error::new("Failed to create stream", ErrorKind::Normal))?;
	let source2 = TeedReadableStreamSource { bytes_promise };
	let stream2 = readable_stream_from_callbacks(cx, Box::new(source2))
		.ok_or_else(|| Error::new("Failed to create stream", ErrorKind::Normal))?;

	let result = Array::new(cx);
	result.set(cx, 0, &Value::object(cx, &stream1.root(cx).into()));
	result.set(cx, 1, &Value::object(cx, &stream2.root(cx).into()));

	Ok(result.as_value(cx))
}

fn read_byte_stream(cx: &Context, stream: &Object) -> ion::Promise {
	unsafe {
		let stream = ion::ReadableStream::new((**stream).get()).expect("Expected parameter to be a ReadableStream");
		future_to_promise::<_, _, _, Error>(cx, move |mut cx| async move {
			let reader = stream.into_reader(&cx)?;
			let bytes;
			(cx, bytes) = cx.await_native_cx(|cx| reader.read_to_end(cx)).await;
			let bytes = bytes.map_err(|e| e.to_error())?;
			Ok(Uint8Array::from_vec(&cx, bytes).as_value(&cx).get())
		})
		.expect("Future queue should be running")
	}
}

struct TeedReadableStreamSource {
	bytes_promise: Promise,
}

impl NativeStreamSourceCallbacks for TeedReadableStreamSource {
	fn start<'cx>(
		&self, _source: &'cx NativeStreamSource, cx: &'cx Context, controller: Object<'cx>,
	) -> ion::ResultExc<Value<'cx>> {
		let controller_heap = TracedHeap::from_local(&controller);
		let controller_heap2 = TracedHeap::from_local(&controller);
		let new_promise = self
			.bytes_promise
			.then(
				cx,
				Some(Function::from_closure(
					cx,
					"",
					Box::new(move |args| {
						let cx = args.cx();
						let bytes = Uint8Array::from_value(cx, &args.access().value(), false, ())?;
						let bytes_clone = Uint8Array::copy_from_bytes(cx, unsafe { bytes.as_slice() })
							.ok_or_else(|| Error::new("Failed to allocate array", ErrorKind::Normal))?;
						let controller = Object::from(controller_heap.root(cx));
						let enqueue_func =
							Function::from_object(cx, &controller.get(cx, "enqueue")?.unwrap().to_object(cx)).unwrap();
						enqueue_func
							.call(cx, &controller, &[bytes_clone.as_value(cx)])
							.map_err(|e| e.unwrap().exception)?;
						let close_func =
							Function::from_object(cx, &controller.get(cx, "close")?.unwrap().to_object(cx)).unwrap();
						close_func.call(cx, &controller, &[]).map_err(|e| e.unwrap().exception)?;
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
						let reason = args.access().value();
						let controller = Object::from(controller_heap2.root(cx));
						let cancel_func =
							Function::from_object(cx, &controller.get(cx, "cancel")?.unwrap().to_object(cx)).unwrap();
						cancel_func.call(cx, &controller, &[reason]).map_err(|e| e.unwrap().exception)?;
						Ok(Value::undefined(cx))
					}),
					1,
					PropertyFlags::empty(),
				)),
			)
			.ok_or_else(|| Exception::Error(Error::new("Failed to create promise", ErrorKind::Normal)))?;
		Ok(Value::object(cx, &new_promise.root(cx).into()))
	}

	fn pull<'cx>(
		&self, _source: &'cx NativeStreamSource, cx: &'cx Context, _controller: Object<'cx>,
	) -> ion::ResultExc<Promise> {
		Ok(Promise::resolved(cx, Value::undefined(cx)))
	}

	fn cancel(self: Box<Self>, cx: &Context, _reason: Value) -> ion::ResultExc<Promise> {
		Ok(Promise::resolved(cx, Value::undefined(cx)))
	}
}
