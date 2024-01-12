use std::any::TypeId;

use as_any::AsAny;
use ion::{Object, Value, Promise, class::Reflector, Context, ResultExc};

#[js_class]
pub struct NativeStreamSource {
	reflector: Reflector,

	#[trace(no_trace)]
	callbacks: Option<Box<dyn NativeStreamSourceCallbacks>>,
}

pub trait NativeStreamSourceCallbacks: AsAny {
	// The source argument is provided mostly so async code in the implementation can call
	// get_typed_source on it inside its future.
	fn start<'cx>(
		&self, source: &'cx NativeStreamSource, cx: &'cx Context, controller: Object<'cx>,
	) -> ResultExc<Value<'cx>>;
	fn pull<'cx>(
		&self, source: &'cx NativeStreamSource, cx: &'cx Context, controller: Object<'cx>,
	) -> ResultExc<Promise>;
	fn cancel<'cx>(self: Box<Self>, cx: &'cx Context, reason: Value) -> ResultExc<Promise>;

	fn name(&self) -> &'static str {
		core::any::type_name::<Self>()
	}

	fn id(&self) -> TypeId {
		TypeId::of::<Self>()
	}
}

impl NativeStreamSource {
	pub fn new(callbacks: Box<dyn NativeStreamSourceCallbacks>) -> Self {
		Self {
			reflector: Default::default(),
			callbacks: Some(callbacks),
		}
	}

	pub fn get_typed_source<T: NativeStreamSourceCallbacks>(&self) -> &T {
		self.callbacks
			.as_ref()
			.expect("Already canceled")
			.as_ref()
			.as_any()
			.downcast_ref::<T>()
			.expect("Callbacks object was not of given type")
	}

	pub fn get_typed_source_mut<T: NativeStreamSourceCallbacks>(&mut self) -> &mut T {
		self.callbacks
			.as_mut()
			.expect("Already canceled")
			.as_mut()
			.as_any_mut()
			.downcast_mut::<T>()
			.expect("Callbacks object was not of given type")
	}
}

#[js_class]
impl NativeStreamSource {
	#[ion(constructor)]
	pub fn constructor() -> NativeStreamSource {
		panic!("Cannot construct NativeStreamSource from script")
	}

	pub fn start<'cx>(&'cx self, cx: &'cx Context, controller: Object<'cx>) -> ResultExc<Value<'cx>> {
		self.callbacks.as_ref().expect("start called after cancel").start(self, cx, controller)
	}

	pub fn pull(&self, cx: &Context, controller: Object) -> ResultExc<Promise> {
		self.callbacks.as_ref().expect("start called after cancel").pull(self, cx, controller)
	}

	pub fn cancel(&mut self, cx: &Context, reason: Value) -> ResultExc<Promise> {
		let b: Box<_> = self.callbacks.take().expect("cancel called more than once");
		b.cancel(cx, reason)
	}
}
