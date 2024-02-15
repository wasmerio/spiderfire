use ion::{
	class::Reflector, conversions::ToValue, function::Opt, ClassDefinition, Context, Error, ErrorKind, Heap, Object,
	Result, Value,
};
use mozjs::jsapi::{JSObject, ToStringSlow};

use crate::globals::encoding::TextEncoder;

use super::{TransformStream, TransformStreamDefaultController};

#[js_class]
pub(super) struct TextEncoderStreamTransformer {
	reflector: Reflector,
	stream: Heap<*mut JSObject>,
}

impl TextEncoderStreamTransformer {
	fn new(stream: &Object) -> Self {
		Self {
			reflector: Default::default(),
			stream: Heap::from_local(stream),
		}
	}
}

impl TextEncoderStreamTransformer {
	fn transform_chunk(&self, cx: &Context, chunk: Value, controller: &TransformStreamDefaultController) -> Result<()> {
		let stream = TextEncoderStream::get_private(cx, &self.stream.root(cx).into()).unwrap();
		let encoder = TextEncoder::get_mut_private(cx, &stream.encoder.root(cx).into()).unwrap();
		let chunk_str = unsafe { ToStringSlow(cx.as_ptr(), chunk.handle().into()) };
		if chunk_str.is_null() {
			return Err(Error::none());
		}
		let chunk_str = ion::String::from(cx.root(chunk_str)).to_owned(cx)?;
		controller
			.enqueue(cx, encoder.encode(cx, Opt(Some(chunk_str)))?.as_value(cx))
			.map_err(|e| e.to_error())?;
		Ok(())
	}
}

#[js_class]
impl TextEncoderStreamTransformer {
	#[ion(constructor)]
	pub fn constructor() -> Result<TextEncoderStreamTransformer> {
		Err(Error::new("Cannot construct this type", ErrorKind::Type))
	}

	pub fn transform(&self, cx: &Context, chunk: Value, controller: &TransformStreamDefaultController) -> Result<()> {
		self.transform_chunk(cx, chunk, controller)
	}

	pub fn flush(&self, _cx: &Context, _controller: &TransformStreamDefaultController) -> Result<()> {
		Ok(())
	}
}

#[js_class]
pub struct TextEncoderStream {
	reflector: Reflector,
	transform_stream: Heap<*mut JSObject>,
	encoder: Heap<*mut JSObject>,
}

impl TextEncoderStream {
	fn transform_stream<'cx>(&self, cx: &'cx Context) -> &'cx TransformStream {
		TransformStream::get_private(cx, &self.transform_stream.root(cx).into()).unwrap()
	}

	fn encoder<'cx>(&self, cx: &'cx Context) -> &'cx TextEncoder {
		TextEncoder::get_private(cx, &self.encoder.root(cx).into()).unwrap()
	}
}

#[js_class]
impl TextEncoderStream {
	#[ion(constructor)]
	pub fn constructor(cx: &Context, #[ion(this)] this: &Object) -> Result<TextEncoderStream> {
		let encoder = cx.root(TextEncoder::new_object(cx, Box::new(TextEncoder::constructor())));

		let transformer = Object::from(cx.root(TextEncoderStreamTransformer::new_object(
			cx,
			Box::new(TextEncoderStreamTransformer::new(this)),
		)));
		let transform_stream = TransformStream::construct(cx, &[transformer.as_value(cx)]).map_err(|e| e.to_error())?;

		Ok(Self {
			reflector: Default::default(),
			transform_stream: Heap::from_local(&transform_stream),
			encoder: Heap::from_local(&encoder),
		})
	}

	#[ion(get)]
	pub fn get_readable(&self, cx: &Context) -> *mut JSObject {
		self.transform_stream(cx).get_readable()
	}

	#[ion(get)]
	pub fn get_writable(&self, cx: &Context) -> *mut JSObject {
		self.transform_stream(cx).get_writable()
	}

	#[ion(get)]
	pub fn get_encoding(&self, cx: &Context) -> String {
		self.encoder(cx).get_encoding()
	}
}
