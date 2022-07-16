/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

pub use config::Config;
use object::format_object;
use primitive::format_primitive;

use crate::{Context, Local, Object, Value};

mod array;
mod boxed;
mod class;
mod config;
mod date;
mod function;
mod object;
mod primitive;

pub const INDENT: &str = "  ";
pub const NEWLINE: &str = "\n";

pub fn format_value<'c>(cx: &Context<'c>, cfg: Config, value: &Local<'c, Value>) -> String {
	if !value.is_object() {
		format_primitive(cx, cfg, value)
	} else {
		format_object(cx, cfg, &Object::from_raw(cx, (**value).to_object()))
	}
}
