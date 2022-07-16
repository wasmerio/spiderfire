/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use colored::Colorize;
use mozjs::conversions::jsstr_to_string;

use crate::{Context, Local, Value};
use crate::format::Config;

pub fn format_primitive<'c>(cx: &Context<'c>, cfg: Config, value: &Local<'c, Value>) -> String {
	if value.is_boolean() {
		value.to_boolean().to_string().color(cfg.colors.boolean).to_string()
	} else if value.is_number() {
		let number = value.to_number();

		if number == f64::INFINITY {
			"Infinity".color(cfg.colors.number).to_string()
		} else if number == f64::NEG_INFINITY {
			"-Infinity".color(cfg.colors.number).to_string()
		} else {
			number.to_string().color(cfg.colors.number).to_string()
		}
	} else if value.is_string() {
		let str = unsafe { jsstr_to_string(cx.cx(), value.to_jsstr()) };
		if cfg.quoted {
			format!(r#""{}""#, str).color(cfg.colors.string).to_string()
		} else {
			str
		}
	} else if value.is_null() {
		"null".color(cfg.colors.null).to_string()
	} else if value.is_undefined() {
		"undefined".color(cfg.colors.undefined).to_string()
	} else {
		unreachable!("Internal Error: Expected Primitive")
	}
}
