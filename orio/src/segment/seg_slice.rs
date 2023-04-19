// Copyright 2023 Strixpyrr
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::cmp::min;
use std::collections::Bound;
use std::error::Error;
use std::fmt;
use std::fmt::{Display, Formatter};
use std::ops::{Index, Range, RangeBounds};
use std::str::pattern::Pattern;
use itertools::FoldWhile::{Continue, Done};
use itertools::Itertools;
use simdutf8::compat::{from_utf8, Utf8Error};
use crate::segment::Segment;

/// A group of shared [`Segment`]s the can act as a pseudo-slice of bytes.
pub struct SegmentSlice {
	len: usize,
	segments: Vec<Segment>
}

impl SegmentSlice {
	pub(crate) fn new(segments: &[Segment], range: Range<usize>) -> Self {
		Self {
			len: range.len(),
			segments: {
				if range.is_empty() {
					Vec::new()
				} else {
					let mut vec: Vec<_> = segments.iter()
												  .map(Segment::share_all)
												  .collect();
					let Range { mut start, mut end, .. } = range;

					for seg in vec.iter_mut() {
						let n = min(seg.len(), start);
						start -= n;

						seg.consume(n);

						if start == 0 {
							break
						}
					}

					for seg in vec.iter_mut().rev() {
						let n = min(seg.len(), end);
						end -= n;

						seg.truncate(n);

						if end == 0 {
							break
						}
					}

					vec.retain(|seg| !seg.is_empty());
					vec
				}
			}
		}
	}

	/// Returns the length.
	pub fn len(&self) -> usize { self.len }

	pub(crate) fn into_inner(self) -> Vec<Segment> { self.segments }

	/// Gets the byte at `index`, or `None` if out of bounds.
	pub fn get(&self, mut index: usize) -> Option<&u8> {
		for data in self.slice_iter() {
			let len = data.len();
			if index < len {
				return Some(&data[index])
			}

			index -= len;
		}

		None
	}

	/// Returns a new slice within `range`.
	pub fn slice<R: RangeBounds<usize>>(&self, range: R) -> Self {
		let start = match range.start_bound() {
			Bound::Included(start) => *start,
			Bound::Excluded(start) => *start + 1,
			Bound::Unbounded => 0
		};
		let end = match range.end_bound() {
			Bound::Included(end) => *end + 1,
			Bound::Excluded(end) => *end,
			Bound::Unbounded => self.len
		};
		Self::new(&self.segments, start..end)
	}

	/// Finds the index of a `byte`.
	pub fn find_byte(&self, byte: u8) -> Option<usize> {
		self.iter().position(|b| b == byte)
	}

	/// Finds the byte index of a UTF-8 pattern, returning `None` and a UTF-8 error
	/// (if any) if invalid UTF-8 or the end of the slice is encountered before the
	/// pattern is found.
	pub fn find_utf8<'a>(&'a self, pat: impl Pattern<'a> + Clone) -> Option<usize> {
		let Done(pos) = self.decode_utf8()
							.fold_while(0, |off, text|
								if let Some(pos) = text.find(pat.clone()) {
									Done(off + pos)
								} else {
									Continue(off + text.len())
								}
							) else { return None };
		Some(pos)
	}

	/// Returns the exclusive byte index of the last valid UTF-8, and the error, if
	/// any.
	pub fn valid_utf8(&self) -> (Self, Option<OffsetUtf8Error>) {
		let mut end = 0;
		for data in self.slice_iter() {
			if let Err(error) = from_utf8(data) {
				return (
					self.slice(..end + error.valid_up_to()),
					Some(OffsetUtf8Error::new(error, end))
				)
			} else {
				end += data.len();
			}
		}
		(self.slice(..end), None)
	}

	/// Decodes the slice as UTF-8.
	///
	/// # Panics
	///
	/// Panics when the slice contains invalid UTF-8. Use [`Self::valid_utf8`] to
	/// get a slice of valid UTF-8 first.
	pub fn decode_utf8(&self) -> impl Iterator<Item = &str> {
		// Todo: If more bytes are needed to complete a codepoint at the end of
		//  a segment, this will fail. For ASCII, this will work.
		self.slice_iter()
			.map(|data| from_utf8(data).expect("invalid UTF-8"))
	}

	/// Copies all bytes into `dst`.
	///
	/// # Panics
	/// Panics if the two slices have different lengths.
	pub fn copy_into_slice(&self, mut dst: &mut [u8]) {
		assert_eq!(dst.len(), self.len(), "Both slices must have the same length");

		for data in self.slice_iter() {
			let n = min(data.len(), dst.len());
			dst.copy_from_slice(&data[..n]);
			dst = &mut dst[n..];

			if dst.is_empty() {
				break
			}
		}
	}

	/// Iterates over bytes.
	pub fn iter(&self) -> impl Iterator<Item = u8> + '_ {
		self.slice_iter()
			.flatten()
			.cloned()
	}

	/// Iterates over segments as byte slices.
	fn slice_iter(&self) -> impl Iterator<Item = &[u8]> {
		self.segments
			.iter()
			.map(Segment::data)
	}
}

impl Index<usize> for SegmentSlice {
	type Output = u8;

	fn index(&self, index: usize) -> &u8 {
		let Some(value) = self.get(index) else {
			panic!("index {index} out of bounds")
		};
		value
	}
}

#[derive(Copy, Clone, Debug)]
pub struct OffsetUtf8Error {
	inner: Utf8Error,
	offset: usize
}

impl OffsetUtf8Error {
	fn new(inner: Utf8Error, offset: usize) -> Self {
		Self { inner, offset }
	}

	pub fn into_inner(self) -> Utf8Error { self.inner }

	pub fn valid_up_to(&self) -> usize {
		self.offset + self.inner.valid_up_to()
	}

	pub fn error_len(&self) -> Option<usize> {
		self.inner.error_len()
	}
}

impl Display for OffsetUtf8Error {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		if let Some(error_len) = self.error_len() {
			write!(
				f,
				"invalid utf-8 sequence of {error_len} bytes from index {}",
				self.valid_up_to()
			)
		} else {
			write!(
				f,
				"incomplete utf-8 byte sequence from index {}",
				self.valid_up_to()
			)
		}
	}
}

impl Error for OffsetUtf8Error {
	fn source(&self) -> Option<&(dyn Error + 'static)> {
		Some(&self.inner)
	}
}
