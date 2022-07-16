use std::ptr;
use mozjs::jsapi::{JS_NewGlobalObject, JSAutoRealm, OnNewGlobalHookOption};
use mozjs::rust::{Handle, JSEngine, RealmOptions, Runtime, SIMPLE_GLOBAL_CLASS};
use mozjs::rust::jsapi_wrapped::JS_ValueToSource;
use mozjs::conversions::jsstr_to_string;
use ion::{Context, Array, Value};
use ion::flags::PropertyFlags;

#[test]
fn array() {
	let engine = JSEngine::init().unwrap();
	let runtime = Runtime::new(engine.handle());
	let h_options = OnNewGlobalHookOption::FireOnNewGlobalHook;
	let c_options = RealmOptions::default();

	let global = unsafe { JS_NewGlobalObject(runtime.cx(), &SIMPLE_GLOBAL_CLASS, ptr::null_mut(), h_options, &*c_options) };
	let _realm = JSAutoRealm::new(runtime.cx(), global);

	let mut cx = runtime.cx();
	let cx = Context::new(&mut cx);

	let mut array = Array::new(&cx);
	array.set(&cx, 0, Value::null(&cx));
	array.define(&cx, 2, Value::undefined(&cx), PropertyFlags::all());
	let value1 = **array.get(&cx, 0).unwrap();
	let value2 = **array.get(&cx, 2).unwrap();
	unsafe {
		println!(
			"Value 1: {}",
			jsstr_to_string(cx.cx(), JS_ValueToSource(cx.cx(), Handle::from_marked_location(&value1)))
		);
		println!(
			"Value 2: {}",
			jsstr_to_string(cx.cx(), JS_ValueToSource(cx.cx(), Handle::from_marked_location(&value2)))
		);
	}

	assert!(array.delete(&cx, 0).0);
	assert!(array.delete(&cx, 2).0);
	assert!(array.get(&cx, 0).is_none());
	assert!(array.get(&cx, 2).is_some());
}
