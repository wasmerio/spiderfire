/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use mozjs::jsapi::JS::{ForOfIterator as FOIterator, ForOfIterator_NonIterableBehavior, RootedObject, RootedValue};
use mozjs_sys::jsgc::{IntoHandle, IntoMutableHandle};

use crate::{Context, Error, ErrorKind, Object, Result, Value};

// Copied from [rust-mozjs](https://github.com/servo/rust-mozjs/blob/master/src/conversions.rs#L619-L642)
pub(crate) struct ForOfIteratorGuard<'a> {
	root: &'a mut FOIterator,
}

impl<'a> ForOfIteratorGuard<'a> {
	pub(crate) fn new(cx: &Context, root: &'a mut FOIterator) -> Self {
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

pub struct ForOfIterator<'cx: 'c + 'i, 'c, 'i: 'g, 'g> {
	cx: &'cx Context<'c>,
	iterator: &'i mut ForOfIteratorGuard<'g>,
	done: bool,
}

impl<'cx: 'c + 'i, 'c, 'i: 'g, 'g> ForOfIterator<'cx, 'c, 'i, 'g> {
	pub fn from_object(cx: &'cx Context<'c>, object: &Object<'i>) -> Result<ForOfIterator<'cx, 'c, 'i, 'g>> {
		let iterator = FOIterator {
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

		Ok(ForOfIterator { cx, iterator, done: false })
	}

	pub fn is_done(&self) -> bool {
		self.done
	}
}

impl<'cx: 'c + 'i, 'c, 'i: 'g, 'g> Iterator for ForOfIterator<'cx, 'c, 'i, 'g> {
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

		(!self.done).then_some(Ok(value))
	}
}
