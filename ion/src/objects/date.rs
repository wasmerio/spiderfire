/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::ops::{Deref, DerefMut};

use chrono::{DateTime, TimeZone, Utc};
use mozjs::error::throw_type_error;
use mozjs::jsapi::{AssertSameCompartment, ClippedTime, JS, JSObject, NewDateObject};
use mozjs::jsval::ObjectValue;
use mozjs::rust::Handle;
use mozjs::rust::jsapi_wrapped::{DateGetMsecSinceEpoch, DateIsValid, ObjectIsDate};
use mozjs_sys::jsgc::{GCMethods, RootKind};

use crate::{Context, Local, Value};
use crate::value::{FromValue, ToValue};

#[derive(Clone, Debug)]
pub struct Date {
	pub(crate) date: *mut JSObject,
}

impl Date {
	pub fn new<'c>(cx: &Context<'c>) -> Local<'c, Date> {
		Date::from_date(cx, Utc::now())
	}

	pub fn from_date<'c>(cx: &Context<'c>, date: DateTime<Utc>) -> Local<'c, Date> {
		let date = unsafe { NewDateObject(cx.cx(), ClippedTime { t: date.timestamp_millis() as f64 }) };
		Local::new(cx, Date { date })
	}

	pub(crate) fn from_raw<'c>(cx: &Context<'c>, date: *mut JSObject) -> Option<Local<'c, Date>> {
		if unsafe { Date::is_date_raw(cx, date) } {
			Some(Local::new(cx, Date { date }))
		} else {
			None
		}
	}

	pub fn to_value<'c>(&self, cx: &Context<'c>) -> Local<'c, Value> {
		Value::from_raw(cx, ObjectValue(self.date))
	}

	pub fn to_date(&self, cx: &Context) -> Option<DateTime<Utc>> {
		let handle = unsafe { Handle::from_marked_location(&self.date) };

		let mut milliseconds: f64 = f64::MAX;
		if unsafe { !DateGetMsecSinceEpoch(cx.cx(), handle, &mut milliseconds) } || milliseconds == f64::MAX {
			None
		} else {
			Some(Utc.timestamp_millis(milliseconds as i64))
		}
	}

	pub fn is_valid(&self, cx: &Context) -> bool {
		let handle = unsafe { Handle::from_marked_location(&self.date) };

		let mut is_valid = true;
		return unsafe { DateIsValid(cx.cx(), handle, &mut is_valid) } && is_valid;
	}

	pub(crate) unsafe fn is_date_raw(cx: &Context, obj: *mut JSObject) -> bool {
		rooted!(in(cx.cx()) let mut robj = obj);

		let mut is_date = false;
		ObjectIsDate(cx.cx(), robj.handle(), &mut is_date) && is_date
	}
}

impl RootKind for Date {
	#[allow(non_snake_case)]
	fn rootKind() -> JS::RootKind {
		JS::RootKind::Object
	}
}

impl GCMethods for Date {
	unsafe fn initial() -> Self {
		Date { date: GCMethods::initial() }
	}

	unsafe fn post_barrier(v: *mut Self, prev: Self, next: Self) {
		GCMethods::post_barrier(&mut (*v).date, prev.date, next.date)
	}
}

impl Deref for Date {
	type Target = *mut JSObject;

	fn deref(&self) -> &Self::Target {
		&self.date
	}
}

impl DerefMut for Date {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.date
	}
}

impl FromValue for Date {
	fn from_value<'c, 's: 'c>(cx: &Context<'c>, value: Local<'s, Value>) -> Result<Local<'c, Self>, ()> {
		if !value.is_object() {
			unsafe { throw_type_error(cx.cx(), "Value is not an object") };
			return Err(());
		}

		let object = value.to_object();
		unsafe { AssertSameCompartment(cx.cx(), object) };
		if unsafe { !Date::is_date_raw(cx, object) } {
			unsafe { throw_type_error(cx.cx(), "Value is not a date") };
			return Err(());
		}

		Date::from_raw(cx, object).ok_or(())
	}
}

impl ToValue for Date {
	fn to_value<'c, 's: 'c>(this: Local<'s, Self>, cx: &Context<'c>) -> Local<'c, Value> {
		this.to_value(cx)
	}
}
