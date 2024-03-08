/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

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
	([$class:ident] $(($pointer:ty)$(,)?)*) => {
        $(
            impl $class<$pointer> {
                pub fn root<'cx>(&self, cx: &'cx Context) -> Local<'cx, $pointer> {
                    cx.root(self.heap.get())
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

	pub fn to_traced(&self) -> TracedHeap<T> {
		TracedHeap::new(self.heap.get())
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
	pub fn to_local(&self) -> Local<'_, T> {
		unsafe { Local::from_heap(&self.heap) }
	}
}

impl_heap_root! {
	[Heap]
	(JSVal),
	(*mut JSObject),
	(*mut JSString),
	(*mut JSScript),
	(PropertyKey),
	(*mut JSFunction),
	(*mut BigInt),
	(*mut Symbol),
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
		unsafe { RootedTraceableSet::remove(&*self.heap) }
		self.heap.set(v);
		unsafe { RootedTraceableSet::add(&*self.heap) };
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
	pub fn to_local(&self) -> Local<'_, T> {
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

impl<T> Clone for TracedHeap<T>
where
	T: GCMethods + Copy + 'static,
	JSHeap<T>: Traceable + Default,
{
	fn clone(&self) -> Self {
		Self::new(self.heap.get())
	}
}

impl_heap_root! {
	[TracedHeap]
	(JSVal),
	(*mut JSObject),
	(*mut JSString),
	(*mut JSScript),
	(PropertyKey),
	(*mut JSFunction),
	(*mut BigInt),
	(*mut Symbol),
}

/// Value stored on the heap and traced permanently. There is
/// no need to trace [PermanentHeap] instances, and thus there
/// is no [Traceable] implementation for this type. This can be
/// considered the rust parallel to PersistentRooted. This type
/// is mainly useful for use in thread statics, since dropping a
/// [TracedHeap] after [RootedTraceableSet] is dropped can cause
/// threads to panic.
#[derive(Debug)]
pub struct PermanentHeap<T>
where
	T: GCMethods + Copy + 'static,
	JSHeap<T>: Traceable,
{
	heap: Box<JSHeap<T>>,
}

impl<T> PermanentHeap<T>
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
}

impl<T> PermanentHeap<T>
where
	T: GCMethods + Copy + RootKind + 'static,
	JSHeap<T>: Traceable + Default,
{
	pub fn from_local(local: &Local<'_, T>) -> Self {
		Self::new(local.get())
	}

	/// This constructs a Local from the Heap directly as opposed to rooting on the stack.
	/// The returned Local cannot be used to construct a HandleMut.
	pub fn to_local(&self) -> Local<'_, T> {
		unsafe { Local::from_heap(&self.heap) }
	}
}

impl_heap_root! {
	[PermanentHeap]
	(JSVal),
	(*mut JSObject),
	(*mut JSString),
	(*mut JSScript),
	(PropertyKey),
	(*mut JSFunction),
	(*mut BigInt),
	(*mut Symbol),
}

pub trait HeapPointer<T> {
	fn to_ptr(&self) -> T;
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

impl<T> HeapPointer<T> for TracedHeap<T>
where
	T: GCMethods + Copy + 'static,
	JSHeap<T>: Traceable + Default,
{
	fn to_ptr(&self) -> T {
		self.heap.get()
	}
}

impl<T> HeapPointer<T> for PermanentHeap<T>
where
	T: GCMethods + Copy + 'static,
	JSHeap<T>: Traceable + Default,
{
	fn to_ptr(&self) -> T {
		self.heap.get()
	}
}
