use core::slice;

use bytemuck::cast_slice;
use byteorder::NativeEndian;
use mozjs::jsapi::{JS_ParseJSON1, ToJSON};
use mozjs_sys::jsapi::JSObject;
use utf16string::WStr;

use crate::{Context, Error, ErrorKind, Exception, Object, ResultExc, Value};

pub fn parse(cx: &Context, text: String) -> ResultExc<Object> {
	let Some(str) = crate::String::copy_from_str(cx, text.as_str()) else {
		return Err(Error::new("Failed to allocate string", ErrorKind::Normal).into());
	};
	let mut result = Value::undefined(cx);
	if !unsafe { JS_ParseJSON1(cx.as_ptr(), str.handle().into(), result.handle_mut().into()) } {
		let exc = Exception::new(cx)?;
		return Err(exc.unwrap_or_else(|| Error::new("Failed to parse JSON", ErrorKind::Normal).into()));
	}

	Ok(result.to_object(cx))
}

pub fn stringify(cx: &Context, value: Value) -> ResultExc<String> {
	let mut string = String::new();
	let replacer = cx.root::<*mut JSObject>(std::ptr::null_mut());
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
		let exc = Exception::new(cx)?;
		return Err(exc.unwrap_or_else(|| Error::new("Failed to stringify JSON", ErrorKind::Normal).into()));
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
