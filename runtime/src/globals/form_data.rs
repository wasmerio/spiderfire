use bytes::Bytes;
pub use class::FormData;
use ion::{
	Context, Object,
	conversions::{FromValue, ToValue, IntoValue},
	ClassDefinition, Result, Error, ErrorKind, JSIterator,
};

use super::file::blob::Blob;

// TODO: maintain the same File instance instead of Bytes
#[derive(Clone)]
pub enum FormDataEntryValue {
	String(String),
	File(Bytes, String),
}

impl FormDataEntryValue {
	pub fn from_value<'cx: 'v, 'v>(cx: &'cx Context, value: &ion::Value<'v>, file_name: Option<String>) -> Result<Self> {
		if value.get().is_string() {
			let str = String::from_value(cx, value, false, ())?;
			match file_name {
				None => Ok(Self::String(str)),
				Some(file_name) => Ok(Self::File(str.into_bytes().into(), file_name)),
			}
		} else if value.get().is_object() && Blob::instance_of(cx, &value.to_object(cx), None) {
			let obj = value.to_object(cx);
			let blob = Blob::get_private(&obj);
			Ok(Self::File(blob.get_bytes(), file_name.unwrap_or_else(|| "blob".to_string())))
		} else {
			Err(Error::new("FormData value must be a string or a Blob", ErrorKind::Type))
		}
	}
}

impl<'cx> ToValue<'cx> for FormDataEntryValue {
	fn to_value(&self, cx: &'cx Context, value: &mut ion::Value) {
		match self {
			Self::String(s) => s.to_value(cx, value),
			// TODO: this should return a file, not a blob
			Self::File(bytes, _name) => Box::new(Blob::new(bytes.clone())).into_value(cx, value),
		}
	}
}

pub struct KvPair {
	pub key: String,
	pub value: FormDataEntryValue,
}

#[js_class]
#[ion(runtime = crate)]
pub mod class {
	use ion::{Context, Object, Result, symbol::WellKnownSymbolCode, conversions::ToValue};

	use super::{FormDataEntryValue, KvPair, FormDataIterator};

	#[ion(into_value)]
	pub struct FormData {
		// FormData is an ordered collection, so we use a Vec instead of e.g. a Map.
		pub(super) kv_pairs: Vec<KvPair>,
	}

	impl FormData {
		#[ion(constructor)]
		pub fn constructor() -> FormData {
			FormData { kv_pairs: vec![] }
		}

		pub fn append<'cx>(&mut self, cx: &'cx Context, name: String, value: ion::Value<'cx>, file_name: Option<String>) -> Result<()> {
			let value = FormDataEntryValue::from_value(cx, &value, file_name)?;
			self.kv_pairs.push(KvPair { key: name, value });
			Ok(())
		}

		pub fn delete(&mut self, name: String) {
			self.kv_pairs.retain(|kv| kv.key != name);
		}

		pub fn get(&self, name: String) -> Option<FormDataEntryValue> {
			self.kv_pairs.iter().find(|kv| kv.key == name).map(|kv| kv.value.clone())
		}

		pub fn get_all(&self, name: String) -> Vec<FormDataEntryValue> {
			self.kv_pairs.iter().filter(|kv| kv.key == name).map(|kv| kv.value.clone()).collect()
		}

		pub fn has(&self, name: String) -> bool {
			self.kv_pairs.iter().any(|kv| kv.key == name)
		}

		pub fn set<'cx>(&mut self, cx: &'cx Context, name: String, value: ion::Value<'cx>, file_name: Option<String>) -> Result<()> {
			let value = FormDataEntryValue::from_value(cx, &value, file_name)?;

			let mut i = 0;
			let mut index = None;

			self.kv_pairs.retain(|kv| {
				if index.is_none() {
					if kv.key == name {
						index = Some(i);
					} else {
						i += 1;
					}
					true
				} else {
					kv.key != name
				}
			});

			match index {
				Some(index) => self.kv_pairs[index].value = value,
				None => self.kv_pairs.push(KvPair { key: name, value }),
			}

			Ok(())
		}

		#[ion(name = WellKnownSymbolCode::Iterator)]
		pub fn iterator<'cx: 'o, 'o>(&self, cx: &'cx Context, #[ion(this)] this: &Object<'o>) -> ion::Iterator {
			let this = this.as_value(cx);
			ion::Iterator::new(FormDataIterator { index: 0 }, &this)
		}
	}
}

struct FormDataIterator {
	index: usize,
}

impl JSIterator for FormDataIterator {
	fn next_value<'cx>(&mut self, cx: &'cx Context, private: &ion::Value<'cx>) -> Option<ion::Value<'cx>> {
		let object = private.to_object(cx);
		let form_data = FormData::get_private(&object);
		if self.index >= form_data.kv_pairs.len() {
			None
		} else {
			let kv = &form_data.kv_pairs[self.index];
			let mut array = ion::Array::new_with_length(cx, 2);
			array.set_as(cx, 0, kv.key.as_str());
			array.set_as(cx, 1, &kv.value);
			self.index += 1;
			Some(array.as_value(cx))
		}
	}
}

pub fn define(cx: &Context, global: &mut Object) -> bool {
	FormData::init_class(cx, global).0
}
