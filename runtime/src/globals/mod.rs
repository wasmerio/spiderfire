/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use ion::{ClassDefinition, Context, Iterator, Object};

pub mod abort;
pub mod base64;
pub mod console;
pub mod encoding;
#[cfg(feature = "fetch")]
pub mod fetch;
pub mod file;
pub mod form_data;
pub mod microtasks;
pub mod streams;
pub mod timers;
pub mod url;

pub fn init_globals(cx: &Context, global: &mut Object) -> bool {
	let result = base64::define(cx, global)
		&& console::define(cx, global)
		&& encoding::define(cx, global)
		&& file::define(cx, global)
		&& form_data::define(cx, global)
		&& url::define(cx, global)
		&& streams::define(cx, global)
		&& Iterator::init_class(cx, global).0;
	#[cfg(feature = "fetch")]
	{
		result && fetch::define(cx, global)
	}
	#[cfg(not(feature = "fetch"))]
	{
		result
	}
}

pub fn init_timers(cx: &Context, global: &mut Object) -> bool {
	timers::define(cx, global) && abort::define(cx, global)
}

pub fn init_microtasks(cx: &Context, global: &mut Object) -> bool {
	microtasks::define(cx, global)
}

#[derive(FromValue)]
pub enum AllowSharedBufferSource {
	#[ion(inherit)]
	ArrayBuffer(mozjs::typedarray::ArrayBuffer),
	#[ion(inherit)]
	ArrayBufferView(mozjs::typedarray::ArrayBufferView),
}

// TODO: put this somewhere that makes sense
impl AllowSharedBufferSource {
	/// Returns the number of elements in the underlying typed array.
	pub fn len(&self) -> usize {
		match self {
			Self::ArrayBuffer(a) => a.len(),
			Self::ArrayBufferView(a) => a.len(),
		}
	}

	/// Retrieves an owned data that's represented by the typed array.
	pub fn to_vec(&self) -> Vec<u8> {
		match self {
			Self::ArrayBuffer(a) => a.to_vec(),
			Self::ArrayBufferView(a) => a.to_vec(),
		}
	}

	/// # Unsafety
	///
	/// The returned slice can be invalidated if the underlying typed array
	/// is neutered.
	pub unsafe fn as_slice(&self) -> &[u8] {
		unsafe {
			match self {
				Self::ArrayBuffer(a) => a.as_slice(),
				Self::ArrayBufferView(a) => a.as_slice(),
			}
		}
	}

	/// # Unsafety
	///
	/// The returned slice can be invalidated if the underlying typed array
	/// is neutered.
	///
	/// The underlying `JSObject` can be aliased, which can lead to
	/// Undefined Behavior due to mutable aliasing.
	pub unsafe fn as_mut_slice(&mut self) -> &mut [u8] {
		unsafe {
			match self {
				Self::ArrayBuffer(a) => a.as_mut_slice(),
				Self::ArrayBufferView(a) => a.as_mut_slice(),
			}
		}
	}

	/// Return a boolean flag which denotes whether the underlying buffer
	/// is a SharedArrayBuffer.
	pub fn is_shared(&self) -> bool {
		match self {
			Self::ArrayBuffer(a) => a.is_shared(),
			Self::ArrayBufferView(a) => a.is_shared(),
		}
	}
}
