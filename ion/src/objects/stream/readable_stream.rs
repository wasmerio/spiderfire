use std::{
	ops::{Deref, DerefMut},
	borrow::Cow,
};

use bytes::Bytes;
use mozjs::jsapi::{
	JSObject, ReadableStreamIsLocked, ReadableStreamIsDisturbed, ReadableStreamGetReader, ReadableStreamReaderMode,
	ReadableStreamReaderReleaseLock, ReadableStreamDefaultReaderRead, AutoRequireNoGC, IsReadableStream, ToStringSlow,
	IsArrayBufferObject, GetArrayBufferByteLength, GetArrayBufferData, ReadableStreamTee,
};
use mozjs_sys::jsapi::{JS_IsArrayBufferViewObject, JS_GetArrayBufferViewByteLength, JS_GetArrayBufferViewData};

use crate::{
	Context, Error, ErrorKind, Object, Promise, TracedHeap, PromiseFuture, ResultExc, Exception,
	conversions::{FromValue, ToValue},
	Local,
};

#[derive(Debug)]
pub struct ReadableStream {
	// Since streams are async by nature, they cannot be tied to the lifetime
	// of one Context.
	stream: TracedHeap<*mut JSObject>,
}

impl ReadableStream {
	pub fn new(obj: *mut JSObject) -> Option<Self> {
		if Self::is_readable_stream(obj) {
			Some(Self { stream: TracedHeap::new(obj) })
		} else {
			None
		}
	}

	pub fn from_local(local: &Local<'_, *mut JSObject>) -> Option<Self> {
		if Self::is_readable_stream(local.get()) {
			Some(Self { stream: TracedHeap::from_local(&local) })
		} else {
			None
		}
	}

	pub fn from_bytes(cx: &Context, bytes: Bytes) -> Self {
		Self {
			stream: super::memory_backed_readable_stream::new_memory_backed(cx, bytes),
		}
	}

	pub fn is_readable_stream(obj: *mut JSObject) -> bool {
		unsafe { IsReadableStream(obj) }
	}

	pub fn is_locked(&self, cx: &Context) -> bool {
		let mut locked = false;

		unsafe {
			ReadableStreamIsLocked(cx.as_ptr(), self.stream.root(&cx).handle().into(), &mut locked);
		}

		locked
	}

	pub fn static_is_locked(cx: &Context, obj: &Local<'_, *mut JSObject>) -> bool {
		let mut locked = false;

		unsafe {
			ReadableStreamIsLocked(cx.as_ptr(), obj.handle().into(), &mut locked);
		}

		locked
	}

	pub fn is_disturbed(&self, cx: &Context) -> bool {
		let mut disturbed = false;

		unsafe {
			ReadableStreamIsDisturbed(cx.as_ptr(), self.stream.root(cx).handle().into(), &mut disturbed);
		}

		disturbed
	}

	pub fn to_object<'cx>(&self, cx: &'cx Context) -> Object<'cx> {
		Object::from(cx.root_object(self.stream.root(cx).handle().get()))
	}

	// Lock the stream and acquire a reader
	pub fn into_reader(self, cx: &Context) -> crate::Result<ReadableStreamReader> {
		if self.is_locked(cx) || self.is_disturbed(cx) {
			return Err(Error::new("Stream is already locked or disturbed", ErrorKind::Normal));
		}

		let reader = unsafe {
			ReadableStreamGetReader(
				cx.as_ptr(),
				self.stream.root(cx).handle().into(),
				ReadableStreamReaderMode::Default,
			)
		};

		Ok(ReadableStreamReader {
			stream: self.stream,
			reader: TracedHeap::new(reader),
		})
	}

	pub fn try_clone(&mut self, cx: &Context) -> crate::Result<Self> {
		let stream = self.stream.root(cx).handle().into();
		let mut branch1 = Object::null(cx);
		let mut branch2 = Object::null(cx);

		unsafe {
			if !ReadableStreamTee(
				cx.as_ptr(),
				stream,
				branch1.handle_mut().into(),
				branch2.handle_mut().into(),
			) {
				return Err(Error::none());
			}
		}

		self.stream = TracedHeap::from_local(&branch1);
		Ok(Self::from_local(&branch2).unwrap())
	}
}

impl<'cx> FromValue<'cx> for ReadableStream {
	type Config = ();

	fn from_value(cx: &'cx Context, value: &crate::Value, _strict: bool, _config: Self::Config) -> crate::Result<Self> {
		if !value.get().is_object() {
			return Err(Error::new("Expected object for readable stream", ErrorKind::Type));
		}

		Self::new((*value.to_object(cx)).get())
			.ok_or_else(|| Error::new("The given object is not a readable stream", ErrorKind::Type))
	}
}

impl Deref for ReadableStream {
	type Target = TracedHeap<*mut JSObject>;

	fn deref(&self) -> &Self::Target {
		&self.stream
	}
}

impl DerefMut for ReadableStream {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.stream
	}
}

pub struct ReadableStreamReader {
	stream: TracedHeap<*mut JSObject>,
	reader: TracedHeap<*mut JSObject>,
}

impl ReadableStreamReader {
	// Release the stream lock and turn the reader back into a stream
	pub fn into_stream(self, cx: &Context) -> ReadableStream {
		unsafe {
			ReadableStreamReaderReleaseLock(cx.as_ptr(), self.reader.root(cx).handle().into());
		}

		ReadableStream { stream: self.stream }
	}

	pub fn into_rust_stream(self, mut cx: Context) -> impl futures::Stream<Item = crate::ResultExc<Vec<u8>>> {
		async_stream::try_stream! {
			loop {
				let chunk;
				(cx, chunk) = cx.await_native_cx(|cx| unsafe { self.read_chunk(cx) }).await;
				let chunk = chunk?;
				match chunk {
					Some(c) => yield c.to_vec(),
					None => break,
				}
			}
		}
	}

	fn read_chunk_raw<'cx>(&self, cx: &'cx Context) -> Promise {
		unsafe {
			let promise = cx.root_object(ReadableStreamDefaultReaderRead(
				cx.as_ptr(),
				self.reader.root(cx).handle().into(),
			));
			Promise::from(promise).expect("ReadableStreamDefaultReaderRead should return a Promise")
		}
	}

	// Safety: The returned slice must be consumed before the next
	// SpiderMonkey API call, which may cause GC to collect the chunk
	pub async unsafe fn read_chunk<'cx>(&self, mut cx: Context) -> ResultExc<Option<Cow<[u8]>>> {
		let chunk = self.read_chunk_raw(&cx);
		let chunk_val;
		(cx, chunk_val) = PromiseFuture::new(cx, &chunk).await;
		let chunk = match chunk_val {
			Ok(v) => {
				if !v.is_object() {
					return Err(Exception::Error(Error::new(
						"ReadableStreamDefaultReader.read() should return an object",
						ErrorKind::Type,
					)));
				}
				Object::from(cx.root_object(v.to_object()))
			}
			Err(v) => {
				return Err(Exception::Other(v));
			}
		};

		let done = bool::from_value(
			&cx,
			&chunk.get(&cx, "done").expect("Chunk must have a done property"),
			true,
			(),
		)
		.expect("chunk.done must be a boolean");

		if done {
			return Ok(None);
		}

		let obj = chunk.get(&cx, "value").expect("Chunk must have a value property");
		if obj.get().is_string() {
			let str = crate::String::from(cx.root_string(obj.get().to_string()));
			let str = str.to_owned(&cx);
			return Ok(Some(Cow::Owned(str.into_bytes())));
		} else if obj.get().is_object() {
			unsafe {
				let obj_ptr = (*obj.to_object(&cx)).get();
				if IsArrayBufferObject(obj_ptr) {
					let length = GetArrayBufferByteLength(obj_ptr);
					let mut is_shared_memory = false;
					let data_ptr = GetArrayBufferData(obj_ptr, &mut is_shared_memory, &AutoRequireNoGC { _address: 0 });
					return Ok(Some(Cow::Borrowed(std::slice::from_raw_parts(
						data_ptr as *const _,
						length,
					))));
				} else if JS_IsArrayBufferViewObject(obj_ptr) {
					let length = JS_GetArrayBufferViewByteLength(obj_ptr);
					let mut is_shared_memory = false;
					let data_ptr =
						JS_GetArrayBufferViewData(obj_ptr, &mut is_shared_memory, &AutoRequireNoGC { _address: 0 });
					return Ok(Some(Cow::Borrowed(std::slice::from_raw_parts(
						data_ptr as *const _,
						length,
					))));
				}
			}
		}

		let obj_str = crate::String::from(cx.root_string(ToStringSlow(cx.as_ptr(), obj.as_value(&cx).handle().into())));
		Err(Exception::Error(Error::new(
			format!("Cannot process chunk with unknown type: {}", obj_str.to_owned(&cx)).as_str(),
			ErrorKind::Type,
		)))
	}

	pub async fn read_to_end(&self, mut cx: Context) -> ResultExc<Vec<u8>> {
		let mut result = vec![];

		loop {
			let chunk;
			(cx, chunk) = cx.await_native_cx(|cx| unsafe { self.read_chunk(cx) }).await;
			match chunk? {
				Some(Cow::Borrowed(slice)) => result.extend_from_slice(slice),
				Some(Cow::Owned(v)) => result.extend_from_slice(&v),
				None => break Ok(result),
			}
		}
	}
}
