use proc_macro2::TokenStream;
use syn::{meta::ParseNestedMeta, Attribute, LitStr, Result};

use super::{ParseAttribute, ParseArgument};

#[derive(Default)]
struct InstrumentAttribute {
	name: Option<LitStr>,
	level: Option<LitStr>,
	target: Option<LitStr>,
	ret: bool,
}

impl ParseAttribute for InstrumentAttribute {
	fn parse(&mut self, meta: &ParseNestedMeta) -> Result<()> {
		self.name.parse_argument(meta, "name", None)?;
		self.level.parse_argument(meta, "level", None)?;
		self.target.parse_argument(meta, "target", None)?;
		self.ret.parse_argument(meta, "ret", None)?;
		Ok(())
	}
}

pub(crate) fn instrument_from_attributes(attrs: &mut Vec<Attribute>, name: &str) -> Result<Option<TokenStream>> {
	let mut indices = Vec::new();

	let mut dont_instrument = false;
	let mut instrument_attribute: Option<InstrumentAttribute> = None;
	let mut instrument_tokens: Option<TokenStream> = None;

	for (i, attr) in attrs.iter().enumerate() {
		if attr.path().is_ident("dont_instrument") {
			dont_instrument = true;
			indices.push(i);
		}

		if attr.path().is_ident("instrument") {
			let mut attribute = InstrumentAttribute::default();
			attr.parse_nested_meta(|meta| attribute.parse(&meta))?;

			instrument_attribute = Some(attribute);
			instrument_tokens = Some(attr.parse_args()?);
			indices.push(i);
		}
	}
	for index in indices {
		attrs.remove(index);
	}

	// Need to get this far even if we don't have the feature, to ensure correct
	// syntax and remove the attributes
	#[cfg(feature = "instrument")]
	{
		if dont_instrument {
			Ok(None)
		} else {
			let ret = match instrument_attribute {
				Some(InstrumentAttribute { ret: true, .. }) => None,
				_ => Some(quote!(ret,)),
			};

			let name = match instrument_attribute {
				Some(InstrumentAttribute { name: Some(_), .. }) => None,
				_ => Some(quote!(name = #name,)),
			};

			let target = match instrument_attribute {
				Some(InstrumentAttribute { target: Some(_), .. }) => None,
				_ => Some(quote!(target = "ion::native_func",)),
			};

			let level = match instrument_attribute {
				Some(InstrumentAttribute { level: Some(_), .. }) => None,
				_ => {
					cfg_if::cfg_if! {
						if #[cfg(feature = "instrument-level-trace")] {
							Some(quote!(level = "trace",))
						} else if #[cfg(feature = "instrument-level-debug")] {
							Some(quote!(level = "debug",))
						} else if #[cfg(feature = "instrument-level-info")] {
							Some(quote!(level = "info",))
						} else {
							Some(quote!(level = "trace",))
						}
					}
				}
			};

			Ok(Some(quote!(#name #ret #target #level #instrument_tokens)))
		}
	}

	#[cfg(not(feature = "instrument"))]
	{
		_ = dont_instrument;
		_ = instrument_attribute;
		_ = instrument_tokens;
		_ = name;
		Ok(None)
	}
}
