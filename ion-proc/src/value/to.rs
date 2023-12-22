/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use convert_case::{Case, Casing};
use proc_macro2::{Ident, Span, TokenStream};
use syn::{Block, Data, DeriveInput, Error, Field, Fields, GenericParam, Generics, ItemImpl, parse2, Result};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;

use crate::attribute::krate::crate_from_attributes;
use crate::utils::add_trait_bounds;
use crate::value::attribute::{FieldAttribute, Tag};

pub(crate) fn impl_to_value(mut input: DeriveInput) -> Result<ItemImpl> {
	let ion = &crate_from_attributes(&input.attrs);

	add_trait_bounds(&mut input.generics, &parse_quote!(#ion::conversions::ToValue));
	let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
	let mut impl_generics: Generics = parse2(quote_spanned!(impl_generics.span() => #impl_generics))?;

	let has_cx = impl_generics.params.iter().any(|param| {
		if let GenericParam::Lifetime(lt) = param {
			lt.lifetime == parse_quote!('cx)
		} else {
			false
		}
	});
	if !has_cx {
		impl_generics.params.push(parse2(quote!('cx))?);
	}

	let tag = Tag::default();
	let inherit = false;
	let repr = None;
	// for attr in &input.attrs {
	// 	if attr.path().is_ident("ion") {
	// 		let args: Punctuated<DataAttribute, Token![,]> = attr.parse_args_with(Punctuated::parse_terminated)?;

	// 		for arg in args {
	// 			match arg {
	// 				DataAttribute::Tag(data_tag) => {
	// 					tag = data_tag;
	// 				}
	// 				DataAttribute::Inherit(_) => {
	// 					inherit = true;
	// 				}
	// 			}
	// 		}
	// 	} else if attr.path().is_ident("repr") {
	// 		let nested = attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
	// 		let allowed_reprs: Vec<Ident> = vec![
	// 			parse_quote!(i8),
	// 			parse_quote!(i16),
	// 			parse_quote!(i32),
	// 			parse_quote!(i64),
	// 			parse_quote!(u8),
	// 			parse_quote!(u16),
	// 			parse_quote!(u32),
	// 			parse_quote!(u64),
	// 		];
	// 		for meta in nested {
	// 			if let Meta::Path(path) = &meta {
	// 				for allowed_repr in &allowed_reprs {
	// 					if path.is_ident(allowed_repr) {
	// 						if repr.is_none() {
	// 							repr = Some(path.get_ident().unwrap().clone());
	// 						} else {
	// 							return Err(Error::new(meta.span(), "Only One Representation Allowed in #[repr]"));
	// 						}
	// 					}
	// 				}
	// 			}
	// 		}
	// 	}
	// }

	let name = &input.ident;

	let (body, requires_object) = impl_body(ion, input.span(), &input.data, name, tag, inherit, repr)?;

	let prefix = requires_object.then(|| quote!(let mut __object = ion::Object::new(cx);));
	let postfix = requires_object.then(|| quote!(__object.to_value(cx, value);));

	parse2(quote_spanned!(input.span() =>
		#[automatically_derived]
		impl #impl_generics #ion::conversions::ToValue<'cx> for #name #ty_generics #where_clause {
			fn to_value<'v>(&self, cx: &'cx #ion::Context, value: &mut #ion::Value) {
				#prefix
				#body
				#postfix
			}
		}
	))
}

fn impl_body(
	ion: &TokenStream, span: Span, data: &Data, _ident: &Ident, tag: Tag, inherit: bool, _repr: Option<Ident>,
) -> Result<(Block, bool)> {
	match data {
		Data::Struct(data) => match &data.fields {
			Fields::Named(fields) => {
				let declarations = map_fields(ion, &fields.named, None, tag, inherit)?;
				parse2(quote_spanned!(span => {
					#(#declarations)*
				}))
				.map(|block| (block, true))
			}
			Fields::Unnamed(fields) => {
				let declarations = map_fields(ion, &fields.unnamed, None, tag, inherit)?;
				parse2(quote_spanned!(span => {
					#(#declarations)*
				}))
				.map(|block| (block, true))
			}
			Fields::Unit => parse2(quote_spanned!(span => { value.handle_mut().set(mozjs::jsval::UndefinedValue()); }))
				.map(|block| (block, false)),
		},
		Data::Enum(_) => {
			// TODO: figure enums out
			Err(Error::new(span, "#[derive(ToValue)] support for enum types is WIP"))

			// let unit = data.variants.iter().all(|variant| matches!(variant.fields, Fields::Unit));

			// let variants: Vec<(Block, _)> = data
			// 	.variants
			// 	.iter()
			// 	.filter_map(|variant| {
			// 		let variant_ident = &variant.ident;
			// 		let variant_string = variant_ident.to_string();

			// 		let mut tag = tag.clone();
			// 		let mut inherit = inherit;

			// 		for attr in &variant.attrs {
			// 			if attr.path().is_ident("ion") {
			// 				let args: Punctuated<VariantAttribute, Token![,]> =
			// 					match attr.parse_args_with(Punctuated::parse_terminated) {
			// 						Ok(args) => args,
			// 						Err(e) => return Some(Err(e)),
			// 					};

			// 				for arg in args {
			// 					match arg {
			// 						VariantAttribute::Tag(variant_tag) => {
			// 							tag = variant_tag;
			// 						}
			// 						VariantAttribute::Inherit(_) => {
			// 							inherit = true;
			// 						}
			// 						VariantAttribute::Skip(_) => {
			// 							return None;
			// 						}
			// 					}
			// 				}
			// 			}
			// 		}

			// 		let handle_result = quote!(if let ::std::result::Result::Ok(success) = variant {
			// 			return ::std::result::Result::Ok(success);
			// 		});
			// 		match &variant.fields {
			// 			Fields::Named(fields) => {
			// 				let mapped = match map_fields(ion, &fields.named, Some(variant_string), tag, inherit) {
			// 					Ok(mapped) => mapped,
			// 					Err(e) => return Some(Err(e)),
			// 				};
			// 				let (requirement, idents, declarations, requires_object) = mapped;

			// 				Some(
			// 					parse2(quote_spanned!(variant.span() => {
			// 						let variant: #ion::Result<Self> = (|| {
			// 							#requirement
			// 							#(#declarations)*
			// 							::std::result::Result::Ok(Self::#variant_ident { #(#idents, )* })
			// 						})();
			// 						#handle_result
			// 					}))
			// 					.map(|block| (block, requires_object)),
			// 				)
			// 			}
			// 			Fields::Unnamed(fields) => {
			// 				let mapped = match map_fields(ion, &fields.unnamed, Some(variant_string), tag, inherit) {
			// 					Ok(mapped) => mapped,
			// 					Err(e) => return Some(Err(e)),
			// 				};
			// 				let (requirement, idents, declarations, requires_object) = mapped;

			// 				Some(
			// 					parse2(quote_spanned!(variant.span() => {
			// 						let variant: #ion::Result<Self> = (|| {
			// 							#requirement
			// 							#(#declarations)*
			// 							::std::result::Result::Ok(Self::#variant_ident(#(#idents, )*))
			// 						})();
			// 						#handle_result
			// 					}))
			// 					.map(|block| (block, requires_object)),
			// 				)
			// 			}
			// 			Fields::Unit => {
			// 				if let Some((_, discriminant)) = &variant.discriminant {
			// 					if unit && repr.is_some() {
			// 						return Some(
			// 							parse2(quote_spanned!(
			// 								variant.fields.span() => {
			// 									if discriminant == #discriminant {
			// 										return ::std::result::Result::Ok(Self::#variant_ident);
			// 									}
			// 								}
			// 							))
			// 							.map(|block| (block, false)),
			// 						);
			// 					}
			// 				}
			// 				Some(
			// 					parse2(quote!({return ::std::result::Result::Ok(Self::#variant_ident);}))
			// 						.map(|block| (block, false)),
			// 				)
			// 			}
			// 		}
			// 	})
			// 	.collect::<Result<_>>()?;
			// let (variants, requires_object): (Vec<_>, Vec<_>) = variants.into_iter().unzip();
			// let requires_object = requires_object.into_iter().any(|b| b);

			// let error = format!("Value does not match any of the variants of enum {}", ident);

			// let mut if_unit = None;

			// if unit {
			// 	if let Some(repr) = repr {
			// 		if_unit = Some(
			// 			quote_spanned!(repr.span() => let discriminant: #repr = #ion::conversions::FromValue::from_value(cx, value, true, #ion::conversions::ConversionBehavior::EnforceRange)?;),
			// 		);
			// 	}
			// }

			// parse2(quote_spanned!(span => {
			// 	#if_unit
			// 	#(#variants)*

			// 	::std::result::Result::Err(#ion::Error::new(#error, #ion::ErrorKind::Type))
			// }))
			// .map(|b| (b, requires_object))
		}
		Data::Union(_) => Err(Error::new(
			span,
			"#[derive(ToValue)] is not implemented for union types",
		)),
	}
}

fn map_fields(
	ion: &TokenStream, fields: &Punctuated<Field, Token![,]>, _variant: Option<String>, _tag: Tag, _inherit: bool,
) -> Result<Vec<TokenStream>> {
	// let mut is_tagged = None;

	// let requirement = match tag {
	// 	Tag::Untagged(_) => quote!(),
	// 	Tag::External(kw) => {
	// 		is_tagged = Some(kw);
	// 		if let Some(variant) = variant {
	// 			let error = format!("Expected Object at External Tag {}", variant);
	// 			quote_spanned!(kw.span() =>
	// 				let __object: #ion::Object = __object.get_as(cx, #variant, true, ())
	// 					.ok_or_else(|| #ion::Error::new(#error, #ion::ErrorKind::Type))?;
	// 			)
	// 		} else {
	// 			return Err(Error::new(kw.span(), "Cannot have Tag for Struct"));
	// 		}
	// 	}
	// 	Tag::Internal { kw, key, .. } => {
	// 		is_tagged = Some(kw);
	// 		if let Some(variant) = variant {
	// 			let missing_error = format!("Expected Internal Tag key {}", key.value());
	// 			let error = format!("Expected Internal Tag {} at key {}", variant, key.value());
	// 			quote_spanned!(kw.span() =>
	// 				let __key: ::std::string::String = __object.get_as(cx, #key, true, ()).ok_or_else(|| #ion::Error::new(#missing_error, #ion::ErrorKind::Type))?;
	// 				if __key != #variant {
	// 					return Err(#ion::Error::new(#error, #ion::ErrorKind::Type));
	// 				}
	// 			)
	// 		} else {
	// 			return Err(Error::new(kw.span(), "Cannot have Tag for Struct"));
	// 		}
	// 	}
	// };
	// let mut requires_object = is_tagged.is_some();

	let vec: Vec<_> = fields
		.iter()
		.enumerate()
		.filter_map(|(index, field)| {
			let (ident, mut key) = if let Some(ident) = &field.ident {
				(ident.clone(), ident.to_string().to_case(Case::Camel))
			} else {
				let ident = format_ident!("field{}", index);
				(ident, index.to_string())
			};

			let attrs = &field.attrs;

			// let mut inherit = inherit;

			for attr in attrs {
				if attr.path().is_ident("ion") {
					let args: Punctuated<FieldAttribute, Token![,]> =
						match attr.parse_args_with(Punctuated::parse_terminated) {
							Ok(args) => args,
							Err(e) => return Some(Err(e)),
						};

					for arg in args {
						use FieldAttribute as FA;
						match arg {
							FA::Name { name, .. } => {
								key = name.value();
							}
							// FA::Inherit(_) => {
							// 	inherit = true;
							// }
							FA::Skip(_) => {
								return None;
							}
							_ => (),
						}
					}
				}
			}

			let stmt = 
			// if inherit {
			// 	if is_tagged.is_some() {
			// 		return Some(Err(Error::new(field.span(), "Inherited Field cannot be parsed from a Tagged Enum")));
			// 	}
			// 	quote_spanned!(field.span() => let #ident: #ty = <#ty as #ion::conversions::FromValue>::from_value(cx, value, #strict || strict, #convert))
			// } else 
			{
				quote_spanned!(field.span() => {
					let mut __val = #ion::Value::undefined(cx);
					self.#ident.to_value(cx, &mut __val);
					__object.set(cx, #key, &__val);	
				})
			};

			Some(Ok(stmt))
		})
		.collect::<Result<_>>()?;

	Ok(vec)
}
