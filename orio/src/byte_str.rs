// SPDX-License-Identifier: Apache-2.0

//! Types containing borrowed-segmented or owned sequences of bytes, called *byte
//! strings*. These can be decoded to UTF-8, hashed, sliced, split, pattern matched,
//! and encoded to or decoded from base64 and hex strings. [`ByteStr`] is analogous
//! to [`str`], containing borrowed bytes, but can contain multiple slices. [`ByteString`]
//! is analogous to [`String`], containing a contiguous sequence of bytes.

mod conv;
mod decoding;
mod encoding;
mod hash;
mod iter;
mod parsing;

use std::borrow::{Borrow, Cow};
use std::ops::{Add, AddAssign, Deref, DerefMut, Index, Range, RangeBounds};
use std::{fmt, mem, slice};
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::iter::once;
use all_asserts::assert_le;
use simdutf8::compat::from_utf8;
use crate::Utf8Error;
use crate::util::partial_utf8::read_partial_utf8_into;
use crate::pattern::Pattern;
pub use encoding::EncodeBytes;
pub use iter::*;
pub use hash::*;
pub use parsing::*;

/// A borrowed, segmented string of bytes.
#[derive(Clone)]
pub struct ByteStr<'a> {
	data: Vec<&'a [u8]>,
	utf8: Option<Cow<'a, str>>,
	len: usize,
}

/// An owned string of bytes.
#[derive(Clone, amplify_derive::From)]
pub struct ByteString {
	// Todo: change this to a "valid range", to allow invalid bytes to be pushed but
	//  still skip the check for valid UTF-8 bytes?
	#[from]
	#[from(Vec<u8>)]
	#[from(String)]
	data: Data,
}

#[derive(Clone, amplify_derive::From)]
enum Data {
	Bytes(#[from] Vec<u8>),
	String(#[from] String)
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
	pub fn range<R: RangeBounds<usize>>(&self, range: R) -> ByteStr<'a> {
		let range = slice::range(range, ..self.len);
		let utf8 = self.utf8.as_ref().and_then(|str| {
			let range = range.clone();
			match str {
				&Cow::Borrowed(str) => str.get(range).map(Into::into),
				Cow::Owned(str) => str.get(range).map(|str| str.to_owned().into())
			}
		});
		self.slices_in_range(range)
			.into_byte_str(utf8)
	}

	/// Decodes and caches the bytes as UTF-8, returning a borrow of the cache.
	/// Subsequent calls to this function will borrow from the cache, and calls to
	/// [`utf8`] will clone from it.
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
	/// [`utf8_cache`] if any.
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

	/// Finds the first range matching `pattern` in the byte string.
	pub fn find(&self, pattern: impl Pattern) -> Option<Range<usize>> {
		match self.cached_utf8() {
			Some(utf8) => pattern.find_in_str(once(utf8)),
			_ => pattern.find_in(self.slices())
		}
	}

	/// Finds the first range matching `pattern` in the byte string, within a byte
	/// `range`.
	pub fn find_in_range<R: RangeBounds<usize>>(
		&self,
		pattern: impl Pattern,
		range: R
	) -> Option<Range<usize>> {
		let range = slice::range(range, ..self.len);
		match self.cached_utf8().and_then(|str|
			str.get(range.clone())
		) {
			Some(utf8) => pattern.find_in_str(once(utf8)),
			_ => pattern.find_in(self.slices_in_range(range))
		}
	}

	/// Iterates over ranges matching `pattern` in the byte string.
	pub fn matches<'b, P>(&'b self, pattern: P) -> impl Iterator<Item = Range<usize>> + 'b
						  where P: Pattern,
								P::Matcher: 'b {
		use std::iter::Once;
		use crate::pattern::{Matcher, Matches, StrMatches};
		enum _Matches<'a, 'b: 'a, M> {
			Bytes (Matches<'b, Slices<'a, 'b>, M>),
			String(StrMatches<'b, Once<&'b str>, M>)
		}

		impl<'a, M: Matcher> Iterator for _Matches<'a, '_, M> {
			type Item = Range<usize>;

			fn next(&mut self) -> Option<Self::Item> {
				match self {
					Self::Bytes (matches) => matches.next(),
					Self::String(matches) => matches.next()
				}
			}
		}

		match self.cached_utf8() {
			Some(utf8) => _Matches::String(pattern.matches_in_str(once(utf8))),
			_ => _Matches::Bytes(pattern.matches_in(self.slices()))
		}
	}

	/// Splits the byte string into a pair of borrowed strings at an index. The
	/// first contains bytes in range `[0, mid)` (with a length of `mid` bytes),
	/// the second contains bytes in range `[mid, len)`.
	///
	/// The returned bytes will contain valid UTF-8 if the current bytes are marked
	/// as valid UTF-8, and `mid` falls on a character boundary.
	pub fn split_at(&self, mid: usize) -> (ByteStr<'a>, ByteStr<'a>) {
		fn split<'a>(slices: &Vec<&'a [u8]>, mid: usize, len: usize) -> (Vec<&'a [u8]>, Vec<&'a [u8]>) {
			assert_le!(mid, len, "split index out of bounds");

			match &slices[..] {
				_ if mid == len => (slices.clone(), vec![]),
				_ if mid == 0   => (vec![], slices.clone()),
				[] => (vec![], vec![]),
				[slice] => {
					let (a, b) = slice.split_at(mid);
					(vec![a], vec![b])
				}
				slices => {
					// Find the index at which the total count reaches or exceeds
					// the midpoint, and this final count.
					let (boundary_index, boundary_count) =
						slices.iter().scan(0, |count, slice|
							(*count < mid).then(|| {
								*count += slice.len();
								*count
							})
						).enumerate()
						 .last()
						 .unwrap();
					if boundary_count == mid {
						let (a, b) = slices.split_at(boundary_index);
						(a.to_vec(), b.to_vec())
					} else {
						let mut first = slices[..=boundary_index].to_vec();
						let mut last  = slices[boundary_index.. ].to_vec();
						let [.., first_overlap] = &mut first[..] else { unreachable!() };
						let [last_overlap, .. ] = &mut last [..] else { unreachable!() };
						let overlap_len = first_overlap.len() - boundary_count - mid;
						*first_overlap = &first_overlap[..overlap_len];
						* last_overlap = & last_overlap[overlap_len..];
						(first, last)
					}
				}
			}
		}

		fn count(slices: &[&[u8]]) -> usize {
			slices.iter()
				  .copied()
				  .map(<[u8]>::len)
				  .sum()
		}

		let (data_a, data_b) = split(&self.data, mid, self.len);
		let (utf8_a, utf8_b) = self.utf8.as_ref().and_then(|str|
			str.is_char_boundary(mid).then(||
				match str {
					Cow::Borrowed(str) => {
						let (a, b) = str.split_at(mid);
						(a.into(), b.into())
					}
					Cow::Owned(str) => {
						let (a, b) = str.split_at(mid);
						(a.to_owned().into(), b.to_owned().into())
					}
				}
			)
		).unzip();
		(
			ByteStr::from_vec::<Cow<_>>(data_a.to_vec(), utf8_a, count(&data_a)),
			ByteStr::from_vec::<Cow<_>>(data_b.to_vec(), utf8_b, count(&data_b))
		)
	}

	/// Splits the byte string into two owned sequences, returning an allocated
	/// byte string containing bytes in range `[at, len)`, leaving the current one
	/// containing bytes in range `[0, at)`.
	///
	/// The bytes strings will contain valid UTF-8 if the current one is marked as
	/// valid UTF-8, and the index falls on a character boundary.
	pub fn split_off(&mut self, mid: usize) -> Self {
		fn split<'a>(slices: &mut Vec<&'a [u8]>, mid: usize, len: usize) -> Vec<&'a [u8]> {
			assert_le!(mid, len, "split index out of bounds");

			match &mut slices[..] {
				_ if mid == len => vec![],
				_ if mid == 0   => {
					let clone = slices.clone();
					slices.clear();
					clone
				}
				[slice] => {
					let (a, b) = slice.split_at(mid);
					*slice = a;
					vec![b]
				}
				_ => {
					// Find the index at which the total count reaches or exceeds
					// the midpoint, and this final count.
					let (boundary_index, boundary_count) =
						slices.iter().scan(0, |count, slice|
							(*count < mid).then(|| {
								*count += slice.len();
								*count
							})
						).enumerate()
						 .last()
						 .unwrap();
					if boundary_count == mid {
						slices.split_off(boundary_index)
					} else {
						slices.truncate(boundary_index + 1);
						let mut last  = slices[boundary_index.. ].to_vec();
						let [.., first_overlap] = &mut slices[..] else { unreachable!() };
						let [last_overlap, .. ] = &mut last  [..] else { unreachable!() };
						let overlap_len = first_overlap.len() - boundary_count - mid;
						*first_overlap = &first_overlap[..overlap_len];
						* last_overlap = & last_overlap[overlap_len..];
						last
					}
				}
			}
		}

		let utf8 = self.utf8.as_mut().and_then(|str|
			str.is_char_boundary(mid).then(||
				match str {
					Cow::Borrowed(str) => {
						let (a, b) = str.split_at(mid);
						*str = a;
						Cow::Borrowed(b)
					}
					Cow::Owned(str) => Cow::Owned(str.split_off(mid))
				}
			)
		);
		Self::from_vec(
			split(&mut self.data, mid, self.len),
			utf8,
			self.len
		)
	}

	/// Splits the byte string into a pair of borrowed sequences at the first match
	/// of a `delimiter` pattern, returning `None` if no match is found. The first
	/// contains bytes in range `[0, start)`, the second contains bytes in range
	/// `[end, len)`.
	pub fn split_once(&self, delimiter: impl Pattern) -> Option<(ByteStr, ByteStr)> {
		let Range { start, end } = self.find(delimiter)?;
		let (mut first, last) = self.split_at(end);
		first.truncate(start);
		Some((first, last))
	}

	/// Replaces all occurrences of a pattern with a slice, returning a new owned
	/// byte string.
	pub fn replace(&self, from: impl Pattern, to: &[u8]) -> ByteString {
		use simdutf8::basic::from_utf8;
		let is_utf8 = self.utf8.is_some() && from_utf8(to).is_ok();
		let mut data = Vec::with_capacity(self.len);
		let mut last = 0;
		for Range { start, end } in self.matches(from) {
			for slice in self.slices_in_range(last..start) {
				data.extend_from_slice(slice);
			}
			data.extend_from_slice(to);
			last = end;
		}

		for slice in self.slices_in_range(last..self.len) {
			data.extend_from_slice(slice);
		}

		ByteString::from_data(
			if is_utf8 {
				Data::from_utf8_unchecked(data)
			} else {
				Data::Bytes(data)
			}
		)
	}

	/// Shortens the byte string length to a maximum of `len` bytes.
	pub fn truncate(&mut self, mut len: usize) {
		let Self { data, utf8, .. } = self;

		match utf8 {
			Some(Cow::Borrowed(str)) if str.is_char_boundary(len) => *str = &str[..len],
			Some(Cow::Owned   (str)) if str.is_char_boundary(len) => str.truncate(len),
			_ => *utf8 = None
		}

		match &mut data[..] {
			[slice] if len < self.len => {
				*slice = &slice[..self.len];
				self.len = len;
			}
			_ if len < self.len =>
				// This creates no holes, it is equivalent to truncating.
				self.data.retain_mut(|slice|
					if len == 0 {
						false
					} else {
						if slice.len() > len {
							*slice = &slice[..len];
							len = 0;
						} else {
							len -= slice.len();
						}
						true
					}
				),
			_ => { }
		}
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
	fn from_slice(data: &'a [u8], utf8: Option<&'a str>) -> Self {
		Self::from_vec(vec![data], utf8, data.len())
	}

	fn from_vec<U: Into<Cow<'a, str>>>(data: Vec<&'a [u8]>, utf8: Option<U>, len: usize) -> Self {
		Self {
			data,
			utf8: utf8.map(Into::into),
			len
		}
	}

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

	fn slices_in_range(&self, range: Range<usize>) -> SlicesInRange<'a, '_> {
		SlicesInRange::new(range, self.slices())
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
		Self::from_data(Data::String(String::new()))
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
			str.get(range.clone())
		);
		let data = &self.data[range];
		ByteStr::from_slice(data, utf8)
	}

	/// Decodes the bytes as UTF-8, storing the result and returning the data as a
	/// string if successful. If the result is `Ok`, subsequent calls skip this
	/// check.
	pub fn check_utf8(&mut self) -> Result<&str, Utf8Error> {
		// ...wat?
		let Self { data } = self;
		if let Data::Bytes(bytes) = data {
			from_utf8(bytes)?;
			*data = Data::String(
				unsafe {
					String::from_utf8_unchecked(mem::take(bytes))
				}
			)
		}

		let Data::String(str) = data else {
			unsafe {
				std::hint::unreachable_unchecked()
			}
		};
		Ok(str)
	}

	/// Decodes the bytes as UTF-8.
	pub fn utf8(&self) -> Result<&str, Utf8Error> {
		match &self.data {
			Data::String(str) => Ok(str),
			Data::Bytes(bytes) => Ok(from_utf8(bytes)?)
		}
	}

	/// Returns the UTF-8 representation of the data checked by [`check_utf8`], or
	/// `None` if the data has not been checked.
	///
	/// [`check_utf8`]: Self::check_utf8
	pub fn checked_utf8(&self) -> Option<&str> {
		let Data::String(str) = &self.data else {
			return None
		};
		Some(str)
	}

	/// Finds the first range matching `pattern` in the byte string.
	pub fn find(&self, pattern: impl Pattern) -> Option<Range<usize>> {
		match self.checked_utf8() {
			Some(utf8) => pattern.find_in_str(once(utf8)),
			_ => pattern.find_in(once(&self.data[..]))
		}
	}

	/// Finds the first range matching `pattern` in the byte string, within a byte
	/// `range`.
	pub fn find_in_range<R: RangeBounds<usize>>(
		&self,
		pattern: impl Pattern,
		range: R
	) -> Option<Range<usize>> {
		let range = slice::range(range, ..self.data.len());
		match self.checked_utf8().and_then(|str|
			str.get(range.clone())
		) {
			Some(utf8) => pattern.find_in_str(once(utf8)),
			_ => pattern.find_in(once(&self.data[range]))
		}
	}

	/// Iterates over ranges matching `pattern` in the byte string.
	pub fn matches<'a, P>(&'a self, pattern: P) -> impl Iterator<Item = Range<usize>> + 'a
	where P: Pattern,
		  P::Matcher: 'a {
		use std::iter::Once;
		use crate::pattern::{Matcher, Matches, StrMatches};
		enum _Matches<'a, M> {
			Bytes (Matches<'a, Once<&'a [u8]>, M>),
			String(StrMatches<'a, Once<&'a str >, M>)
		}

		impl<M: Matcher> Iterator for _Matches<'_, M> {
			type Item = Range<usize>;

			fn next(&mut self) -> Option<Self::Item> {
				match self {
					Self::Bytes (matches) => matches.next(),
					Self::String(matches) => matches.next()
				}
			}
		}

		match self.checked_utf8() {
			Some(utf8) => _Matches::String(pattern.matches_in_str(once(utf8))),
			_ => _Matches::Bytes(pattern.matches_in(once(&self.data[..])))
		}
	}

	/// Splits the byte string into a pair of borrowed strings at an index. The
	/// first contains bytes in range `[0, mid)` (with a length of `mid` bytes),
	/// the second contains bytes in range `[mid, len)`.
	///
	/// The returned bytes will contain valid UTF-8 if the current bytes are marked
	/// as valid UTF-8, and `mid` falls on a character boundary.
	pub fn split_at(&self, mid: usize) -> (ByteStr, ByteStr) {
		let (data_a, data_b) = self.data.split_at(mid);
		let (utf8_a, utf8_b) = self.checked_utf8().and_then(|utf8| {
			utf8.is_char_boundary(mid)
				.then(|| utf8.split_at(mid))
		}).unzip();
		(
			ByteStr::from_slice(data_a, utf8_a),
			ByteStr::from_slice(data_b, utf8_b)
		)
	}

	/// Splits the byte string into two owned sequences, returning an allocated
	/// byte string containing bytes in range `[at, len)`, leaving the current one
	/// containing bytes in range `[0, at)`.
	///
	/// The bytes strings will contain valid UTF-8 if the current one is marked as
	/// valid UTF-8, and the index falls on a character boundary.
	pub fn split_off(&mut self, at: usize) -> Self {
		self.check_utf8_split(at);
		let split = self.data.split_off(at);
		let data = if matches!(&self.data, Data::String(_)) {
			Data::from_utf8_unchecked(split)
		} else {
			split.into()
		};
		Self { data }
	}

	/// Splits the byte string into a pair of borrowed sequences at the first match
	/// of a `delimiter` pattern, returning `None` if no match is found. The first
	/// contains bytes in range `[0, start)`, the second contains bytes in range
	/// `[end, len)`.
	pub fn split_once(&self, delimiter: impl Pattern) -> Option<(ByteStr, ByteStr)> {
		let Range { start, end } = self.find(delimiter)?;
		let (mut first, last) = self.split_at(end);
		first.truncate(start);
		Some((first, last))
	}

	/// Replaces all occurrences of a pattern with a slice, returning a new byte
	/// string.
	pub fn replace(&self, from: impl Pattern, to: &[u8]) -> Self {
		use simdutf8::basic::from_utf8;
		let mut data = Vec::with_capacity(self.len());
		let mut last = 0;
		for Range { start, end } in self.matches(from) {
			data.extend_from_slice(&self.data[last..start]);
			data.extend_from_slice(to);
			last = end;
		}

		data.extend_from_slice(&self.data[last..]);
		Data::new(data, self.data.is_utf8() && from_utf8(to).is_ok())
			.into()
	}

	/// Shortens the byte string length to a maximum of `len` bytes.
	pub fn truncate(&mut self, len: usize) {
		self.check_utf8_split(len);
		self.data.truncate(len);
	}

	/// Appends `slice` to the byte string.
	pub fn extend_from_slice(&mut self, slice: &[u8]) {
		self.unmark_utf8();
		self.data.extend_from_slice(slice);
	}

	/// Appends `slice` to the byte string.
	pub fn extend_from_str(&mut self, slice: &str) {
		self.data.extend_from_slice(slice.as_bytes());
	}

	/// Borrows the data into a [`ByteStr`].
	pub fn as_byte_str(&self) -> ByteStr<'_> {
		ByteStr::from_slice(&self.data, self.checked_utf8())
	}

	/// Returns the internal data as a slice of bytes.
	pub fn as_slice(&self) -> &[u8] { self.data.as_slice() }
	/// Returns the internal data.
	pub fn into_bytes(self) -> Vec<u8> {
		match self.data {
			Data::Bytes(bytes) => bytes,
			Data::String(utf8) => utf8.into_bytes()
		}
	}
	/// Returns the internal data as a UTF-8 string.
	pub fn into_utf8(self) -> Result<String, Utf8Error> {
		self.utf8()?;
		Ok(self.into_utf8_unchecked())
	}
}

impl Borrow<[u8]> for Data {
	#[inline]
	fn borrow(&self) -> &[u8] {
		match self {
			Self::Bytes(vec) => vec,
			Self::String(str) => str.as_bytes()
		}
	}
}

impl AsRef<[u8]> for Data {
	#[inline]
	fn as_ref(&self) -> &[u8] {
		self
	}
}

impl Deref for Data {
	type Target = Vec<u8>;

	#[inline]
	fn deref(&self) -> &Vec<u8> {
		match self {
			Self::Bytes(vec) => vec,
			Self::String(str) => unsafe {
				// Safety: String and Vec<u8> have the same memory layout.
				mem::transmute(str)
			}
		}
	}
}

impl DerefMut for Data {
	#[inline]
	fn deref_mut(&mut self) -> &mut Vec<u8> {
		match self {
			Self::Bytes(vec) => vec,
			Self::String(str) => unsafe {
				// Safety: data is checked before mutating the string.
				str.as_mut_vec()
			}
		}
	}
}

impl Data {
	fn new(data: Vec<u8>, is_utf8: bool) -> Self {
		if is_utf8 {
			Self::from_utf8_unchecked(data)
		} else {
			data.into()
		}
	}

	fn from_utf8_unchecked(data: Vec<u8>) -> Self {
		unsafe {
			Self::String(String::from_utf8_unchecked(data))
		}
	}

	fn is_utf8(&self) -> bool {
		matches!(self, Self::String(_))
	}

	fn unmark_utf8(&mut self) {
		*self = self.take_bytes().into();
	}

	fn take_bytes(&mut self) -> Vec<u8> {
		match self {
			Self::Bytes(bytes) => mem::take(bytes),
			Self::String(str) => mem::take(str).into_bytes()
		}
	}
}

impl ByteString {
	#[inline]
	const fn from_data(data: Data) -> Self {
		Self { data }
	}

	fn into_utf8_unchecked(self) -> String {
		match self.data {
			Data::Bytes(bytes) => unsafe {
				String::from_utf8_unchecked(bytes)
			},
			Data::String(utf8) => utf8
		}
	}

	fn check_utf8_split(&mut self, idx: usize) {
		match &self.data {
			Data::String(str) if !str.is_char_boundary(idx) => self.unmark_utf8(),
			_ => { }
		}
	}

	fn unmark_utf8(&mut self) {
		self.data.unmark_utf8();
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
		let repr = match &self.data {
			Data::Bytes (bytes) => repr.field("data", bytes),
			Data::String(str  ) => repr.field("data", &str)
		};
		repr.finish()
	}
}

impl<I> Index<I> for ByteString where [u8]: Index<I> {
	type Output = <[u8] as Index<I>>::Output;

	#[inline]
	fn index(&self, index: I) -> &Self::Output {
		self.as_slice().index(index)
	}
}

impl Eq for ByteString { }

impl PartialEq for ByteString {
	fn eq(&self, other: &Self) -> bool {
		&*self.data == &*other.data
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
		self.data.deref().partial_cmp(data.deref())
	}
}

impl Extend<u8> for ByteString {
	fn extend<T: IntoIterator<Item = u8>>(&mut self, iter: T) {
		self.unmark_utf8();
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
