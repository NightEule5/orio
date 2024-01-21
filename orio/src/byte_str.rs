// SPDX-License-Identifier: Apache-2.0

mod conv;
mod encoding;
mod hash;
mod iter;

use std::borrow::Cow;
use std::ops::{Add, AddAssign, Index, RangeBounds};
use std::{fmt, slice};
use std::cell::Cell;
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::slice::SliceIndex;
use std::str::from_utf8_unchecked;
use simdutf8::compat::from_utf8;
use crate::Utf8Error;
use crate::util::partial_utf8::read_partial_utf8_into;
pub use encoding::EncodeBytes;
pub use iter::*;

/// A borrowed, segmented string of bytes.
#[derive(Clone)]
pub struct ByteStr<'a> {
	data: Vec<&'a [u8]>,
	utf8: Option<Cow<'a, str>>,
	len: usize,
}

/// An owned string of bytes.
#[derive(Clone)]
pub struct ByteString {
	// Todo: change this to a "valid range", to allow invalid bytes to be pushed but
	//  still skip the check for valid UTF-8 bytes?
	is_utf8: Cell<bool>,
	data: Vec<u8>,
}

impl<'a> ByteStr<'a> {
	/// Creates a byte string from `str`.
	pub fn from_utf8(str: &'a str) -> Self {
		Self {
			utf8: Some(str.into()),
			data: vec![str.as_bytes()],
			len: str.len(),
		}
	}
}

impl<'a> ByteStr<'a> {
	/// Creates an empty byte string.
	#[inline]
	pub const fn new() -> Self {
		Self {
			utf8: Some(Cow::Borrowed("")),
			data: Vec::new(),
			len: 0,
		}
	}

	/// Returns the length in bytes of the byte string.
	#[inline]
	pub fn len(&self) -> usize { self.len }
	/// Returns `true` if the byte string is empty.
	#[inline]
	pub fn is_empty(&self) -> bool { self.len == 0 }
	/// Returns `true` if the byte string is not empty.
	#[inline]
	pub fn is_not_empty(&self) -> bool { self.len > 0 }

	/// Returns the byte at `index`, or `None` if `index` is out of bounds.
	pub fn get(&self, mut index: usize) -> Option<&u8> {
		for chunk in self.data.iter() {
			if index < chunk.len() {
				return Some(&chunk[index])
			}

			index -= chunk.len();
		}

		None
	}

	/// Returns a byte string borrowing bytes within `range` from this byte string.
	pub fn range<R: RangeBounds<usize>>(&self, range: R) -> ByteStr {
		let Self { data, utf8, len } = self;
		let range = slice::range(range, ..*len);
		let utf8 = utf8.as_ref().and_then(|str|
			range.clone()
				 .get(str.as_ref())
				 .map(Into::into)
		);

		let mut front_skip = range.start;
		let skip_len = data.iter().take_while(|&&slice|
			front_skip >= slice.len() && {
				front_skip -= slice.len();
				true
			}
		).count();
		let mut take = range.len();
		let substr = data[skip_len..].iter().map_while(|&slice| {
			if front_skip > 0 {
				let slice = &slice[front_skip..];
				front_skip = 0;
				Some(slice)
			} else if take > 0 {
				let slice = &slice[..take.min(slice.len())];
				take -= slice.len();
				Some(slice)
			} else {
				None
			}
		}).collect();
		ByteStr { utf8, ..substr }
	}

	/// Decodes and caches the bytes as UTF-8, returning a borrow of the cache.
	/// Subsequent calls to this function will borrow from the cache, and calls to
	/// [`utf8`][] will clone from it.
	///
	/// [`utf8`]: Self::utf8
	pub fn cache_utf8(&mut self) -> Result<&str, Utf8Error> {
		Ok(
			match self.utf8 {
				Some(ref utf8) => utf8,
				None => self.utf8.insert(self.decode_utf8()?)
			}
		)
	}

	/// Returns a string of UTF-8-decoded bytes, cloning from the value cached by
	/// [`utf8_cache`][] if any.
	///
	/// [`utf8_cache`]: Self::utf8_cache
	pub fn utf8(&self) -> Result<Cow<'a, str>, Utf8Error> {
		if let Some(utf8) = self.utf8.clone() {
			Ok(utf8)
		} else {
			self.decode_utf8()
		}
	}

	/// Returns the cached UTF-8 representation of the data, or `None` if the data
	/// has not been decoded.
	pub fn cached_utf8(&self) -> Option<&str> {
		self.utf8.as_deref()
	}

	/// Iterates over segment slices in the byte string.
	pub fn slices(&self) -> Slices<'a, '_> {
		self.data.iter().copied()
	}

	/// Iterates over bytes in this byte string.
	pub fn bytes(&self) -> Bytes<'a, '_> {
		Bytes::new(self.slices(), self.len)
	}

	/// Clones the borrowed data into an owned [`ByteString`].
	pub fn to_byte_string(&self) -> ByteString {
		if let Some(utf8) = self.utf8.clone() {
			utf8.into()
		} else {
			self.bytes().collect()
		}
	}

	/// Returns the internal data.
	pub fn into_vec(self) -> Vec<&'a [u8]> {
		self.data
	}
}

impl<'a> ByteStr<'a> {
	fn decode_utf8(&self) -> Result<Cow<'a, str>, Utf8Error> {
		match &*self.data {
			&[bytes] => Ok(from_utf8(bytes)?.into()),
			data => {
				let mut buf = String::with_capacity(self.len);
				read_partial_utf8_into(data.iter().copied(), &mut buf)?;
				Ok(buf.into())
			}
		}
	}
}

impl fmt::Debug for ByteStr<'_> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let mut repr = f.debug_struct("ByteStr");
		if let Some(utf8) = self.cached_utf8() {
			repr.field("data", &utf8);
		} else {
			repr.field("data", &self.data);
		}
		repr.finish_non_exhaustive()
	}
}

impl<'a> Default for ByteStr<'a> {
	#[inline]
	fn default() -> Self { Self::new() }
}

impl Hash for ByteStr<'_> {
	fn hash<H: Hasher>(&self, state: &mut H) {
		// Hash each byte one-by-one, to ensure byte strings with the same bytes
		// have the same hash. If `self.data` was hashed, different slices of the
		// same byte sequence would may have different hashes.
		for slice in self.slices() {
			for b in slice {
				b.hash(state);
			}
		}
	}
}

impl Index<usize> for ByteStr<'_> {
	type Output = u8;

	fn index(&self, index: usize) -> &Self::Output {
		self.get(index)
			.expect(
				&format!(
					"index {index} should be less than the byte string length {}",
					self.len
				)
			)
	}
}

impl Eq for ByteStr<'_> { }

impl PartialEq for ByteStr<'_> {
	fn eq(&self, other: &Self) -> bool {
		if let (&[a], &[b]) = (self.as_ref(), other.as_ref()) {
			a == b
		} else if self.len != other.len {
			false
		} else {
			self.bytes().eq(other.bytes())
		}
	}
}

impl PartialEq<[u8]> for ByteStr<'_> {
	fn eq(&self, other: &[u8]) -> bool {
		if let &[data] = self.as_ref() {
			data == other
		} else if self.len != other.len() {
			false
		} else {
			for (a, b) in self.bytes().zip(other) {
				if a != b {
					return false
				}
			}

			true
		}
	}
}

impl PartialEq<str> for ByteStr<'_> {
	#[inline]
	fn eq(&self, other: &str) -> bool {
		self == other.as_bytes()
	}
}

impl PartialEq<ByteString> for ByteStr<'_> {
	fn eq(&self, other: &ByteString) -> bool {
		self == other.as_ref()
	}
}

impl Ord for ByteStr<'_> {
	fn cmp(&self, other: &Self) -> Ordering {
		if let (&[a], &[b]) = (self.as_ref(), other.as_ref()) {
			a.cmp(b)
		} else {
			self.bytes().cmp(other.bytes())
		}
	}
}

impl PartialOrd for ByteStr<'_> {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}

impl PartialOrd<[u8]> for ByteStr<'_> {
	fn partial_cmp(&self, other: &[u8]) -> Option<Ordering> {
		if let &[data] = self.as_ref() {
			data.partial_cmp(other)
		} else {
			self.bytes().partial_cmp(other)
		}
	}
}

impl PartialOrd<str> for ByteStr<'_> {
	#[inline]
	fn partial_cmp(&self, other: &str) -> Option<Ordering> {
		self.partial_cmp(other.as_bytes())
	}
}

impl Add for ByteStr<'_> {
	type Output = Self;

	#[inline]
	fn add(mut self, rhs: Self) -> Self {
		self += rhs;
		self
	}
}

impl AddAssign for ByteStr<'_> {
	fn add_assign(&mut self, rhs: Self) {
		self.utf8 = self.utf8.take().and_then(|utf8_a|
			rhs.utf8.map(|utf8_b| utf8_a + utf8_b)
		);

		self.data.reserve(rhs.len);
		self.data.extend_from_slice(&rhs.data);
		self.len += rhs.len;
	}
}

impl ByteString {
	/// Creates an empty byte string.
	#[inline]
	pub const fn new() -> Self {
		Self {
			is_utf8: Cell::new(true),
			data: Vec::new(),
		}
	}

	/// Returns the length in bytes of the byte string.
	#[inline]
	pub fn len(&self) -> usize { self.data.len() }
	/// Returns `true` if the byte string is empty.
	#[inline]
	pub fn is_empty(&self) -> bool { self.len() == 0 }
	/// Returns `true` if the byte string is not empty.
	#[inline]
	pub fn is_not_empty(&self) -> bool { self.len() > 0 }

	/// Returns the byte at `index`, or `None` if `index` is out of bounds.
	pub fn get(&mut self, index: usize) -> Option<u8> {
		self.data.get(index).cloned()
	}

	/// Returns a slice of the byte string bounded by `range`.
	pub fn range<R: RangeBounds<usize>>(&self, range: R) -> ByteStr<'_> {
		let range = slice::range(range, ..self.len());
		let utf8 = self.checked_utf8().and_then(|str|
			range.clone()
				 .get(str)
				 .map(Into::into)
		);
		let slice = &self.as_slice()[range];
		ByteStr {
			utf8,
			data: vec![slice],
			len: slice.len(),
		}
	}

	/// Returns a string of UTF-8-decoded bytes.
	pub fn utf8(&self) -> Result<&str, Utf8Error> {
		if self.is_utf8.get() {
			// The byte string was created from valid UTF-8, skip the check.
			Ok(self.utf8_unchecked())
		} else {
			let utf8 = from_utf8(&self.data)?;
			self.is_utf8.set(true);
			Ok(utf8)
		}
	}

	/// Appends `slice` to the byte string.
	pub fn extend_from_slice(&mut self, slice: &[u8]) {
		self.set_utf8(false);
		self.data.extend_from_slice(slice);
	}

	/// Appends `slice` to the byte string.
	pub fn extend_from_str(&mut self, slice: &str) {
		self.data.extend_from_slice(slice.as_bytes());
	}

	/// Borrows the data into a [`ByteStr`].
	pub fn as_byte_str(&self) -> ByteStr<'_> {
		ByteStr {
			utf8: self.checked_utf8().map(Into::into),
			data: vec![&self.data],
			len: self.len(),
		}
	}

	/// Returns the internal data as a slice of bytes.
	pub fn as_slice(&self) -> &[u8] { self.data.as_slice() }
	/// Returns the internal data.
	pub fn into_bytes(self) -> Vec<u8> { self.data }
	/// Returns the internal data as a UTF-8 string.
	pub fn into_utf8(self) -> Result<String, Utf8Error> {
		self.utf8()?;
		Ok(self.into_utf8_unchecked())
	}
}

impl ByteString {
	fn checked_utf8(&self) -> Option<&str> {
		self.is_utf8
			.get()
			.then(|| self.utf8_unchecked())
	}

	fn into_utf8_unchecked(self) -> String {
		unsafe {
			String::from_utf8_unchecked(self.data)
		}
	}

	fn utf8_unchecked(&self) -> &str {
		unsafe {
			from_utf8_unchecked(&self.data)
		}
	}

	fn set_utf8(&mut self, value: bool) {
		*self.is_utf8.get_mut() = value;
	}
}

impl Default for ByteString {
	#[inline]
	fn default() -> Self {
		Self::new()
	}
}

impl fmt::Debug for ByteString {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let mut repr = f.debug_struct("ByteString");
		if let Some(utf8) = self.checked_utf8() {
			repr.field("data", &utf8);
		} else {
			repr.field("data", &self.data);
		}
		repr.finish()
	}
}

impl<I: SliceIndex<[u8]>> Index<I> for ByteString {
	type Output = I::Output;

	fn index(&self, index: I) -> &Self::Output {
		index.index(self.as_slice())
	}
}

impl Eq for ByteString { }

impl PartialEq for ByteString {
	fn eq(&self, other: &Self) -> bool {
		self.data == other.data
	}
}

impl<'a> PartialEq<ByteStr<'a>> for ByteString {
	fn eq(&self, other: &ByteStr<'a>) -> bool {
		other == self.as_slice()
	}
}

impl Ord for ByteString {
	fn cmp(&self, Self { data, .. }: &Self) -> Ordering {
		self.data.cmp(data)
	}
}

impl PartialOrd for ByteString {
	fn partial_cmp(&self, Self { data, .. }: &Self) -> Option<Ordering> {
		self.data.partial_cmp(data)
	}
}

impl Extend<u8> for ByteString {
	fn extend<T: IntoIterator<Item = u8>>(&mut self, iter: T) {
		self.set_utf8(false);
		self.data.extend(iter)
	}
}

#[cfg(test)]
mod test {
	use base16ct::{lower, upper};
	use base64::Engine;
	use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
	use quickcheck::TestResult;
	use quickcheck_macros::quickcheck;
	use crate::{ByteStr, ByteString, EncodeBytes};

	#[quickcheck]
	fn same_size_eq(data: Vec<u8>) {
		let a1 = ByteStr::from(&*data);
		let b1 = ByteStr::from(&*data);
		assert_eq!(a1, b1, "ByteStr == ByteStr");
		let a2 = ByteString::from(data.clone());
		let b2 = ByteString::from(data.clone());
		assert_eq!(a2, b2, "ByteString == ByteString");
		assert_eq!(a1, a2, "ByteStr == ByteString");
		assert_eq!(a2, a1, "ByteString == ByteStr");
	}

	#[quickcheck]
	fn split_eq(data: Vec<u8>, split: usize) -> TestResult {
		if split >= data.len() {
			return TestResult::discard()
		}

		let (a, b) = data.split_at(split);
		let split_str = ByteStr::from(vec![a, b]);
		let full_str = ByteStr::from(&*data);
		let owned_str = ByteString::from(data.clone());

		assert_eq!(split_str, full_str, "ByteStr (split) == ByteStr (full)");
		assert_eq!(full_str, split_str, "ByteStr (full) == ByteStr (split)");
		assert_eq!(split_str, owned_str, "ByteStr (split) == ByteString");
		assert_eq!(owned_str, split_str,  "ByteString == ByteStr (split)");
		TestResult::passed()
	}

	#[quickcheck]
	fn encode_rolling(data: Vec<u8>, split: usize) -> TestResult {
		if split >= data.len() {
			return TestResult::discard()
		}

		let (a, b) = data.split_at(split);
		let bstr = ByteStr::from(a) + ByteStr::from(b);

		assert_eq!(
			bstr.base64_string(),
			STANDARD_NO_PAD.encode(&data),
			"standard base64"
		);
		assert_eq!(
			bstr.base64_url_string(),
			URL_SAFE_NO_PAD.encode(&data),
			"url-safe base64"
		);
		assert_eq!(
			bstr.hex_lower_string(),
			lower::encode_string(&data),
			"lowercase hex"
		);
		assert_eq!(
			bstr.hex_upper_string(),
			upper::encode_string(&data),
			"uppercase hex"
		);
		TestResult::passed()
	}
}
