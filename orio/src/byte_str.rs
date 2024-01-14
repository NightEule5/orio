// SPDX-License-Identifier: Apache-2.0

mod hash;
mod encoding;

use std::borrow::{Borrow, BorrowMut, Cow};
use std::ops::{Add, Index, IndexMut, RangeBounds};
use std::{fmt, slice};
use std::slice::SliceIndex;
use simdutf8::compat::{from_utf8};
use crate::segment::RBuf;
use crate::{Seg, Utf8Error};
use crate::util::partial_utf8::read_partial_utf8_into;
pub use encoding::EncodeBytes;

/// A borrowed, segmented string of bytes.
#[derive(Clone, Eq)]
pub struct ByteStr<'a> {
	utf8: Option<Cow<'a, str>>,
	data: Vec<&'a [u8]>,
	len: usize,
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

impl<'a> Default for ByteStr<'a> {
	fn default() -> Self {
		Self {
			utf8: Some(Cow::Borrowed("")),
			data: Vec::new(),
			len: 0,
		}
	}
}

impl ByteStr<'_> {
	/// Creates an empty byte string.
	#[inline]
	pub fn new() -> Self { Self::default() }

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
		let Self { utf8, data, len } = self;
		let range = slice::range(range, ..*len);
		let utf8 = utf8.as_ref().map(|str| (&str[range.clone()]).into());

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
	pub fn utf8_cache(&mut self) -> Result<&str, Utf8Error> {
		Ok(
			match self.utf8 {
				Some(ref utf8) => utf8,
				None => self.utf8.insert(self.utf8()?.into())
			}
		)
	}

	/// Returns a string of UTF-8-decoded bytes, cloning from the value cached by
	/// [`utf8_cache`][] if any.
	///
	/// [`utf8_cache`]: Self::utf8_cache
	pub fn utf8(&self) -> Result<String, Utf8Error> {
		let Some(utf8) = self.utf8.as_ref() else {
			let mut buf = String::with_capacity(self.len);
			read_partial_utf8_into(self.data.iter().copied(), &mut buf)?;
			return Ok(buf)
		};

		Ok(utf8.clone().into_owned())
	}

	/// Returns the cached UTF-8 representation of the data, or `None` if the data
	/// has not been decoded.
	pub fn cached_utf8(&self) -> Option<&str> {
		self.utf8.as_deref()
	}

	/// Clones the borrowed data into an owned [`ByteString`].
	pub fn to_byte_string(&self) -> ByteString {
		self.data.iter().fold(
			Vec::with_capacity(self.len),
			|mut acc, cur| {
				acc.extend_from_slice(cur);
				acc
			}
		).into()
	}
}

impl<'a> ByteStr<'a> {
	/// Returns the internal data.
	pub fn into_vec(self) -> Vec<&'a [u8]> {
		self.data
	}

	pub(crate) fn iter(&self) -> impl Iterator<Item = &'a [u8]> + '_ {
		self.data
			.iter()
			.copied()
	}
}

impl fmt::Debug for ByteStr<'_> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		let mut dbg = f.debug_struct("ByteStr");
		if let Some(utf8) = self.cached_utf8() {
			dbg.field("data", &utf8);
		} else {
			dbg.field("data", &self.data);
		}
		dbg.finish_non_exhaustive()
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

impl PartialEq for ByteStr<'_> {
	fn eq(&self, other: &Self) -> bool {
		if let (&[a], &[b]) = (self.as_ref(), other.as_ref()) {
			a == b
		} else if self.len != other.len {
			false
		} else {
			for (a, b) in self.iter().flatten().zip(other.iter().flatten()) {
				if a != b {
					return false
				}
			}

			true
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
			for (a, b) in self.iter().flatten().zip(other) {
				if a != b {
					return false
				}
			}

			true
		}
	}
}

impl PartialEq<ByteString> for ByteStr<'_> {
	fn eq(&self, other: &ByteString) -> bool {
		self == other.as_ref()
	}
}

impl<'a> From<Vec<&'a [u8]>> for ByteStr<'a> {
	fn from(data: Vec<&'a [u8]>) -> Self {
		let len = data.iter().copied().map(<[u8]>::len).sum();
		Self {
			utf8: None,
			data,
			len,
		}
	}
}

impl<'a> From<&'a [u8]> for ByteStr<'a> {
	fn from(value: &'a [u8]) -> Self {
		Self::from(vec![value])
	}
}

impl<'a, const N: usize> From<&'a RBuf<Seg<'a, N>>> for ByteStr<'a> {
	fn from(value: &'a RBuf<Seg<'a, N>>) -> Self {
		Self {
			utf8: None,
			data: value.iter_slices().collect(),
			len: value.count(),
		}
	}
}

impl<'a> FromIterator<&'a [u8]> for ByteStr<'a> {
	fn from_iter<T: IntoIterator<Item = &'a [u8]>>(iter: T) -> Self {
		let data: Vec<_> = iter.into_iter().collect();
		data.into()
	}
}

impl<'a> AsRef<[&'a [u8]]> for ByteStr<'a> {
	fn as_ref(&self) -> &[&'a [u8]] {
		&self.data
	}
}

impl<'a> Borrow<[&'a [u8]]> for ByteStr<'a> {
	fn borrow(&self) -> &[&'a [u8]] {
		&self.data
	}
}

impl Add for ByteStr<'_> {
	type Output = Self;

	fn add(mut self, Self { ref utf8, ref data, len }: Self) -> Self {
		self.utf8 = if let Some(utf8_a) = self.utf8 {
			if let Some(utf8_b) = utf8 {
				Some(utf8_a.into_owned().add(utf8_b).into())
			} else {
				None
			}
		} else {
			None
		};

		self.data.extend_from_slice(data);
		self.len += len;
		self
	}
}

/// An owned string of bytes.
#[derive(Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ByteString {
	data: Vec<u8>
}

impl ByteString {
	/// Creates an empty byte string.
	#[inline]
	pub fn new() -> Self { Self::default() }

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
		let slice = &self.as_slice()[range];
		ByteStr {
			utf8: None,
			data: vec![slice],
			len: slice.len(),
		}
	}

	/// Returns a string of UTF-8-decoded bytes.
	pub fn utf8(&self) -> Result<&str, Utf8Error> {
		from_utf8(&self.data).map_err(Into::into)
	}

	/// Appends `slice` to the byte string.
	pub fn extend_from_slice(&mut self, slice: &[u8]) {
		self.data.extend_from_slice(slice);
	}

	/// Borrows the data into a [`ByteStr`].
	pub fn as_byte_str(&self) -> ByteStr<'_> {
		ByteStr {
			utf8: None,
			data: vec![&self.data],
			len: self.len(),
		}
	}

	/// Returns the internal data as a slice of bytes.
	pub fn as_slice(&self) -> &[u8] { self.data.as_slice() }
	/// Returns the internal data as a mutable slice of bytes.
	pub fn as_mut_slice(&mut self) -> &mut [u8] { self.data.as_mut_slice() }
	/// Returns the internal data.
	pub fn into_bytes(self) -> Vec<u8> { self.data }
}

impl<I: SliceIndex<[u8]>> Index<I> for ByteString {
	type Output = I::Output;

	fn index(&self, index: I) -> &Self::Output {
		index.index(self.as_slice())
	}
}

impl<I: SliceIndex<[u8]>> IndexMut<I> for ByteString {
	fn index_mut(&mut self, index: I) -> &mut Self::Output {
		index.index_mut(self.as_mut_slice())
	}
}

impl<'a> PartialEq<ByteStr<'a>> for ByteString {
	fn eq(&self, other: &ByteStr<'a>) -> bool {
		other == self.as_slice()
	}
}

impl<'a> From<&'a ByteString> for ByteStr<'a> {
	fn from(value: &'a ByteString) -> Self {
		Self::from(value.as_slice())
	}
}

impl<'a> From<&ByteStr<'a>> for ByteString {
	fn from(value: &ByteStr<'a>) -> Self {
		value.to_byte_string()
	}
}

impl From<Vec<u8>> for ByteString {
	fn from(data: Vec<u8>) -> Self {
		Self { data }
	}
}

impl From<&[u8]> for ByteString {
	fn from(value: &[u8]) -> Self {
		value.to_vec().into()
	}
}

impl FromIterator<u8> for ByteString {
	fn from_iter<T: IntoIterator<Item = u8>>(iter: T) -> Self {
		iter.into_iter()
			.collect::<Vec<_>>()
			.into()
	}
}

impl<'a> FromIterator<&'a [u8]> for ByteString {
	fn from_iter<T: IntoIterator<Item = &'a [u8]>>(iter: T) -> Self {
		iter.into_iter()
			.flatten()
			.copied()
			.collect()
	}
}

impl Borrow<[u8]> for ByteString {
	fn borrow(&self) -> &[u8] {
		&self.data
	}
}

impl BorrowMut<[u8]> for ByteString {
	fn borrow_mut(&mut self) -> &mut [u8] {
		&mut self.data
	}
}

impl AsRef<[u8]> for ByteString {
	fn as_ref(&self) -> &[u8] { &self.data }
}

impl AsMut<[u8]> for ByteString {
	fn as_mut(&mut self) -> &mut [u8] { &mut self.data }
}

impl Extend<u8> for ByteString {
	fn extend<T: IntoIterator<Item = u8>>(&mut self, iter: T) {
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
