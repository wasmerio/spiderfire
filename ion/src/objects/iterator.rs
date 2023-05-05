/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::iter;

use mozjs::jsapi::JS::{ForOfIterator, ForOfIterator_NonIterableBehavior, RootedObject, RootedValue};
use mozjs_sys::jsgc::{IntoHandle, IntoMutableHandle};

use crate::{Context, Error, ErrorKind, Object, Result, Value};

// Copied from [rust-mozjs](https://github.com/servo/rust-mozjs/blob/master/src/conversions.rs#L619-L642)
pub(crate) struct ForOfIteratorGuard<'a> {
	root: &'a mut ForOfIterator,
}

impl<'a> ForOfIteratorGuard<'a> {
	pub(crate) fn new(cx: &Context, root: &'a mut ForOfIterator) -> Self {
		unsafe {
			root.iterator.add_to_root_stack(**cx);
		}
		ForOfIteratorGuard { root }
	}
}

impl<'a> Drop for ForOfIteratorGuard<'a> {
	fn drop(&mut self) {
		unsafe {
			self.root.iterator.remove_from_root_stack();
		}
	}
}

pub struct Iterator<'cx: 'c + 'i, 'c, 'i: 'g, 'g> {
	cx: &'cx Context<'c>,
	iterator: &'i mut ForOfIteratorGuard<'g>,
	done: bool,
}

impl<'cx: 'c + 'i, 'c, 'i: 'g, 'g> Iterator<'cx, 'c, 'i, 'g> {
	pub fn from_object(cx: &'cx Context<'c>, object: &Object<'i>) -> Result<Iterator<'cx, 'c, 'i, 'g>> {
		let iterator = ForOfIterator {
			cx_: **cx,
			iterator: RootedObject::new_unrooted(),
			nextMethod: RootedValue::new_unrooted(),
			index: u32::MAX, // NOT_ARRAY
		};
		let iterator = cx.root_iterator(iterator);

		unsafe {
			let value = Value::object(cx, object);
			if !iterator
				.root
				.init(value.handle().into_handle(), ForOfIterator_NonIterableBehavior::ThrowOnNonIterable)
			{
				return Err(Error::new("Failed to Initialise Iterator", ErrorKind::Type));
			}

			if iterator.root.iterator.ptr.is_null() {
				return Err(Error::new("Expected Iterable", ErrorKind::Type));
			}
		}

		Ok(Iterator { cx, iterator, done: false })
	}

	pub fn is_done(&self) -> bool {
		self.done
	}
}

impl<'cx: 'c + 'i, 'c, 'i: 'g, 'g> iter::Iterator for Iterator<'cx, 'c, 'i, 'g> {
	type Item = Result<Value<'cx>>;

	fn next(&mut self) -> Option<Result<Value<'cx>>> {
		if self.done {
			return None;
		}

		let mut value = Value::undefined(self.cx);
		unsafe {
			if !self.iterator.root.next(value.handle_mut().into_handle_mut(), &mut self.done) {
				return Some(Err(Error::new("Failed to Execute Next on Iterator", ErrorKind::Normal)));
			}
		}
		Some(Ok(value))
	}
}
