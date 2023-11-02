/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::cmp::Ordering;
use std::fmt;
use std::fmt::{Display, Formatter};
use std::str::FromStr;

use hyper::header::{HeaderMap, HeaderName, HeaderValue};

pub use class::*;
use ion::{Array, Context, Error, ErrorKind, Object, OwnedKey, Result, Value};
use ion::conversions::{FromValue, ToValue};

#[derive(FromValue)]
pub enum Header {
	#[ion(inherit)]
	Multiple(Vec<String>),
	#[ion(inherit)]
	Single(String),
}

impl Display for Header {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		match self {
			Header::Multiple(vec) => f.write_str(&vec.join(", ")),
			Header::Single(str) => f.write_str(str),
		}
	}
}

impl ToValue<'_> for Header {
	fn to_value(&self, cx: &Context, value: &mut Value) {
		self.to_string().to_value(cx, value)
	}
}

pub struct HeadersObject(HeaderMap);

impl<'cx> FromValue<'cx> for HeadersObject {
	type Config = ();

	fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, _: bool, _: ()) -> Result<HeadersObject>
	where
		'cx: 'v,
	{
		let object = Object::from_value(cx, value, true, ())?;
		let mut headers = HeaderMap::new();
		append_to_headers(cx, &mut headers, object, false)?;
		Ok(HeadersObject(headers))
	}
}

pub struct HeaderEntry {
	name: String,
	value: String,
}

impl<'cx> FromValue<'cx> for HeaderEntry {
	type Config = ();
	fn from_value<'v>(cx: &'cx Context, value: &Value<'v>, _: bool, _: ()) -> Result<HeaderEntry>
	where
		'cx: 'v,
	{
		let vec = Vec::<String>::from_value(cx, value, false, ())?;
		if vec.len() != 2 {
			return Err(Error::new(
				&format!("Received Header Entry with Length {}, Expected Length 2", vec.len()),
				ErrorKind::Type,
			));
		}
		Ok(HeaderEntry {
			name: vec[0].clone(),
			value: vec[1].clone(),
		})
	}
}

impl ToValue<'_> for HeaderEntry {
	fn to_value(&self, cx: &Context, value: &mut Value) {
		let mut array = Array::new(cx);
		array.set_as(cx, 0, &self.name);
		array.set_as(cx, 1, &self.value);
		array.to_value(cx, value);
	}
}

#[derive(Default, FromValue)]
pub enum HeadersInit {
	#[ion(inherit)]
	Existing(Headers),
	#[ion(inherit)]
	Array(Vec<HeaderEntry>),
	#[ion(inherit)]
	Object(HeadersObject),
	#[default]
	#[ion(skip)]
	Empty,
}

impl HeadersInit {
	pub(crate) fn into_headers(self, mut headers: HeadersInner, kind: HeadersKind) -> Result<Headers> {
		match self {
			HeadersInit::Existing(existing) => {
				headers
					.as_mut()
					.extend(existing.headers.as_ref().into_iter().map(|(name, value)| (name.clone(), value.clone())));
				Ok(Headers { headers, kind })
			}
			HeadersInit::Array(vec) => Headers::from_array(vec, headers, kind),
			HeadersInit::Object(object) => {
				headers.as_mut().extend(object.0);
				Ok(Headers { headers, kind })
			}
			HeadersInit::Empty => Ok(Headers { headers, kind }),
		}
	}
}

#[derive(Debug)]
pub(crate) enum HeadersInner {
	Owned(HeaderMap),
	MutRef(*mut HeaderMap),
}

impl HeadersInner {
	pub fn as_ref(&self) -> &HeaderMap {
		match self {
			HeadersInner::Owned(map) => map,
			HeadersInner::MutRef(map) => unsafe { &**map },
		}
	}

	pub fn as_mut(&mut self) -> &mut HeaderMap {
		match self {
			HeadersInner::Owned(map) => map,
			HeadersInner::MutRef(map) => unsafe { &mut **map },
		}
	}
}

impl Clone for HeadersInner {
	fn clone(&self) -> HeadersInner {
		match self {
			HeadersInner::Owned(map) => HeadersInner::Owned(map.clone()),
			HeadersInner::MutRef(map) => HeadersInner::Owned(unsafe { (**map).clone() }),
		}
	}
}

impl Default for HeadersInner {
	fn default() -> HeadersInner {
		HeadersInner::Owned(HeaderMap::new())
	}
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub enum HeadersKind {
	Immutable,
	Request,
	RequestNoCors,
	Response,
	#[default]
	None,
}

#[js_class]
mod class {
	use std::ops::{Deref, DerefMut};
	use std::str::FromStr;
	use std::vec;

	use http::header::{Entry, HeaderMap, HeaderName, HeaderValue, SET_COOKIE};

	use ion::{ClassDefinition, Context, Error, JSIterator, Object, Result, Value};
	use ion::conversions::ToValue;
	use ion::symbol::WellKnownSymbolCode;

	use crate::globals::fetch::header::{get_header, Header, HeaderEntry, HeadersInit, HeadersInner, HeadersKind};

	#[derive(Clone, Default)]
	#[ion(from_value, to_value)]
	pub struct Headers {
		pub(crate) headers: HeadersInner,
		pub(crate) kind: HeadersKind,
	}

	impl Headers {
		pub(crate) fn from_array(vec: Vec<HeaderEntry>, mut headers: HeadersInner, kind: HeadersKind) -> Result<Headers> {
			for entry in vec {
				let mut name = entry.name;
				let value = entry.value;
				name.make_ascii_lowercase();

				let name = HeaderName::from_str(&name)?;
				let value = HeaderValue::try_from(&value)?;
				headers.as_mut().append(name, value);
			}
			Ok(Headers { headers, kind })
		}

		#[ion(constructor)]
		pub fn constructor(init: Option<HeadersInit>) -> Result<Headers> {
			init.unwrap_or_default().into_headers(HeadersInner::default(), HeadersKind::None)
		}

		pub fn append(&mut self, name: String, value: String) -> Result<()> {
			if self.kind != HeadersKind::Immutable {
				let name = HeaderName::from_str(&name.to_lowercase())?;
				let value = HeaderValue::from_str(&value)?;
				self.headers.as_mut().append(name, value);
				Ok(())
			} else {
				Err(Error::new("Cannot Modify Readonly Headers", None))
			}
		}

		pub fn delete(&mut self, name: String) -> Result<bool> {
			if self.kind != HeadersKind::Immutable {
				let name = HeaderName::from_str(&name.to_lowercase())?;
				match self.headers.as_mut().entry(name) {
					Entry::Occupied(o) => {
						o.remove_entry_mult();
						Ok(true)
					}
					Entry::Vacant(_) => Ok(false),
				}
			} else {
				Err(Error::new("Cannot Modify Readonly Headers", None))
			}
		}

		pub fn get(&self, name: String) -> Result<Option<Header>> {
			let name = HeaderName::from_str(&name.to_lowercase())?;
			get_header(self.headers.as_ref(), &name)
		}

		pub fn get_set_cookie(&self) -> Result<Vec<String>> {
			let header = get_header(self.headers.as_ref(), &SET_COOKIE)?;
			Ok(header.map_or_else(Vec::new, |header| match header {
				Header::Multiple(vec) => vec,
				Header::Single(str) => vec![str],
			}))
		}

		pub fn has(&self, name: String) -> Result<bool> {
			let name = HeaderName::from_str(&name.to_lowercase())?;
			Ok(self.headers.as_ref().contains_key(name))
		}

		pub fn set(&mut self, name: String, value: String) -> Result<()> {
			if self.kind != HeadersKind::Immutable {
				let name = HeaderName::from_str(&name.to_lowercase())?;
				let value = HeaderValue::from_str(&value)?;
				self.headers.as_mut().insert(name, value);
				Ok(())
			} else {
				Err(Error::new("Cannot Modify Readonly Headers", None))
			}
		}

		pub fn entries<'cx: 'o, 'o>(&self, cx: &'cx Context, #[ion(this)] this: &Object<'o>) -> ion::Iterator {
			self.iterator(cx, this)
		}

		#[ion(name = WellKnownSymbolCode::Iterator)]
		pub fn iterator<'cx: 'o, 'o>(&self, cx: &'cx Context, #[ion(this)] this: &Object<'o>) -> ion::Iterator {
			let thisv = this.as_value(cx);
			let cookies: Vec<_> = self.headers.as_ref().get_all(&SET_COOKIE).iter().map(HeaderValue::clone).collect();

			let mut keys: Vec<_> = self
				.headers
				.as_ref()
				.keys()
				.map(HeaderName::as_str)
				.map(str::to_ascii_lowercase)
				.collect();
			keys.reserve(cookies.len());
			for _ in 0..(cookies.len()) {
				keys.push(String::from(SET_COOKIE.as_str()));
			}
			keys.sort();

			ion::Iterator::new(
				HeadersIterator {
					keys: keys.into_iter(),
					cookies: cookies.into_iter(),
				},
				&thisv,
			)
		}
	}

	pub struct HeadersIterator {
		keys: vec::IntoIter<String>,
		cookies: vec::IntoIter<HeaderValue>,
	}

	impl JSIterator for HeadersIterator {
		fn next_value<'cx>(&mut self, cx: &'cx Context, private: &Value<'cx>) -> Option<Value<'cx>> {
			let object = private.to_object(cx);
			let headers = Headers::get_private(&object);
			let key = self.keys.next();
			key.and_then(|key| {
				if key == SET_COOKIE.as_str() {
					self.cookies.next().map(|value| [key.as_str(), value.to_str().unwrap()].as_value(cx))
				} else {
					get_header(headers.headers.as_ref(), &HeaderName::from_bytes(key.as_bytes()).unwrap())
						.unwrap()
						.map(|value| [key.as_str(), &value.to_string()].as_value(cx))
				}
			})
		}
	}

	impl Deref for Headers {
		type Target = HeaderMap;

		fn deref(&self) -> &HeaderMap {
			self.headers.as_ref()
		}
	}

	impl DerefMut for Headers {
		fn deref_mut(&mut self) -> &mut HeaderMap {
			self.headers.as_mut()
		}
	}
}

fn append_to_headers<'cx: 'o, 'o>(cx: &'cx Context, headers: &mut HeaderMap, obj: Object<'o>, unique: bool) -> Result<()> {
	for key in obj.keys(cx, None).map(|key| key.to_owned_key(cx)) {
		let key = match key {
			OwnedKey::Int(i) => i.to_string(),
			OwnedKey::String(s) => s,
			_ => continue,
		};

		let name = HeaderName::from_str(&key.to_lowercase())?;
		let value = obj.get(cx, &key).unwrap();
		if let Ok(array) = Array::from_value(cx, &value, false, ()) {
			if !unique {
				for i in 0..array.len(cx) {
					if let Some(str) = array.get_as::<String>(cx, i, false, ()) {
						let value = HeaderValue::from_str(&str)?;
						headers.insert(name.clone(), value);
					}
				}
			} else {
				let vec: Vec<_> = array
					.to_vec(cx)
					.into_iter()
					.map(|v| String::from_value(cx, &v, false, ()))
					.collect::<Result<_>>()?;
				let str = vec.join(";");
				let value = HeaderValue::from_str(&str)?;
				headers.insert(name, value);
			}
		} else if let Ok(str) = String::from_value(cx, &value, false, ()) {
			let value = HeaderValue::from_str(&str)?;
			headers.insert(name, value);
		} else {
			return Err(Error::new("Could not convert value to Header Value", ErrorKind::Type));
		};
	}
	Ok(())
}

pub fn get_header(headers: &HeaderMap, name: &HeaderName) -> Result<Option<Header>> {
	let values: Vec<_> = headers.get_all(name).into_iter().collect();
	match values.len().cmp(&1) {
		Ordering::Less => Ok(None),
		Ordering::Equal => Ok(Some(Header::Single(String::from(values[0].to_str()?)))),
		Ordering::Greater => {
			let values: Vec<String> = values.iter().map(|v| Ok(String::from(v.to_str()?))).collect::<Result<_>>()?;
			Ok(Some(Header::Multiple(values)))
		}
	}
}
