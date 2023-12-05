use ion::{Context, Object};

mod readable_stream_extensions;

pub fn define(cx: &Context, global: &mut Object) -> bool {
	readable_stream_extensions::define(cx, global)
}
