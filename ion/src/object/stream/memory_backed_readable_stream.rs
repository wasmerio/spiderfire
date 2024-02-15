use std::{ffi::c_void, cell::RefCell};

use bytes::Bytes;
use crate::{Context, TracedHeap};
use mozjs::{
	jsapi::{
		JSContext, JSObject, HandleObject, JS_GetArrayBufferViewData, AutoRequireNoGC, ReadableStreamUnderlyingSource,
		HandleValue, ReadableStreamUpdateDataAvailableFromSource, NewReadableExternalSourceStreamObject,
		ReadableStreamClose,
	},
	glue::{
		DeleteReadableStreamUnderlyingSource, ReadableStreamUnderlyingSourceTraps,
		CreateReadableStreamUnderlyingSource, ReadableStreamUnderlyingSourceGetSource,
	},
	jsval::JSVal,
};

static UNDERLYING_SOURCE_TRAPS: ReadableStreamUnderlyingSourceTraps = ReadableStreamUnderlyingSourceTraps {
	requestData: Some(request_data),
	writeIntoReadRequestBuffer: Some(write_into_read_request_buffer),
	cancel: Some(cancel),
	onClosed: Some(close),
	onErrored: Some(error),
	finalize: Some(finalize),
};

pub fn new_memory_backed(cx: &Context, bytes: Bytes) -> TracedHeap<*mut JSObject> {
	let available = bytes.len();

	let source = Box::into_raw(Box::new(MemoryBackedReadableStream { bytes: RefCell::new(bytes) }));

	let js_stream = unsafe {
		let js_wrapper = CreateReadableStreamUnderlyingSource(&UNDERLYING_SOURCE_TRAPS, source as *const c_void);

		cx.root::<*mut JSObject>(NewReadableExternalSourceStreamObject(
			cx.as_ptr(),
			js_wrapper,
			std::ptr::null_mut(),
			HandleObject::null(),
		))
	};

	if available > 0 {
		unsafe {
			ReadableStreamUpdateDataAvailableFromSource(cx.as_ptr(), js_stream.handle().into(), available as u32);
		}
	}

	unsafe { ReadableStreamClose(cx.as_ptr(), js_stream.handle().into()) };

	TracedHeap::from_local(&js_stream)
}

struct MemoryBackedReadableStream {
	bytes: RefCell<Bytes>,
}

unsafe extern "C" fn request_data(
	_source: *const c_void, _cx: *mut JSContext, _stream: HandleObject, _desired_size: usize,
) {
	// Note: this is the place to pull more data by querying sources.
	// Once we have more data, we mush call ReadableStreamUpdateDataAvailableFromSource
	// to signal the availability of more bytes.
}

#[allow(unsafe_code)]
unsafe extern "C" fn write_into_read_request_buffer(
	source: *const c_void, _cx: *mut JSContext, _stream: HandleObject, chunk: HandleObject, length: usize,
	bytes_written: *mut usize,
) {
	unsafe {
		let source = &*(source as *const MemoryBackedReadableStream);
		let mut is_shared_memory = false;
		let buffer = JS_GetArrayBufferViewData(*chunk, &mut is_shared_memory, &AutoRequireNoGC { _address: 0 });
		assert!(!is_shared_memory);
		let slice = std::slice::from_raw_parts_mut(buffer as *mut u8, length);
		source.write_into_buffer(slice);

		// Currently we're always able to completely fulfill the write request.
		*bytes_written = length;
	}
}

#[allow(unsafe_code)]
unsafe extern "C" fn cancel(
	_source: *const c_void, _cx: *mut JSContext, _stream: HandleObject, _reason: HandleValue, _resolve_to: *mut JSVal,
) {
}

#[allow(unsafe_code)]
unsafe extern "C" fn close(_source: *const c_void, _cx: *mut JSContext, _stream: HandleObject) {}

#[allow(unsafe_code)]
unsafe extern "C" fn error(_source: *const c_void, _cx: *mut JSContext, _stream: HandleObject, _reason: HandleValue) {}

#[allow(unsafe_code)]
unsafe extern "C" fn finalize(source: *mut ReadableStreamUnderlyingSource) {
	unsafe {
		let rust_source = ReadableStreamUnderlyingSourceGetSource(source);
		drop(Box::from_raw(rust_source as *mut MemoryBackedReadableStream));

		DeleteReadableStreamUnderlyingSource(source);
	}
}

impl MemoryBackedReadableStream {
	fn write_into_buffer(&self, dest: &mut [u8]) {
		let length = dest.len();
		let mut bytes = self.bytes.borrow_mut();
		assert!(bytes.len() >= length);
		let mut chunk = bytes.split_off(length);
		std::mem::swap(&mut chunk, &mut *bytes);
		dest.copy_from_slice(chunk.as_ref());
	}
}
