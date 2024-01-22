use ion::{Object, Value, Promise, class::Reflector, Context, ResultExc};

#[js_class]
pub struct NativeStreamSink {
	reflector: Reflector,

	#[trace(no_trace)]
	callbacks: Box<dyn NativeStreamSinkCallbacks>,
}

pub trait NativeStreamSinkCallbacks {
	fn start<'cx>(&self, cx: &'cx Context, controller: Object<'cx>) -> ResultExc<Value<'cx>>;
	fn write(&self, cx: &Context, chunk: Value, controller: Object) -> ResultExc<Promise>;
	fn close(&self, cx: &Context) -> ResultExc<Promise>;
	fn abort(&self, cx: &Context, reason: Value) -> ResultExc<Promise>;
}

impl NativeStreamSink {
	pub fn new(callbacks: Box<dyn NativeStreamSinkCallbacks>) -> Self {
		Self { reflector: Default::default(), callbacks }
	}
}

#[js_class]
impl NativeStreamSink {
	#[ion(constructor)]
	pub fn constructor() -> NativeStreamSink {
		panic!("Cannot construct NativeStreamSink from script")
	}

	pub fn start<'cx>(&self, cx: &'cx Context, controller: Object<'cx>) -> ResultExc<Value<'cx>> {
		self.callbacks.start(cx, controller)
	}

	pub fn write(&self, cx: &Context, chunk: Value, controller: Object) -> ResultExc<Promise> {
		self.callbacks.write(cx, chunk, controller)
	}

	pub fn close(&self, cx: &Context) -> ResultExc<Promise> {
		self.callbacks.close(cx)
	}

	pub fn abort(&self, cx: &Context, reason: Value) -> ResultExc<Promise> {
		self.callbacks.abort(cx, reason)
	}
}
