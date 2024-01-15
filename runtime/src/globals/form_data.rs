use ion::{
	Context, Object,
	conversions::{FromValue, ToValue},
	ClassDefinition, Result, JSIterator,
	symbol::WellKnownSymbolCode,
	class::{Reflector, NativeObject},
	TracedHeap,
};
use mozjs::jsapi::{JSObject, ToStringSlow};

use super::file::{Blob, File, BlobPart, FileOptions, BlobOptions, Endings};

// TODO: maintain the same File instance instead of Bytes
#[derive(Clone)]
pub enum FormDataEntryValue {
	String(String),
	File(TracedHeap<*mut JSObject>),
}

impl FormDataEntryValue {
	pub fn from_value<'cx: 'v, 'v>(
		cx: &'cx Context, value: &ion::Value<'v>, file_name: Option<String>,
	) -> Result<Self> {
		if value.get().is_string() {
			let str = String::from_value(cx, value, false, ())?;
			Ok(Self::String(str))
		} else if value.get().is_object()
			&& (Blob::instance_of(cx, &value.to_object(cx), None) || File::instance_of(cx, &value.to_object(cx), None))
		{
			let obj = value.to_object(cx);
			let file = if File::instance_of(cx, &obj, None) {
				if let Some(name) = file_name {
					let file = File::get_private(&obj);
					cx.root_object(File::new_object(
						cx,
						Box::new(File::constructor(
							vec![BlobPart(file.blob.as_bytes().clone())],
							name,
							Some(FileOptions {
								last_modified: Some(file.get_last_modified()),
								blob: BlobOptions {
									kind: file.blob.kind(),
									endings: Endings::Transparent,
								},
							}),
						)),
					))
					.into()
				} else {
					obj
				}
			} else {
				let name = file_name.unwrap_or("blob".to_string());
				let blob = Blob::get_private(&obj);
				cx.root_object(File::new_object(
					cx,
					Box::new(File::constructor(
						vec![BlobPart(blob.as_bytes().clone())],
						name,
						Some(FileOptions {
							last_modified: None,
							blob: BlobOptions {
								kind: blob.kind(),
								endings: Endings::Transparent,
							},
						}),
					)),
				))
				.into()
			};
			Ok(Self::File(TracedHeap::from_local(&file)))
		} else {
			let str = unsafe { ToStringSlow(cx.as_ptr(), value.handle().into()) };
			let str = ion::String::from(cx.root_string(str));
			Ok(Self::String(str.to_owned(cx)))
		}
	}
}

impl<'cx> ToValue<'cx> for FormDataEntryValue {
	fn to_value(&self, cx: &'cx Context, value: &mut ion::Value) {
		match self {
			Self::String(s) => s.to_value(cx, value),
			Self::File(obj) => obj.get().to_value(cx, value),
		}
	}
}

pub struct KvPair {
	pub key: String,
	pub value: FormDataEntryValue,
}

#[js_class]
pub struct FormData {
	reflector: Reflector,

	// FormData is an ordered collection, so we use a Vec instead of e.g. a Map.
	#[trace(no_trace)]
	pub(super) kv_pairs: Vec<KvPair>,
}

impl FormData {
	pub fn all_pairs(&self) -> impl Iterator<Item = &KvPair> {
		self.kv_pairs.iter()
	}

	pub fn append_native_string(&mut self, key: String, value: String) {
		self.kv_pairs.push(KvPair {
			key,
			value: FormDataEntryValue::String(value),
		});
	}

	pub fn append_native_file(&mut self, key: String, value: &File) {
		self.kv_pairs.push(KvPair {
			key,
			value: FormDataEntryValue::File(TracedHeap::new(value.reflector().get())),
		});
	}

	fn make_iterator(&self, cx: &Context, mode: FormDataIteratorMode) -> ion::Iterator {
		let this = self.reflector.get().as_value(cx);
		ion::Iterator::new(FormDataIterator { index: 0, mode }, &this)
	}
}

#[js_class]
impl FormData {
	#[ion(constructor)]
	pub fn constructor() -> FormData {
		FormData {
			reflector: Reflector::default(),
			kv_pairs: vec![],
		}
	}

	pub fn append<'cx>(
		&mut self, cx: &'cx Context, name: String, value: ion::Value<'cx>, file_name: Option<String>,
	) -> Result<()> {
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

	pub fn set<'cx>(
		&mut self, cx: &'cx Context, name: String, value: ion::Value<'cx>, file_name: Option<String>,
	) -> Result<()> {
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
	pub fn iterator<'cx: 'o, 'o>(&self, cx: &'cx Context) -> ion::Iterator {
		self.make_iterator(cx, FormDataIteratorMode::Both)
	}

	pub fn entries<'cx: 'o, 'o>(&self, cx: &'cx Context) -> ion::Iterator {
		self.make_iterator(cx, FormDataIteratorMode::Both)
	}

	pub fn keys<'cx: 'o, 'o>(&self, cx: &'cx Context) -> ion::Iterator {
		self.make_iterator(cx, FormDataIteratorMode::Keys)
	}

	pub fn values<'cx: 'o, 'o>(&self, cx: &'cx Context) -> ion::Iterator {
		self.make_iterator(cx, FormDataIteratorMode::Values)
	}
}

enum FormDataIteratorMode {
	Keys,
	Values,
	Both,
}

struct FormDataIterator {
	index: usize,
	mode: FormDataIteratorMode,
}

impl JSIterator for FormDataIterator {
	fn next_value<'cx>(&mut self, cx: &'cx Context, private: &ion::Value<'cx>) -> Option<ion::Value<'cx>> {
		let object = private.to_object(cx);
		let form_data = FormData::get_private(&object);
		if self.index >= form_data.kv_pairs.len() {
			None
		} else {
			let kv = &form_data.kv_pairs[self.index];
			self.index += 1;

			match self.mode {
				FormDataIteratorMode::Both => {
					let mut array = ion::Array::new_with_length(cx, 2);
					array.set_as(cx, 0, kv.key.as_str());
					array.set_as(cx, 1, &kv.value);
					Some(array.as_value(cx))
				}
				FormDataIteratorMode::Keys => Some(kv.key.as_value(cx)),
				FormDataIteratorMode::Values => Some(kv.value.as_value(cx)),
			}
		}
	}
}

pub fn define(cx: &Context, global: &mut Object) -> bool {
	FormData::init_class(cx, global).0
}
