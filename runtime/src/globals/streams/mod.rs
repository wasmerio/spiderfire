use ion::{Context, Object, ClassDefinition};

mod native_stream_sink;
mod native_stream_source;
mod readable_stream_extensions;
mod text_decoder_stream;
mod text_encoder_stream;
mod transform_stream;

pub use native_stream_sink::{NativeStreamSink, NativeStreamSinkCallbacks};
pub use native_stream_source::{NativeStreamSource, NativeStreamSourceCallbacks};
pub use readable_stream_extensions::readable_stream_from_callbacks;
pub use text_decoder_stream::TextDecoderStream;
pub use text_encoder_stream::TextEncoderStream;
pub use transform_stream::{TransformStream, TransformStreamDefaultController};

pub fn define(cx: &Context, global: &Object) -> bool {
	readable_stream_extensions::define(cx, global)
		&& native_stream_sink::NativeStreamSink::init_class(cx, global).0
		&& native_stream_source::NativeStreamSource::init_class(cx, global).0
		&& transform_stream::TransformStream::init_class(cx, global).0
		&& transform_stream::TransformStreamDefaultController::init_class(cx, global).0
		&& text_encoder_stream::TextEncoderStream::init_class(cx, global).0
		&& text_encoder_stream::TextEncoderStreamTransformer::init_class(cx, global).0
		&& text_decoder_stream::TextDecoderStream::init_class(cx, global).0
		&& text_decoder_stream::TextDecoderStreamTransformer::init_class(cx, global).0
}
