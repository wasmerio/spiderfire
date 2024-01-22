use core::slice;

use bytemuck::cast_slice;
use byteorder::NativeEndian;
use mozjs::jsapi::ToJSON;
use utf16string::WStr;

use crate::{Context, Result, Value, Error, ErrorKind, Object};

pub fn parse(cx: &Context, text: String) -> Result<Object> {
	let Some(str) = crate::String::copy_from_str(cx, text.as_str()) else {
		return Err(Error::new("Failed to allocate string", ErrorKind::Normal));
	};
	let mut result = Value::undefined(cx);
	if !unsafe { mozjs::jsapi::JS_ParseJSON1(cx.as_ptr(), str.handle().into(), result.handle_mut().into()) } {
		return Err(Error::none());
	}

	Ok(result.to_object(cx))
}

pub fn stringify(cx: &Context, value: Value) -> Result<String> {
	let mut string = String::new();
	let replacer = cx.root_object(std::ptr::null_mut());
	let space = Value::undefined(cx);
	if !unsafe {
		ToJSON(
			cx.as_ptr(),
			value.handle().into(),
			replacer.handle().into(),
			space.handle().into(),
			Some(json_write),
			&mut string as *mut String as *mut _,
		)
	} {
		return Err(Error::none());
	}

	Ok(string)
}

unsafe extern "C" fn json_write(buf: *const u16, len: u32, data: *mut ::std::os::raw::c_void) -> bool {
	unsafe {
		let string = &mut *(data as *mut String);
		let slice = slice::from_raw_parts(buf, len as usize);
		let Ok(wstr) = WStr::<NativeEndian>::from_utf16(cast_slice(slice)) else {
			return false;
		};
		string.push_str(wstr.to_utf8().as_str());
		true
	}
}
