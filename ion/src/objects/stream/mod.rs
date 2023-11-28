use std::ops::{Deref, DerefMut};

use bytes::Bytes;
use mozjs::jsapi::{
	JSObject, ReadableStreamIsLocked, ReadableStreamIsDisturbed, ReadableStreamGetReader, ReadableStreamReaderMode, ReadableStreamReaderReleaseLock,
};

use crate::{Local, Context, Error, ErrorKind, Object};

mod memory_backed_readable_stream;

#[derive(Debug)]
pub struct ReadableStream<'r> {
	stream: Local<'r, *mut JSObject>,
}

impl<'r> ReadableStream<'r> {
	pub fn from_bytes(cx: &'r Context, bytes: Bytes) -> Self {
		Self {
			stream: memory_backed_readable_stream::new_memory_backed(cx, bytes),
		}
	}

	pub fn is_locked(&self, cx: &'r Context) -> bool {
		let mut locked = false;

		unsafe {
			ReadableStreamIsLocked(cx.as_ptr(), self.stream.handle().into(), &mut locked);
		}

		locked
	}

	pub fn is_disturbed(&self, cx: &'r Context) -> bool {
		let mut disturbed = false;

		unsafe {
			ReadableStreamIsDisturbed(cx.as_ptr(), self.stream.handle().into(), &mut disturbed);
		}

		disturbed
	}

	pub fn to_object(&self, cx: &'r Context) -> Object<'r> {
		Object::from(cx.root_object(self.stream.handle().get()))
	}

	// Lock the stream and acquire a reader
	pub fn into_reader(self, cx: &'r Context) -> crate::Result<ReadableStreamReader> {
		if self.is_locked(cx) || self.is_disturbed(cx) {
			return Err(Error::new("Stream is already locked or disturbed", ErrorKind::Normal));
		}

		let reader = unsafe {
			cx.root_object(ReadableStreamGetReader(
				cx.as_ptr(),
				self.stream.handle().into(),
				ReadableStreamReaderMode::Default,
			))
		};

		Ok(ReadableStreamReader { stream: self.stream, reader })
	}
}

impl<'r> Deref for ReadableStream<'r> {
	type Target = Local<'r, *mut JSObject>;

	fn deref(&self) -> &Self::Target {
		&self.stream
	}
}

impl<'r> DerefMut for ReadableStream<'r> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.stream
	}
}

pub struct ReadableStreamReader<'r> {
	stream: Local<'r, *mut JSObject>,
	reader: Local<'r, *mut JSObject>,
}

impl<'r> ReadableStreamReader<'r> {
	// Release the stream lock and turn the reader back into a stream
	pub fn into_stream(self, cx: &'r Context) -> ReadableStream {
		unsafe {
			ReadableStreamReaderReleaseLock(cx.as_ptr(), self.reader.handle().into());
		}

		ReadableStream { stream: self.stream }
	}
}
