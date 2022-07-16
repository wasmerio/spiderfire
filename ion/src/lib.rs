/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate mozjs;

use std::result::Result as Result2;

pub use context::{Context, Local};
pub use error::Error;
pub use ion_proc::*;
pub use objects::*;
pub use primitives::*;
pub use value::Value;

pub mod context;
pub mod error;
pub mod exception;
pub mod flags;
pub mod format;
pub mod functions;
pub mod objects;
pub mod primitives;
pub mod spec;
pub mod value;

pub type Result<T> = Result2<T, Error>;
