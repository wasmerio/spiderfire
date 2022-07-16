/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::cmp::Ordering;

use colored::Colorize;
use mozjs::conversions::jsstr_to_string;
use mozjs::jsapi::ESClass;
use mozjs::rust::Handle;
use mozjs::rust::jsapi_wrapped::JS_ValueToSource;

use crate::{Context, Local, Object};
use crate::flags::IteratorFlags;
use crate::format::{Config, format_value, INDENT, NEWLINE};
use crate::format::array::format_array;
use crate::format::boxed::format_boxed;
use crate::format::class::format_class_object;
use crate::format::date::format_date;
use crate::format::function::format_function;
use crate::functions;
use crate::objects;

pub fn format_object<'c>(cx: &Context<'c>, cfg: Config, object: &Local<'c, Object>) -> String {
	match object.get_builtin_class(cx) {
		Some(class) => {
			use ESClass::*;
			match class {
				Boolean | Number | String | BigInt => format_boxed(cx, cfg, object, class),
				Array => format_array(cx, cfg, &objects::Array::from_raw(cx, ***object).unwrap()),
				Object => format_plain_object(cx, cfg, object),
				Date => format_date(cx, cfg, &objects::Date::from_raw(cx, ***object).unwrap()),
				Function => format_function(cx, cfg, unsafe { &functions::Function::from_object_raw(cx, ***object).unwrap() }),
				Other => format_class_object(cx, cfg, object),
				_ => {
					let value = object.to_value(cx);
					let handle = unsafe { Handle::from_marked_location(&value.val) };
					unsafe { jsstr_to_string(cx.cx(), JS_ValueToSource(cx.cx(), handle)) }
				}
			}
		}
		None => String::from("Format Error: Could Not Get Builtin Class"),
	}
}

pub fn format_plain_object<'c>(cx: &Context<'c>, cfg: Config, object: &Local<'c, Object>) -> String {
	let color = cfg.colors.object;
	if cfg.depth < 4 {
		let keys = object.keys(cx, Some(IteratorFlags::empty()));
		let length = keys.len();

		if length == 0 {
			"{}".color(color).to_string()
		} else if cfg.multiline {
			let mut string = format!("{{{}", NEWLINE).color(color).to_string();

			let inner_indent = INDENT.repeat((cfg.indentation + cfg.depth + 1) as usize);
			let outer_indent = INDENT.repeat((cfg.indentation + cfg.depth) as usize);
			for (i, key) in keys.into_iter().enumerate().take(length) {
				let value = object.get(cx, &key.to_string()).unwrap();
				let value_string = format_value(cx, cfg.depth(cfg.depth + 1).quoted(true), &value);
				string.push_str(&inner_indent);
				string.push_str(&format!("{}: {}", key.to_string().color(color), value_string));

				if i != length - 1 {
					string.push_str(&",".color(color).to_string());
				}
				string.push_str(NEWLINE);
			}

			string.push_str(&outer_indent);
			string.push_str(&"}".color(color).to_string());
			string
		} else {
			let mut string = "{ ".color(color).to_string();
			let len = length.clamp(0, 3);
			for (i, key) in keys.into_iter().enumerate().take(len) {
				let value = object.get(cx, &key.to_string()).unwrap();
				let value_string = format_value(cx, cfg.depth(cfg.depth + 1).quoted(true), &value);
				string.push_str(&format!("{}: {}", key.to_string().color(color), value_string));

				if i != len - 1 {
					string.push_str(&", ".color(color).to_string());
				}
			}

			let remaining = length - len;
			match remaining.cmp(&1) {
				Ordering::Equal => string.push_str(&"... 1 more item ".color(color).to_string()),
				Ordering::Greater => string.push_str(&format!("... {} more items ", remaining).color(color).to_string()),
				_ => (),
			}
			string.push_str(&"}".color(color).to_string());

			string
		}
	} else {
		"[Object]".color(color).to_string()
	}
}
