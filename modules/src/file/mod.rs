use ion::{Context, Object, ClassDefinition};

pub mod blob;

pub fn define(cx: &Context, global: &mut Object) -> bool {
	blob::Blob::init_class(cx, global).0
}
