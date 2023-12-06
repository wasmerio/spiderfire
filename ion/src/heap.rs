use mozjs_sys::jsgc::{GCMethods, RootKind};
use mozjs::{
	jsapi::Heap as JSHeap,
	jsapi::{JSObject, JSString, JSScript, PropertyKey, JSFunction, BigInt, Symbol},
	jsval::JSVal,
	rust::{RootedTraceableSet, Traceable},
};

use crate::{Context, Local};

macro_rules! impl_heap_root {
	([$class:ident] $(($root_method:ident, $pointer:ty)$(,)?)*) => {
        $(
            impl $class<$pointer> {
                pub fn root<'cx>(&self, cx: &'cx Context) -> Local<'cx, $pointer> {
                    cx.$root_method(self.heap.get())
                }
            }
        )*
	};
}

/// Value stored on the heap. [Heap<T>] instances are **not**
/// automatically traced, and must be traced in the usual way.
pub struct Heap<T>
where
	T: GCMethods + Copy + 'static,
	JSHeap<T>: Traceable,
{
	heap: Box<mozjs::jsapi::Heap<T>>,
}

impl<T> Heap<T>
where
	T: GCMethods + Copy + 'static,
	JSHeap<T>: Traceable + Default,
{
	pub fn new(ptr: T) -> Self {
		Self { heap: JSHeap::boxed(ptr) }
	}

	pub fn get(&self) -> T {
		self.heap.get()
	}

	pub fn set(&self, v: T) {
		self.heap.set(v)
	}
}

impl<T> Heap<T>
where
	T: GCMethods + Copy + RootKind + 'static,
	JSHeap<T>: Traceable + Default,
{
	pub fn from_local(local: Local<'_, T>) -> Self {
		Self::new(local.get())
	}
}

impl_heap_root! {
	[Heap]
	(root_value, JSVal),
	(root_object, *mut JSObject),
	(root_string, *mut JSString),
	(root_script, *mut JSScript),
	(root_property_key, PropertyKey),
	(root_function, *mut JSFunction),
	(root_bigint, *mut BigInt),
	(root_symbol, *mut Symbol),
}

unsafe impl<T> Traceable for Heap<T>
where
	T: GCMethods + Copy + RootKind + 'static,
	JSHeap<T>: Traceable + Default,
{
	unsafe fn trace(&self, trc: *mut mozjs_sys::jsapi::JSTracer) {
		unsafe { self.heap.trace(trc) };
	}
}

/// Value stored on the heap and traced automatically. There is
/// no need to trace [TracedHeap<T>] instances, and thus there
/// is no [Traceable] implementation for this type.
pub struct TracedHeap<T>
where
	T: GCMethods + Copy + 'static,
	JSHeap<T>: Traceable,
{
	heap: Box<mozjs::jsapi::Heap<T>>,
}

impl<T> TracedHeap<T>
where
	T: GCMethods + Copy + 'static,
	JSHeap<T>: Traceable + Default,
{
	pub fn new(ptr: T) -> Self {
		let heap = JSHeap::boxed(ptr);
		unsafe { RootedTraceableSet::add(&*heap) };
		Self { heap }
	}

	pub fn get(&self) -> T {
		self.heap.get()
	}

	pub fn set(&self, v: T) {
		self.heap.set(v)
	}
}

impl<T> TracedHeap<T>
where
	T: GCMethods + Copy + RootKind + 'static,
	JSHeap<T>: Traceable + Default,
{
	pub fn from_local(local: Local<'_, T>) -> Self {
		Self::new(local.get())
	}
}

impl<T> Drop for TracedHeap<T>
where
	T: GCMethods + Copy + 'static,
	JSHeap<T>: Traceable,
{
	fn drop(&mut self) {
		unsafe { RootedTraceableSet::remove(&*self.heap) }
	}
}

impl_heap_root! {
	[TracedHeap]
	(root_value, JSVal),
	(root_object, *mut JSObject),
	(root_string, *mut JSString),
	(root_script, *mut JSScript),
	(root_property_key, PropertyKey),
	(root_function, *mut JSFunction),
	(root_bigint, *mut BigInt),
	(root_symbol, *mut Symbol),
}
