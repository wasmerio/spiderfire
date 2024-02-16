/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */

use std::marker::PhantomData;
use std::ops::Deref;

#[derive(Debug)]
pub enum VisibleAscii {}

#[derive(Debug)]
pub enum Latin1 {}

#[derive(Debug)]
pub enum VerbatimBytes {}

pub trait BytePredicate: private::Sealed {}

impl<T: private::Sealed> BytePredicate for T {}

mod private {
	use crate::{
		string::byte::{Latin1, VisibleAscii},
		Value, StringRef,
		conversions::FromValue,
		Context, Result, String, Error,
	};

	use super::VerbatimBytes;

	pub trait Sealed {
		fn predicate(_byte: u8) -> bool {
			true
		}

		fn bytes_from_value(cx: &Context, value: &Value, strict: bool) -> Result<Vec<u8>> {
			let string = StringRef::from_value(cx, value, strict, ())?;
			match string {
				StringRef::Latin1(bstr) => Ok(bstr.to_vec()),
				StringRef::Utf16(wstr) => wstr
					.as_bytes()
					.chunks_exact(2)
					.map(|chunk| {
						let codepoint = u16::from_ne_bytes([chunk[0], chunk[1]]);
						u8::try_from(codepoint).map_err(|_| Error::none())
					})
					.collect::<Result<Vec<_>>>(),
			}
		}
	}

	impl Sealed for VisibleAscii {
		fn predicate(byte: u8) -> bool {
			(0x20..=0x7E).contains(&byte)
		}
	}

	impl Sealed for Latin1 {}

	impl Sealed for VerbatimBytes {
		fn bytes_from_value(cx: &Context, value: &Value, strict: bool) -> Result<Vec<u8>> {
			let str = String::from_value(cx, value, strict, ())?;
			Ok(str.as_bytes(cx).to_vec())
		}
	}
}

#[derive(Debug)]
#[repr(transparent)]
pub struct ByteStr<T: BytePredicate> {
	_predicate: PhantomData<T>,
	bytes: [u8],
}

impl<T: BytePredicate> ByteStr<T> {
	pub fn from(bytes: &[u8]) -> Option<&ByteStr<T>> {
		bytes.iter().copied().all(T::predicate).then(|| unsafe { ByteStr::from_unchecked(bytes) })
	}

	pub unsafe fn from_unchecked(bytes: &[u8]) -> &ByteStr<T> {
		unsafe { &*(bytes as *const _ as *const ByteStr<T>) }
	}

	pub fn as_bytes(&self) -> &[u8] {
		&self.bytes
	}
}

impl<T: BytePredicate> Deref for ByteStr<T> {
	type Target = [u8];

	fn deref(&self) -> &[u8] {
		self.as_bytes()
	}
}

impl<T: BytePredicate> PartialEq for ByteStr<T> {
	fn eq(&self, other: &Self) -> bool {
		self.bytes == other.bytes
	}
}

impl<T: BytePredicate> Eq for ByteStr<T> {}

#[derive(Debug, Default)]
pub struct ByteString<T: BytePredicate = Latin1> {
	_predicate: PhantomData<T>,
	bytes: Vec<u8>,
}

impl<T: BytePredicate> ByteString<T> {
	pub fn from(bytes: Vec<u8>) -> Option<ByteString<T>> {
		bytes
			.iter()
			.copied()
			.all(T::predicate)
			.then(|| unsafe { ByteString::from_unchecked(bytes) })
	}

	pub unsafe fn from_unchecked(bytes: Vec<u8>) -> ByteString<T> {
		ByteString { _predicate: PhantomData, bytes }
	}

	pub fn as_byte_str(&self) -> &ByteStr<T> {
		unsafe { ByteStr::from_unchecked(&self.bytes) }
	}

	pub fn into_vec(self) -> Vec<u8> {
		self.bytes
	}
}

impl<T: BytePredicate> Deref for ByteString<T> {
	type Target = ByteStr<T>;

	fn deref(&self) -> &ByteStr<T> {
		self.as_byte_str()
	}
}

impl<T: BytePredicate> PartialEq for ByteString<T> {
	fn eq(&self, other: &Self) -> bool {
		self.bytes == other.bytes
	}
}

impl<T: BytePredicate> Eq for ByteString<T> {}

impl<T: BytePredicate> Clone for ByteString<T> {
	fn clone(&self) -> Self {
		Self {
			_predicate: PhantomData,
			bytes: self.bytes.clone(),
		}
	}
}
