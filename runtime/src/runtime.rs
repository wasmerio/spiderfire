/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::any::Any;
use std::ptr;

use mozjs::glue::CreateJobQueue;
use mozjs::jsapi::{ContextOptionsRef, JSAutoRealm, SetJobQueue, SetPromiseRejectionTrackerCallback, OnNewGlobalHookOption};

use ion::{Context, ErrorReport, Object};
use ion::module::{init_module_loader, ModuleLoader};
use ion::object::new_global;
use mozjs::rust::{RealmOptions, SIMPLE_GLOBAL_CLASS};

use crate::event_loop::{EventLoop, promise_rejection_tracker_callback};
use crate::event_loop::future::FutureQueue;
use crate::event_loop::macrotasks::MacrotaskQueue;
use crate::event_loop::microtasks::{JOB_QUEUE_TRAPS, MicrotaskQueue};
use crate::globals::{init_globals, init_microtasks, init_timers};
use crate::module::StandardModules;

#[derive(Default)]
pub struct ContextPrivate {
	pub(crate) event_loop: EventLoop,
	pub app_data: Option<Box<dyn Any>>,
}

pub trait ContextExt {
	#[allow(clippy::mut_from_ref)]
	unsafe fn get_private(&self) -> &mut ContextPrivate;

	fn set_app_data(&self, app_data: Box<dyn Any>);

	fn get_raw_app_data(&self) -> *mut dyn Any;

	#[allow(clippy::mut_from_ref)]
	unsafe fn get_app_data<T: 'static>(&self) -> &mut T;
}

impl ContextExt for Context {
	unsafe fn get_private(&self) -> &mut ContextPrivate {
		unsafe { (*self.get_raw_private()).downcast_mut().unwrap() }
	}

	fn set_app_data(&self, app_data: Box<dyn Any>) {
		unsafe { self.get_private() }.app_data = Some(app_data);
	}

	fn get_raw_app_data(&self) -> *mut dyn Any {
		unsafe { ptr::from_mut(self.get_private().app_data.as_deref_mut().unwrap()) }
	}

	//
	unsafe fn get_app_data<T: 'static>(&self) -> &mut T {
		unsafe { (*self.get_raw_app_data()).downcast_mut().unwrap() }
	}
}

pub struct Runtime<'cx> {
	global: Object<'cx>,
	cx: &'cx Context,
	#[allow(dead_code)]
	realm: JSAutoRealm,
}

impl<'cx> Runtime<'cx> {
	pub fn cx(&self) -> &Context {
		self.cx
	}

	pub fn global(&self) -> &Object<'cx> {
		&self.global
	}

	pub fn global_mut(&mut self) -> &Object<'cx> {
		&mut self.global
	}

	pub async fn run_event_loop(&self) -> Result<(), Option<ErrorReport>> {
		let event_loop = unsafe { &mut self.cx.get_private().event_loop };
		let cx = self.cx.duplicate();
		event_loop.run_event_loop(&cx).await
	}
}

impl Drop for Runtime<'_> {
	fn drop(&mut self) {
		let inner_private = self.cx.get_inner_data();
		let _ = unsafe { Box::from_raw(inner_private.as_ptr()) };
	}
}

pub struct RuntimeBuilder<ML: ModuleLoader + 'static = (), Std: StandardModules + 'static = ()> {
	microtask_queue: bool,
	macrotask_queue: bool,
	modules: Option<ML>,
	standard_modules: Option<Std>,
	hook_option: Option<OnNewGlobalHookOption>,
	realm_options: Option<RealmOptions>,
}

impl<ML: ModuleLoader + 'static, Std: StandardModules + 'static> RuntimeBuilder<ML, Std> {
	pub fn new() -> RuntimeBuilder<ML, Std> {
		RuntimeBuilder::default()
	}

	pub fn macrotask_queue(mut self) -> RuntimeBuilder<ML, Std> {
		self.macrotask_queue = true;
		self
	}

	pub fn microtask_queue(mut self) -> RuntimeBuilder<ML, Std> {
		self.microtask_queue = true;
		self
	}

	pub fn modules(mut self, loader: ML) -> RuntimeBuilder<ML, Std> {
		self.modules = Some(loader);
		self
	}

	pub fn standard_modules(mut self, standard_modules: Std) -> RuntimeBuilder<ML, Std> {
		self.standard_modules = Some(standard_modules);
		self
	}

	pub fn hook_option(mut self, hook_option: OnNewGlobalHookOption) -> RuntimeBuilder<ML, Std> {
		self.hook_option = Some(hook_option);
		self
	}

	pub fn realm_options(mut self, realm_options: RealmOptions) -> RuntimeBuilder<ML, Std> {
		self.realm_options = Some(realm_options);
		self
	}

	pub fn build(self, cx: &Context) -> Runtime {
		let global = new_global(
			cx,
			&SIMPLE_GLOBAL_CLASS,
			None,
			self.hook_option.unwrap_or(OnNewGlobalHookOption::FireOnNewGlobalHook),
			self.realm_options,
		);
		let realm = JSAutoRealm::new(cx.as_ptr(), global.handle().get());

		let global_obj = global.handle().get();
		global.set_as(cx, "global", &global_obj);
		init_globals(cx, &global);

		let mut private = Box::<ContextPrivate>::default();

		if self.microtask_queue {
			private.event_loop.microtasks = Some(MicrotaskQueue::default());
			init_microtasks(cx, &global);
			private.event_loop.futures = Some(FutureQueue::default());

			unsafe {
				SetJobQueue(
					cx.as_ptr(),
					CreateJobQueue(
						&JOB_QUEUE_TRAPS,
						ptr::from_ref(private.event_loop.microtasks.as_ref().unwrap()).cast(),
					),
				);
				SetPromiseRejectionTrackerCallback(
					cx.as_ptr(),
					Some(promise_rejection_tracker_callback),
					ptr::null_mut(),
				);
			}
		}
		if self.macrotask_queue {
			private.event_loop.macrotasks = Some(MacrotaskQueue::default());
			init_timers(cx, &global);
		}

		let _options = unsafe { &mut *ContextOptionsRef(cx.as_ptr()) };

		cx.set_private(private);

		let has_loader = self.modules.is_some();
		if let Some(loader) = self.modules {
			init_module_loader(cx, loader);
		}

		if let Some(standard_modules) = self.standard_modules {
			if has_loader {
				standard_modules.init(cx, &global);
			} else {
				standard_modules.init_globals(cx, &global);
			}
		}

		Runtime { global, cx, realm }
	}
}

impl<ML: ModuleLoader + 'static, Std: StandardModules + 'static> Default for RuntimeBuilder<ML, Std> {
	fn default() -> RuntimeBuilder<ML, Std> {
		RuntimeBuilder {
			microtask_queue: false,
			macrotask_queue: false,
			modules: None,
			standard_modules: None,
			hook_option: None,
			realm_options: None,
		}
	}
}
