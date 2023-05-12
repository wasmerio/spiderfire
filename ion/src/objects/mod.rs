/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use mozjs::jsapi::{JSCLASS_RESERVED_SLOTS_MASK, JSCLASS_RESERVED_SLOTS_SHIFT};

pub use array::Array;
pub use date::Date;
pub use iterator::ForOfIterator;
pub(crate) use iterator::ForOfIteratorGuard;
pub use key::Key;
pub use object::Object;
pub use promise::Promise;

mod array;
mod date;
mod iterator;
mod key;
mod object;
mod promise;
pub mod typedarray;

/// Returns the bitmasked representation of reserved slots for a class
pub const fn class_reserved_slots(slots: u32) -> u32 {
	(slots & JSCLASS_RESERVED_SLOTS_MASK) << JSCLASS_RESERVED_SLOTS_SHIFT
}
