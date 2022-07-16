/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::ops::{Deref, DerefMut};

use mozjs::jsapi::{JSContext, Rooted};
use mozjs::rust::{Handle, MutableHandle};
use mozjs_sys::jsgc::{GCMethods, RootKind};

pub struct Context<'s> {
	pub(crate) cx: &'s mut *mut JSContext,
}

impl<'s> Context<'s> {
	pub fn new(cx: &mut *mut JSContext) -> Context {
		Context { cx }
	}

	pub fn cx(&self) -> *mut JSContext {
		(*self.cx).clone()
	}
}

pub struct Local<'c, T: 'c + RootKind + GCMethods> {
	root: &'c mut Rooted<T>,
}

impl<'c, T: 'c + RootKind + GCMethods> Local<'c, T> {
	pub fn new(cx: &Context<'c>, initial: T) -> Local<'c, T> {
		let mut root: &'c mut Rooted<T> = Box::leak(Box::new(Rooted::new_unrooted()));
		root.ptr = initial;
		unsafe {
			root.add_to_root_stack(cx.cx());
		}
		Local { root }
	}

	pub fn handle(&'c self) -> Handle<'c, T> {
		unsafe { Handle::from_marked_location(&self.root.ptr) }
	}

	pub fn handle_mut(&'c mut self) -> MutableHandle<'c, T> {
		unsafe { MutableHandle::from_marked_location(&mut self.root.ptr) }
	}
}

impl<'c, T: 'c + Clone + RootKind + GCMethods> Local<'c, T> {
	pub fn set(&'c mut self, other: Local<'c, T>) {
		self.root.ptr = other.root.ptr.clone();
	}
}

impl<'c, T: 'c + RootKind + GCMethods> Deref for Local<'c, T> {
	type Target = T;

	fn deref(&self) -> &Self::Target {
		&self.root.ptr
	}
}

impl<'c, T: 'c + RootKind + GCMethods> DerefMut for Local<'c, T> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.root.ptr
	}
}

impl<'c, T: 'c + RootKind + GCMethods> Drop for Local<'c, T> {
	fn drop(&mut self) {
		unsafe {
			self.root.ptr = T::initial();
			self.root.remove_from_root_stack();
		}
	}
}
