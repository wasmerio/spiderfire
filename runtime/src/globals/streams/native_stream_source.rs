use ion::{Object, Value, Promise, class::Reflector, Context, ResultExc};

#[js_class]
pub struct NativeStreamSource {
	reflector: Reflector,

	#[ion(no_trace)]
	callbacks: Box<dyn NativeStreamSourceCallbacks>,
}

pub trait NativeStreamSourceCallbacks {
	fn start<'cx>(&self, cx: &'cx Context, controller: Object<'cx>) -> ResultExc<Value<'cx>>;
	fn pull<'cx>(&self, cx: &'cx Context, controller: Object<'cx>) -> ResultExc<Promise>;
	fn cancel<'cx>(&self, cx: &'cx Context, reason: Value) -> ResultExc<Promise>;
}

impl NativeStreamSource {
	pub fn new(callbacks: Box<dyn NativeStreamSourceCallbacks>) -> Self {
		Self { reflector: Default::default(), callbacks }
	}
}

#[js_class]
impl NativeStreamSource {
	#[ion(constructor)]
	pub fn constructor() -> NativeStreamSource {
		panic!("Cannot construct NativeStreamSource from script")
	}

	pub fn start<'cx>(&self, cx: &'cx Context, controller: Object<'cx>) -> ResultExc<Value<'cx>> {
		self.callbacks.start(cx, controller)
	}

	pub fn pull(&self, cx: &Context, controller: Object) -> ResultExc<Promise> {
		self.callbacks.pull(cx, controller)
	}

	pub fn cancel(&self, cx: &Context, reason: Value) -> ResultExc<Promise> {
		self.callbacks.cancel(cx, reason)
	}
}
