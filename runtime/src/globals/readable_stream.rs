use std::{ffi::c_void, cell::RefCell};

use bytes::Bytes;
use ion::{Object, Context};
use mozjs::{
	jsapi::{
		JSContext, HandleObject, JS_GetArrayBufferViewData, AutoRequireNoGC, ReadableStreamUnderlyingSource, HandleValue,
		ReadableStreamUpdateDataAvailableFromSource, NewReadableExternalSourceStreamObject, ReadableStreamClose,
	},
	glue::{
		DeleteReadableStreamUnderlyingSource, ReadableStreamUnderlyingSourceTraps, CreateReadableStreamUnderlyingSource,
		ReadableStreamUnderlyingSourceGetSource,
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

pub fn new_memory_backed<'cx>(cx: &'cx Context, bytes: Bytes) -> Object<'cx> {
	let available = bytes.len();

	let source = Box::into_raw(Box::new(MemoryBackedReadableStream { bytes: RefCell::new(bytes) }));

	let js_stream = unsafe {
		let js_wrapper = CreateReadableStreamUnderlyingSource(&UNDERLYING_SOURCE_TRAPS, source as *const c_void);

		cx.root_object(NewReadableExternalSourceStreamObject(
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

	js_stream.into()
}

pub struct MemoryBackedReadableStream {
	bytes: RefCell<Bytes>,
}

unsafe extern "C" fn request_data(_source: *const c_void, _cx: *mut JSContext, _stream: HandleObject, _desired_size: usize) {
	// Note: this is the place to pull more data by querying sources.
	// Once we have more data, we mush call ReadableStreamUpdateDataAvailableFromSource
	// to signal the availability of more bytes.
}

#[allow(unsafe_code)]
unsafe extern "C" fn write_into_read_request_buffer(
	source: *const c_void, _cx: *mut JSContext, _stream: HandleObject, chunk: HandleObject, length: usize, bytes_written: *mut usize,
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
unsafe extern "C" fn cancel(_source: *const c_void, _cx: *mut JSContext, _stream: HandleObject, _reason: HandleValue, _resolve_to: *mut JSVal) {}

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
		assert!(bytes.len() >= length as usize);
		let mut chunk = bytes.split_off(length);
		std::mem::swap(&mut chunk, &mut *bytes);
		dest.copy_from_slice(chunk.as_ref());
	}
}

/* FOR READING FROM A STREAM NATIVELY
   /// Acquires a reader and locks the stream,
   /// must be done before `read_a_chunk`.
   #[allow(unsafe_code)]
   pub fn start_reading(&self) -> Result<(), ()> {
	   if self.is_locked() || self.is_disturbed() {
		   return Err(());
	   }

	   let global = self.global();
	   let _ar = enter_realm(&*global);
	   let cx = GlobalScope::get_cx();

	   unsafe {
		   rooted!(in(*cx) let reader = ReadableStreamGetReader(
			   *cx,
			   self.js_stream.handle(),
			   ReadableStreamReaderMode::Default,
		   ));

		   // Note: the stream is locked to the reader.
		   self.js_reader.set(reader.get());
	   }

	   self.has_reader.set(true);
	   Ok(())
   }

   /// Read a chunk from the stream,
   /// must be called after `start_reading`,
   /// and before `stop_reading`.
   #[allow(unsafe_code)]
   pub fn read_a_chunk(&self) -> Rc<Promise> {
	   if !self.has_reader.get() {
		   panic!("Attempt to read stream chunk without having acquired a reader.");
	   }

	   let global = self.global();
	   let _ar = enter_realm(&*global);
	   let _aes = AutoEntryScript::new(&*global);

	   let cx = GlobalScope::get_cx();

	   unsafe {
		   rooted!(in(*cx) let promise_obj = ReadableStreamDefaultReaderRead(
			   *cx,
			   self.js_reader.handle(),
		   ));
		   Promise::new_with_js_promise(promise_obj.handle(), cx)
	   }
   }

   /// Releases the lock on the reader,
   /// must be done after `start_reading`.
   #[allow(unsafe_code)]
   pub fn stop_reading(&self) {
	   if !self.has_reader.get() {
		   panic!("ReadableStream::stop_reading called on a readerless stream.");
	   }

	   self.has_reader.set(false);

	   let global = self.global();
	   let _ar = enter_realm(&*global);
	   let cx = GlobalScope::get_cx();

	   unsafe {
		   ReadableStreamReaderReleaseLock(*cx, self.js_reader.handle());
		   // Note: is this the way to nullify the Heap?
		   self.js_reader.set(ptr::null_mut());
	   }
   }

   #[allow(unsafe_code)]
   pub fn is_locked(&self) -> bool {
	   // If we natively took a reader, we're locked.
	   if self.has_reader.get() {
		   return true;
	   }

	   // Otherwise, still double-check that script didn't lock the stream.
	   let cx = GlobalScope::get_cx();
	   let mut locked_or_disturbed = false;

	   unsafe {
		   ReadableStreamIsLocked(*cx, self.js_stream.handle(), &mut locked_or_disturbed);
	   }

	   locked_or_disturbed
   }

   #[allow(unsafe_code)]
   pub fn is_disturbed(&self) -> bool {
	   // Check that script didn't disturb the stream.
	   let cx = GlobalScope::get_cx();
	   let mut locked_or_disturbed = false;

	   unsafe {
		   ReadableStreamIsDisturbed(*cx, self.js_stream.handle(), &mut locked_or_disturbed);
	   }

	   locked_or_disturbed
   }


#[allow(unsafe_code)]
/// Get the `done` property of an object that a read promise resolved to.
pub fn get_read_promise_done(cx: SafeJSContext, v: &SafeHandleValue) -> Result<bool, Error> {
	unsafe {
		rooted!(in(*cx) let object = v.to_object());
		rooted!(in(*cx) let mut done = UndefinedValue());
		match get_dictionary_property(*cx, object.handle(), "done", done.handle_mut()) {
			Ok(true) => match bool::from_jsval(*cx, done.handle(), ()) {
				Ok(ConversionResult::Success(val)) => Ok(val),
				Ok(ConversionResult::Failure(error)) => Err(Error::Type(error.to_string())),
				_ => Err(Error::Type("Unknown format for done property.".to_string())),
			},
			Ok(false) => Err(Error::Type("Promise has no done property.".to_string())),
			Err(()) => Err(Error::JSFailed),
		}
	}
}

#[allow(unsafe_code)]
/// Get the `value` property of an object that a read promise resolved to.
pub fn get_read_promise_bytes(cx: SafeJSContext, v: &SafeHandleValue) -> Result<Vec<u8>, Error> {
	unsafe {
		rooted!(in(*cx) let object = v.to_object());
		rooted!(in(*cx) let mut bytes = UndefinedValue());
		match get_dictionary_property(*cx, object.handle(), "value", bytes.handle_mut()) {
			Ok(true) => match Vec::<u8>::from_jsval(*cx, bytes.handle(), ConversionBehavior::EnforceRange) {
				Ok(ConversionResult::Success(val)) => Ok(val),
				Ok(ConversionResult::Failure(error)) => Err(Error::Type(error.to_string())),
				_ => Err(Error::Type("Unknown format for bytes read.".to_string())),
			},
			Ok(false) => Err(Error::Type("Promise has no value property.".to_string())),
			Err(()) => Err(Error::JSFailed),
		}
	}
}
*/
