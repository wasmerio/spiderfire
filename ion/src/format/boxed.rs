/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use mozjs::jsapi::{ESClass, Unbox};
use mozjs::rust::{Handle, MutableHandle};

use crate::{Context, Local, Object, Value};
use crate::format::Config;
use crate::format::primitive::format_primitive;

pub fn format_boxed<'c>(cx: &Context<'c>, cfg: Config, object: &Local<'c, Object>, class: ESClass) -> String {
	let handle = unsafe { Handle::from_marked_location(&object.obj) };
	let mut unboxed = Value::undefined(cx);
	let unboxed_handle = unsafe { MutableHandle::from_marked_location(&mut unboxed.val) };

	unsafe {
		if Unbox(cx.cx(), handle.into(), unboxed_handle.into()) {
			use ESClass::*;
			match class {
				Boolean | Number | String => format_primitive(cx, cfg, &unboxed),
				BigInt => format!("Unimplemented Formatting: {}", "BigInt"),
				_ => unreachable!(),
			}
		} else {
			String::from("Internal Error: Unbox Failure")
		}
	}
}
