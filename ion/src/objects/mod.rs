/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::fmt::{Display, Formatter};

use mozjs::jsapi::{JSCLASS_RESERVED_SLOTS_MASK, JSCLASS_RESERVED_SLOTS_SHIFT};

pub use array::Array;
pub use date::Date;
pub use object::Object;
pub use promise::Promise;

mod array;
mod date;
mod object;
mod promise;

pub const fn class_reserved_slots(slots: u32) -> u32 {
	(slots & JSCLASS_RESERVED_SLOTS_MASK) << JSCLASS_RESERVED_SLOTS_SHIFT
}

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub enum Key {
	Int(i32),
	String(String),
	Void,
}

impl Display for Key {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			&Key::Int(ref int) => f.write_str(&int.to_string()),
			&Key::String(ref string) => f.write_str(string),
			&Key::Void => panic!("Cannot convert void key into string."),
		}
	}
}
