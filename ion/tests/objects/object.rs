use std::ptr;
use mozjs::jsapi::{JS_NewGlobalObject, JSAutoRealm, OnNewGlobalHookOption};
use mozjs::rust::{Handle, JSEngine, RealmOptions, Runtime, SIMPLE_GLOBAL_CLASS};
use mozjs::rust::jsapi_wrapped::JS_ValueToSource;
use mozjs::conversions::jsstr_to_string;
use ion::{Context, Object, Value};
use ion::flags::PropertyFlags;

#[test]
fn object() {
	let engine = JSEngine::init().unwrap();
	let runtime = Runtime::new(engine.handle());
	let h_options = OnNewGlobalHookOption::FireOnNewGlobalHook;
	let c_options = RealmOptions::default();

	let global = unsafe { JS_NewGlobalObject(runtime.cx(), &SIMPLE_GLOBAL_CLASS, ptr::null_mut(), h_options, &*c_options) };
	let _realm = JSAutoRealm::new(runtime.cx(), global);

	let mut cx = runtime.cx();
	let cx = Context::new(&mut cx);

	let mut object = Object::new(&cx);
	object.set(&cx, "key1", Value::null(&cx));
	object.define(&cx, "key2", Value::undefined(&cx), PropertyFlags::all());
	let value1 = **object.get(&cx, "key1").unwrap();
	let value2 = **object.get(&cx, "key2").unwrap();
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

	let keys = object.keys(&cx, None);
	println!("{:?}", keys);

	assert!(object.delete(&cx, "key1").0);
	assert!(object.delete(&cx, "key2").0);
	assert!(object.get(&cx, "key1").is_none());
	assert!(object.get(&cx, "key2").is_some());
}
