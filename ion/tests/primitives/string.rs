use std::{ptr, string};
use mozjs::rust::{JSEngine, RealmOptions, Runtime, SIMPLE_GLOBAL_CLASS};
use mozjs::jsapi::{JS_NewGlobalObject, JSAutoRealm, OnNewGlobalHookOption};
use ion::{Context, String};

const STRING_1: &'static str = "Hello ";
const STRING_2: &'static str = "World!";

#[test]
fn string() {
	let engine = JSEngine::init().unwrap();
	let runtime = Runtime::new(engine.handle());
	let h_options = OnNewGlobalHookOption::FireOnNewGlobalHook;
	let c_options = RealmOptions::default();

	let global = unsafe { JS_NewGlobalObject(runtime.cx(), &SIMPLE_GLOBAL_CLASS, ptr::null_mut(), h_options, &*c_options) };
	let _realm = JSAutoRealm::new(runtime.cx(), global);

	let mut cx = runtime.cx();
	let cx = Context::new(&mut cx);

	let empty = String::new(&cx);
	let string1 = String::from_str(&cx, STRING_1);
	let string2 = String::from_str(&cx, STRING_2);

	assert_eq!(0, empty.len());
	assert_eq!(STRING_1.len(), string1.len());
	assert_eq!(STRING_2.len(), string2.len());

	assert_eq!(string::String::from(""), empty.to_string(&cx));
	assert_eq!(string::String::from(STRING_1), string1.to_string(&cx));
	assert_eq!(string::String::from(STRING_2), string2.to_string(&cx));

	STRING_1.chars().enumerate().for_each(|(i, c)| {
		assert_eq!(Some(c), string1.char_at(&cx, i), "String 1: Index {}", i);
	});
	STRING_2.chars().enumerate().for_each(|(i, c)| {
		assert_eq!(Some(c), string2.char_at(&cx, i), "String 2: Index {}", i);
	});

	let concat = string1.concat(&cx, string2);
	assert_eq!(STRING_1.len() + STRING_2.len(), concat.len());
	assert_eq!(string::String::from(STRING_1) + STRING_2, concat.to_string(&cx));
}
