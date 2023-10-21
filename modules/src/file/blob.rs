use bytes::Bytes;
use ion::conversions::FromValue;

pub use class::Blob;

#[derive(FromValue, Default, Clone)]
pub struct BlobPropertyBag {
	#[ion(name = "type")]
	content_type: Option<String>,

	#[allow(dead_code)]
	// TODO: implement endings
	endings: Option<String>,
}

#[allow(dead_code)]
const ENDING_TRANSPARENT: &str = "transparent";
#[allow(dead_code)]
const ENDING_NATIVE: &str = "native";

pub struct BlobPart {
	bytes: Bytes,
}

impl BlobPart {
	pub fn get_size(&self) -> usize {
		self.bytes.len()
	}
}

impl<'cx> FromValue<'cx> for BlobPart {
	type Config = ();

	fn from_value<'v>(cx: &'cx ion::Context, value: &ion::Value<'v>, strict: bool, _config: Self::Config) -> ion::Result<Self>
	where
		'cx: 'v,
	{
		if value.get().is_string() {
			let str = String::from_value(cx, value, strict, ())?;
			Ok(Self { bytes: str.into_bytes().into() })
		} else if value.get().is_object() {
			let obj = (*value.to_object(cx)).get();
			let bytes = runtime::typedarray_to_bytes!(obj, [ArrayBuffer, true], [ArrayBufferView, true], [Uint8Array, true])?;
			Ok(Self { bytes })
		} else {
			Err(ion::Error::new("Invalid blob part type", ion::ErrorKind::Type))
		}
	}
}

#[js_class]
mod class {
	use bytes::Bytes;
	use ion::{Array, Context, Result, conversions::FromValue};
	use mozjs::conversions::ConversionBehavior;

	use super::{BlobPropertyBag, BlobPart};

	#[ion(into_value)]
	pub struct Blob {
		parent: Option<*const Blob>,
		parts: Vec<BlobPart>,
		options: BlobPropertyBag,
		start: usize,
		end: usize,
	}

	impl Blob {
		#[ion(constructor)]
		pub fn constructor<'cx>(cx: &'cx Context<'cx>, blob_parts: Array<'cx>, options: Option<BlobPropertyBag>) -> Result<Blob> {
			let mut parts = vec![];
			for (_, part) in blob_parts.iter(cx, None) {
				parts.push(BlobPart::from_value(cx, &part, false, ())?);
			}
			let size = parts.iter().map(|p| p.get_size()).sum();
			Ok(Blob {
				parts,
				options: options.unwrap_or_default(),
				parent: None,
				start: 0,
				end: size,
			})
		}

		#[ion(get)]
		pub fn get_size(&self) -> u64 {
			(self.end - self.start) as u64
		}

		#[ion(get)]
		pub fn get_type(&self) -> String {
			self.options.content_type.as_ref().map(|s| s.as_str()).unwrap_or("").to_string()
		}

		pub fn slice(
			&self, #[ion(convert = ConversionBehavior::EnforceRange)] start: Option<u64>,
			#[ion(convert = ConversionBehavior::EnforceRange)] end: Option<u64>, content_type: Option<String>,
		) -> Blob {
			let start = match start {
				None => self.start,
				Some(start) => self.end.min(self.start.max(start as usize + self.start)),
			};
			let end = match end {
				None => self.end,
				Some(end) => start.max(self.end.min(self.start.max(end as usize + self.start))),
			};
			let mut options = self.options.clone();
			if let Some(content_type) = content_type {
				options.content_type = Some(content_type);
			}
			Blob {
				parent: Some(self.parent.unwrap_or(self)),
				start,
				end,
				parts: vec![],
				options,
			}
		}

		#[ion(skip)]
		pub fn get_bytes(&self) -> Bytes {
			unsafe {
				let mut buf = vec![0u8; self.get_size() as usize];
				let mut slice = &mut buf[..];

				let mut s = self as *const Blob;
				while (*s).parent.is_some() {
					s = (*s).parent.unwrap();
				}

				let mut position = 0;
				let mut read = 0;
				let my_size = self.get_size() as usize;
				for part in (*s).parts.iter() {
					let part_size = part.get_size();
					if position < self.start {
						if position + part_size <= self.start {
							position += part_size;
							continue;
						} else {
							let start = self.start - position;
							let count = (my_size - read).min(part_size - start);
							let (head, tail) = slice.split_at_mut(count);
							head.copy_from_slice(&part.bytes.as_ref()[start..start + count]);
							read += count;
							position += part_size;
							slice = tail;
						}
					} else {
						let count = (my_size - read).min(part_size);
						let (head, tail) = slice.split_at_mut(count);
						head.copy_from_slice(&part.bytes.as_ref()[..count]);
						read += count;
						position += part_size;
						slice = tail;
					}

					if read >= my_size {
						break;
					}
				}

				Bytes::from(buf)
			}
		}

		pub async fn text(&self) -> Result<String> {
			let bytes = self.get_bytes();
			String::from_utf8(bytes.into()).map_err(|_| ion::Error::new("String contains invalid UTF-8 characters", ion::ErrorKind::Normal))
		}

		#[ion(name = "arrayBuffer")]
		pub async fn array_buffer(&self) -> ion::typedarray::ArrayBuffer {
			ion::typedarray::ArrayBuffer::from(Vec::from(self.get_bytes()))
		}
	}
}
