/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

pub use class::UrlSearchParams;

#[js_class]
mod class {
	use form_urlencoded::Serializer;
	use mozjs::gc::Traceable;
	use mozjs::jsapi::{Heap, JSObject, JSTracer};

	use ion::{ClassDefinition, Context, Error, ErrorKind, JSIterator, Local, Object, Result, Value};
	use ion::conversions::ToValue;
	use ion::symbol::WellKnownSymbolCode;

	use crate::globals::url::Url;

	#[ion(name = "URLSearchParams", into_value)]
	pub struct UrlSearchParams {
		pairs: Vec<(String, String)>,
		url: Option<Box<Heap<*mut JSObject>>>,
	}

	// TODO: Implement Constructor for URLSearchParams
	impl UrlSearchParams {
		pub(crate) fn new(cx: &Context, pairs: Vec<(String, String)>, url_object: Option<&Object>) -> Result<UrlSearchParams> {
			if let Some(url_object) = url_object {
				if !Url::instance_of(cx, url_object, None) {
					return Err(Error::new("Expected URL", ErrorKind::Type));
				}
			}

			Ok(UrlSearchParams {
				pairs,
				url: url_object.map(|url_object| Heap::boxed(url_object.handle().get())),
			})
		}

		#[ion(constructor)]
		pub fn constructor() -> UrlSearchParams {
			UrlSearchParams { pairs: vec![], url: None }
		}

		#[ion(skip)]
		pub fn all_pairs(&self) -> impl Iterator<Item = &(String, String)> {
			self.pairs.iter()
		}

		#[ion(get)]
		pub fn get_size(&self) -> i32 {
			self.pairs.len() as i32
		}

		pub fn append(&mut self, name: String, value: String) {
			self.pairs.push((name, value));
			self.update();
		}

		pub fn delete(&mut self, name: String, value: Option<String>) {
			if let Some(value) = value {
				self.pairs.retain(|(k, v)| k != &name && v != &value)
			} else {
				self.pairs.retain(|(k, _)| k != &name)
			}
			self.update();
		}

		pub fn get(&self, key: String) -> Option<String> {
			self.pairs.iter().find(|(k, _)| k == &key).map(|(_, v)| v.clone())
		}

		pub fn getAll(&self, key: String) -> Vec<String> {
			self.pairs.iter().filter(|(k, _)| k == &key).map(|(_, v)| v.clone()).collect()
		}

		pub fn has(&self, key: String, value: Option<String>) -> bool {
			if let Some(value) = value {
				self.pairs.iter().any(|(k, v)| k == &key && v == &value)
			} else {
				self.pairs.iter().any(|(k, _)| k == &key)
			}
		}

		pub fn set(&mut self, key: String, value: String) {
			let mut i = 0;
			let mut index = None;

			self.pairs.retain(|(k, _)| {
				if index.is_none() {
					if k == &key {
						index = Some(i);
					} else {
						i += 1;
					}
					true
				} else {
					k != &key
				}
			});

			match index {
				Some(index) => self.pairs[index].1 = value,
				None => self.pairs.push((key, value)),
			}

			self.update()
		}

		pub fn sort(&mut self) {
			self.pairs.sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
			self.update()
		}

		pub fn toString(&self) -> String {
			Serializer::new(String::new()).extend_pairs(&*self.pairs).finish()
		}

		pub(crate) fn update(&mut self) {
			if let Some(url) = &self.url {
				let url = Object::from(unsafe { Local::from_heap(url) });
				let url = Url::get_private(&url);
				if self.pairs.is_empty() {
					url.url.set_query(None);
				} else {
					url.url.query_pairs_mut().clear().extend_pairs(&self.pairs);
				}
			}
		}

		#[ion(name = WellKnownSymbolCode::Iterator)]
		pub fn iterator<'cx: 'o, 'o>(&self, cx: &'cx Context, #[ion(this)] this: &Object<'o>) -> ion::Iterator {
			let thisv = this.as_value(cx);
			ion::Iterator::new(SearchParamsIterator::default(), &thisv)
		}
	}

	#[derive(Default)]
	pub struct SearchParamsIterator(usize);

	impl JSIterator for SearchParamsIterator {
		fn next_value<'cx>(&mut self, cx: &'cx Context, private: &Value<'cx>) -> Option<Value<'cx>> {
			let object = private.to_object(cx);
			let search_params = UrlSearchParams::get_private(&object);
			let pair = search_params.pairs.get(self.0);
			pair.map(move |(k, v)| {
				self.0 += 1;
				[k, v].as_value(cx)
			})
		}
	}

	unsafe impl Traceable for UrlSearchParams {
		#[inline]
		unsafe fn trace(&self, trc: *mut JSTracer) {
			unsafe {
				self.url.trace(trc);
			}
		}
	}
}
