/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::ffi::CStr;

use colored::Colorize;
use mozjs::rust::get_object_class;

use crate::{Context, Local, Object};
use crate::format::Config;
use crate::format::object::format_plain_object;

pub fn format_class_object<'c>(cx: &Context<'c>, cfg: Config, object: &Local<'c, Object>) -> String {
	let class = unsafe { get_object_class(***object) };
	let name = unsafe { CStr::from_ptr((*class).name).to_str().unwrap() };

	let string = format_plain_object(cx, cfg, object);
	format!("{} {}", name.color(cfg.colors.object), string)
}
