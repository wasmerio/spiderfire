use ion::{Context, Object, ClassDefinition};

mod native_stream_sink;
mod native_stream_source;
mod readable_stream_extensions;
mod transform_stream;

pub use native_stream_sink::{NativeStreamSink, NativeStreamSinkCallbacks};
pub use native_stream_source::{NativeStreamSource, NativeStreamSourceCallbacks};

pub fn define(cx: &Context, global: &mut Object) -> bool {
	readable_stream_extensions::define(cx, global)
		&& native_stream_sink::NativeStreamSink::init_class(cx, global).0
		&& native_stream_source::NativeStreamSource::init_class(cx, global).0
		&& transform_stream::TransformStream::init_class(cx, global).0
		&& transform_stream::TransformStreamDefaultController::init_class(cx, global).0
}
