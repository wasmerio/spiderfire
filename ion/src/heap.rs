use std::fmt::Debug;

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
#[derive(Debug)]
pub struct Heap<T>
where
	T: GCMethods + Copy + 'static,
	JSHeap<T>: Traceable,
{
	heap: Box<JSHeap<T>>,
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
	pub fn from_local(local: &Local<'_, T>) -> Self {
		Self::new(local.get())
	}

	/// This constructs a Local from the Heap directly as opposed to rooting on the stack.
	/// The returned Local cannot be used to construct a HandleMut.
	pub fn to_local<'a>(&'a self) -> Local<'a, T> {
		unsafe { Local::from_heap(&self.heap) }
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

impl<T> Clone for Heap<T>
where
	T: GCMethods + Copy + 'static,
	JSHeap<T>: Traceable + Default,
{
	fn clone(&self) -> Self {
		Self::new(self.heap.get())
	}
}

unsafe impl<T> Traceable for Heap<T>
where
	T: GCMethods + Copy + 'static,
	JSHeap<T>: Traceable + Default,
{
	unsafe fn trace(&self, trc: *mut mozjs_sys::jsapi::JSTracer) {
		unsafe { self.heap.trace(trc) };
	}
}

/// Value stored on the heap and traced automatically. There is
/// no need to trace [TracedHeap<T>] instances, and thus there
/// is no [Traceable] implementation for this type.
#[derive(Debug)]
pub struct TracedHeap<T>
where
	T: GCMethods + Copy + 'static,
	JSHeap<T>: Traceable,
{
	heap: Box<JSHeap<T>>,
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
	pub fn from_local(local: &Local<'_, T>) -> Self {
		Self::new(local.get())
	}

	/// This constructs a Local from the Heap directly as opposed to rooting on the stack.
	/// The returned Local cannot be used to construct a HandleMut.
	pub fn to_local<'a>(&'a self) -> Local<'a, T> {
		unsafe { Local::from_heap(&self.heap) }
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

impl<T> Clone for TracedHeap<T>
where
	T: GCMethods + Copy + 'static,
	JSHeap<T>: Traceable + Default,
{
	fn clone(&self) -> Self {
		Self::new(self.heap.get())
	}
}

trait Private {}

#[allow(private_bounds)]
pub trait HeapPointer<T>: Private {
	fn to_ptr(&self) -> T;
}

impl<T> Private for Heap<T>
where
	T: GCMethods + Copy + 'static,
	JSHeap<T>: Traceable + Default,
{
}

impl<T> HeapPointer<T> for Heap<T>
where
	T: GCMethods + Copy + 'static,
	JSHeap<T>: Traceable + Default,
{
	fn to_ptr(&self) -> T {
		self.heap.get()
	}
}

impl<T> Private for TracedHeap<T>
where
	T: GCMethods + Copy + 'static,
	JSHeap<T>: Traceable + Default,
{
}

impl<T> HeapPointer<T> for TracedHeap<T>
where
	T: GCMethods + Copy + 'static,
	JSHeap<T>: Traceable + Default,
{
	fn to_ptr(&self) -> T {
		self.heap.get()
	}
}
