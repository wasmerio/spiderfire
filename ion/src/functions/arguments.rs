/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::ops::RangeBounds;

use mozjs::jsapi::CallArgs;
use mozjs::jsval::JSVal;

use crate::{Context, Local, Value};

pub struct Arguments<'s> {
	values: Vec<Local<'s, Value>>,
	this: Local<'s, Value>,
	rval: Local<'s, Value>,
	call_args: CallArgs,
}

impl<'s> Arguments<'s> {
	pub unsafe fn new<'c>(cx: &'c Context, argc: u32, vp: *mut JSVal) -> Arguments<'c> {
		let call_args = CallArgs::from_vp(vp, argc);
		let values = (0..(argc + 1)).map(|i| Value::from_raw(cx, call_args.get(i).get())).collect();
		let this = Value::from_raw(cx, call_args.thisv().get());
		let rval = Value::from_raw(cx, call_args.rval().get());

		Arguments { values, this, rval, call_args }
	}

	#[allow(clippy::len_without_is_empty)]
	pub fn len(&self) -> usize {
		self.values.len()
	}

	pub fn get(&self, index: usize) -> Option<&Local<'s, Value>> {
		if self.len() > index + 1 {
			return Some(&self.values[index]);
		}
		None
	}

	pub fn range<R: Iterator<Item = usize> + RangeBounds<usize>>(&self, range: R) -> Vec<&Local<Value>> {
		range.filter_map(|index| self.get(index)).collect()
	}

	pub fn this(&self) -> &Local<'s, Value> {
		&self.this
	}

	pub fn rval(&mut self) -> &mut Local<'s, Value> {
		&mut self.rval
	}

	pub fn is_constructing(&self) -> bool {
		self.call_args.constructing_()
	}
}
