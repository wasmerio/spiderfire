use std::cell::RefCell;

use ion::{
	conversions::ToValue, flags::PropertyFlags, function::Opt, js_fn, object::WritableStream, Context, Error,
	ErrorKind, Function, Object, Promise, ReadableStream, Result, PermanentHeap, Value,
};
use mozjs::jsapi::JSFunction;

thread_local! {
	static STREAM_PIPE_TO: RefCell<Option<PermanentHeap<*mut JSFunction>>> = const { RefCell::new(None) };
}

#[js_fn]
fn pipe_through<'cx>(
	cx: &'cx Context, #[ion(this)] this: &Object<'cx>, transformer: Object<'cx>, Opt(options): Opt<Value<'cx>>,
) -> Result<Object<'cx>> {
	if !ReadableStream::is_readable_stream((**this).get()) {
		return Err(Error::new(
			"pipeThrough must be called on a ReadableStream",
			ErrorKind::Type,
		));
	}

	if ReadableStream::static_is_locked(cx, this) {
		return Err(Error::new(
			"pipeThrough called on a stream that's already locked",
			ErrorKind::Normal,
		));
	}

	let Some(readable_end) = transformer.get(cx, "readable")? else {
		return Err(Error::new(
			"First argument to pipeThrough must be an object with a readable property that is a ReadableStream",
			ErrorKind::Type,
		));
	};

	if !readable_end.get().is_object() || !ReadableStream::is_readable_stream((*readable_end.to_object(cx)).get()) {
		return Err(Error::new(
			"First argument to pipeThrough must be an object with a readable property that is a ReadableStream",
			ErrorKind::Type,
		));
	}

	let readable_end = readable_end.to_object(cx);

	let Some(writable_end) = transformer.get(cx, "writable")? else {
		return Err(Error::new(
			"First argument to pipeThrough must be an object with a writable property that is a writableStream",
			ErrorKind::Type,
		));
	};

	if !writable_end.get().is_object() || !WritableStream::is_writable_stream(&writable_end.to_object(cx)) {
		return Err(Error::new(
			"First argument to pipeThrough must be an object with a writable property that is a writableStream",
			ErrorKind::Type,
		));
	}

	let writable_end = writable_end.to_object(cx);

	let pipe_to_fn = Function::from(STREAM_PIPE_TO.with(|l| {
		l.borrow()
			.as_ref()
			.expect("The pipeTo function should have been found during initialization")
			.root(cx)
	}));

	let Ok(rval) = pipe_to_fn.call(
		cx,
		this,
		&[
			writable_end.as_value(cx),
			options.unwrap_or_else(|| Value::undefined(cx)),
		],
	) else {
		return Err(Error::none());
	};

	let promise = Promise::from(rval.to_object(cx).into_local()).expect("Return value of pipeTo should be a promise");

	// Apparently, this sets the PromiseIsHandled slot.
	promise.add_reactions_ignoring_unhandled_rejection(cx, None, None);

	Ok(readable_end)
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

	let Ok(Some(pipe_to)) = readable_stream_prototype.get(cx, "pipeTo") else {
		return false;
	};

	let pipe_to = if pipe_to.get().is_object() {
		pipe_to.to_object(cx)
	} else {
		return false;
	};

	let Some(pipe_to_fn) = Function::from_object(cx, &pipe_to) else {
		return false;
	};

	STREAM_PIPE_TO.with(move |l| l.replace(Some(PermanentHeap::from_local(&pipe_to_fn))));

	readable_stream_prototype.define_method(cx, "pipeThrough", pipe_through, 1, PropertyFlags::ENUMERATE);

	true
}
