use ion::{ClassDefinition, Context, ReadableStream};
use mozjs::jsapi::{HandleFunction, HandleObject, JSFunction, NewReadableDefaultStreamObject};

use super::{NativeStreamSource, NativeStreamSourceCallbacks};

mod pipe_through;
mod tee;

pub const NULL_FUNCTION: *mut JSFunction = 0 as *mut JSFunction;

pub fn readable_stream_from_callbacks(
	cx: &Context, callbacks: Box<dyn NativeStreamSourceCallbacks>,
) -> Option<ReadableStream> {
	let source_obj = cx.root(NativeStreamSource::new_object(
		cx,
		Box::new(NativeStreamSource::new(callbacks)),
	));

	let stream_obj = unsafe {
		NewReadableDefaultStreamObject(
			cx.as_ptr(),
			source_obj.handle().into(),
			HandleFunction::from_marked_location(&NULL_FUNCTION),
			1.0,
			HandleObject::null(),
		)
	};

	if stream_obj.is_null() {
		None
	} else {
		// This should always succeed
		ReadableStream::new(stream_obj)
	}
}

pub fn define(cx: &ion::Context, global: &ion::Object) -> bool {
	pipe_through::define(cx, global) && tee::define(cx, global)
}
