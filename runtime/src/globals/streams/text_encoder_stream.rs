use ion::{class::Reflector, Heap, Result, ClassDefinition, Context, Error, ErrorKind, Object, conversions::ToValue};
use mozjs::jsapi::JSObject;

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
			stream: Heap::from_local(&stream),
		}
	}
}

// TODO: since we're transforming rust `String`s, we don't handle
// surrogate pairs correctly. If a UTF-16 low surrogate character
// comes in without its corresponding high surrogate character,
// rust's `String` simply refuses the input. Correctly handling
// this will be hard, since we'll have to directly work on the
// UTF-16 data from Spidermonkey.
impl TextEncoderStreamTransformer {
	fn transform_chunk(
		&self, cx: &Context, chunk: String, controller: &TransformStreamDefaultController,
	) -> Result<()> {
		let stream = TextEncoderStream::get_private(&self.stream.root(cx).into());
		let encoder = TextEncoder::get_mut_private(&mut stream.encoder.root(cx).into());
		controller
			.enqueue(cx, encoder.encode(Some(chunk)).as_value(cx))
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

	pub fn transform(&self, cx: &Context, chunk: String, controller: &TransformStreamDefaultController) -> Result<()> {
		self.transform_chunk(cx, chunk, controller)
	}

	pub fn flush(&self, _cx: &Context, _controller: &TransformStreamDefaultController) -> Result<()> {
		// TODO: implement UTF-16 surrogate pair handling correctly, see comment above
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
		TransformStream::get_private(&self.transform_stream.root(cx).into())
	}

	fn encoder<'cx>(&self, cx: &'cx Context) -> &'cx TextEncoder {
		TextEncoder::get_private(&self.encoder.root(cx).into())
	}
}

#[js_class]
impl TextEncoderStream {
	#[ion(constructor)]
	pub fn constructor(cx: &Context, #[ion(this)] this: &Object) -> Result<TextEncoderStream> {
		let encoder = cx.root_object(TextEncoder::new_object(cx, Box::new(TextEncoder::constructor())));

		let transformer = Object::from(cx.root_object(TextEncoderStreamTransformer::new_object(
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
