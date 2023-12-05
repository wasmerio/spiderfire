use mozjs::jsapi::{JSObject, IsWritableStream, WritableStreamIsLocked};

use crate::{TracedHeap, Object, Context, Local};

pub struct WritableStream {
	// Since streams are async by nature, they cannot be tied to the lifetime
	// of one Context.
	stream: TracedHeap<*mut JSObject>,
}

impl WritableStream {
	pub fn from_local(local: Local<'_, *mut JSObject>) -> Option<Self> {
		if Self::is_writable_stream(&local) {
			Some(Self { stream: TracedHeap::from_local(local) })
		} else {
			None
		}
	}

	pub fn is_writable_stream(obj: &Local<'_, *mut JSObject>) -> bool {
		unsafe { IsWritableStream(obj.get()) }
	}

	pub fn is_locked(&self, cx: &Context) -> bool {
		unsafe { WritableStreamIsLocked(cx.as_ptr(), self.stream.root(&cx).handle().into()) }
	}

	pub fn static_is_locked(cx: &Context, obj: &Local<'_, *mut JSObject>) -> bool {
		unsafe { WritableStreamIsLocked(cx.as_ptr(), obj.handle().into()) }
	}

	pub fn to_object<'cx>(&self, cx: &'cx Context) -> Object<'cx> {
		Object::from(cx.root_object(self.stream.root(cx).handle().get()))
	}
}
