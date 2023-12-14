use ion::{
	class::Reflector,
	Heap, Result, ClassDefinition, Context, Error, ErrorKind, Object, Value,
	conversions::{ToValue, FromValue},
};
use mozjs::{
	jsapi::{JSObject, JS_NewUint8Array},
	typedarray::ArrayBufferView,
};

use crate::globals::encoding::{
	decoder::{TextDecoderOptions, TextDecodeOptions},
	TextDecoder,
};

use super::{TransformStream, TransformStreamDefaultController};

#[js_class]
pub(super) struct TextDecoderStreamTransformer {
	reflector: Reflector,
	stream: Heap<*mut JSObject>,
}

impl TextDecoderStreamTransformer {
	fn new(stream: &Object) -> Self {
		Self {
			reflector: Default::default(),
			stream: Heap::from_local(&stream),
		}
	}
}

impl TextDecoderStreamTransformer {
	fn transform_chunk(
		&self, cx: &Context, chunk: ArrayBufferView, final_chunk: bool, controller: &TransformStreamDefaultController,
	) -> Result<()> {
		let stream = TextDecoderStream::get_private(&self.stream.root(cx).into());
		let decoder = TextDecoder::get_mut_private(&mut stream.decoder.root(cx).into());
		match decoder.decode(chunk, Some(TextDecodeOptions::new(!final_chunk))) {
			Ok(string) => controller.enqueue(cx, string.as_value(cx)).map_err(|e| e.to_error())?,
			Err(e) => controller.error(cx, e.as_value(cx))?,
		}
		Ok(())
	}
}

#[js_class]
impl TextDecoderStreamTransformer {
	#[ion(constructor)]
	pub fn constructor() -> Result<TextDecoderStreamTransformer> {
		Err(Error::new("Cannot construct this type", ErrorKind::Type))
	}

	pub fn transform(
		&self, cx: &Context, chunk: ArrayBufferView, controller: &TransformStreamDefaultController,
	) -> Result<()> {
		self.transform_chunk(cx, chunk, false, controller)
	}

	pub fn flush(&self, cx: &Context, controller: &TransformStreamDefaultController) -> Result<()> {
		// Transform a final, empty chunk so we detect partial characters at the end of the stream
		let empty_array = Value::object(cx, &cx.root_object(unsafe { JS_NewUint8Array(cx.as_ptr(), 0) }).into());
		self.transform_chunk(
			cx,
			ArrayBufferView::from_value(cx, &empty_array, false, ())
				.expect("ArrayBuffer should turn into ArrayBufferView"),
			true,
			controller,
		)
	}
}

#[js_class]
pub struct TextDecoderStream {
	reflector: Reflector,
	transform_stream: Heap<*mut JSObject>,
	decoder: Heap<*mut JSObject>,
}

impl TextDecoderStream {
	fn transform_stream<'cx>(&self, cx: &'cx Context) -> &'cx TransformStream {
		TransformStream::get_private(&self.transform_stream.root(cx).into())
	}

	fn decoder<'cx>(&self, cx: &'cx Context) -> &'cx TextDecoder {
		TextDecoder::get_private(&self.decoder.root(cx).into())
	}
}

#[js_class]
impl TextDecoderStream {
	#[ion(constructor)]
	pub fn constructor(
		cx: &Context, #[ion(this)] this: &Object, label: Option<String>, options: Option<TextDecoderOptions>,
	) -> Result<TextDecoderStream> {
		let decoder = cx.root_object(TextDecoder::new_object(
			cx,
			Box::new(TextDecoder::constructor(label, options)?),
		));

		let transformer = Object::from(cx.root_object(TextDecoderStreamTransformer::new_object(
			cx,
			Box::new(TextDecoderStreamTransformer::new(this)),
		)));
		let transform_stream = TransformStream::construct(cx, &[transformer.as_value(cx)]).map_err(|e| e.to_error())?;

		Ok(Self {
			reflector: Default::default(),
			transform_stream: Heap::from_local(&transform_stream),
			decoder: Heap::from_local(&decoder),
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
		self.decoder(cx).get_encoding()
	}

	#[ion(get)]
	pub fn get_fatal(&self, cx: &Context) -> bool {
		self.decoder(cx).fatal
	}

	#[ion(get, name = "ignoreBOM")]
	pub fn get_ignore_bom(&self, cx: &Context) -> bool {
		self.decoder(cx).ignore_byte_order_mark
	}
}
