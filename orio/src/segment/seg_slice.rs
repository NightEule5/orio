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
use std::iter::Peekable;
use std::ops::{Index, Range, RangeFrom, RangeInclusive, RangeTo, RangeToInclusive};
use std::slice::SliceIndex;
use simdutf8::compat::{from_utf8, Utf8Error};
use crate::segment::Segment;

/// A group of [`Segment`]s that can act as a slice of bytes. While segments are
/// not contiguous, they can implement much of the same operations as native slices.
#[derive(Copy, Clone, Default)]
pub struct SegSlice<'a> {
	start: usize,
	end  : usize,
	slice: &'a [Segment],
}

impl SegSlice<'_> {
	pub(crate) fn new(start: usize, end: usize, slice: &[Segment]) -> Self {
		Self { start, end, slice }
	}
	
	pub(crate) fn inner(&self) -> &[Segment] { self.slice }

	/// Returns the length.
	pub fn len(&self) -> usize { self.end - self.start }
	/// Returns the start index.
	pub fn start(&self) -> usize { self.start }
	/// Returns the end index.
	pub fn end  (&self) -> usize { self.end   }

	/// Gets the byte at `index`, or `None` if out of bounds.
	pub fn get(&self, mut index: usize) -> Option<u8> {
		for data in self.slice_iter() {
			let len = data.len();
			if index < len {
				return Some(data[index])
			}

			index -= len;
		}

		None
	}

	/// Iterates over bytes.
	pub fn iter(&self) -> impl Iterator<Item = u8> {
		self.slice_iter()
			.flatten()
			.cloned()
	}

	/// Iterates over segments as byte slices.
	fn slice_iter(&self) -> impl Iterator<Item = &[u8]> {
		let Self { start, end, slice } = self;

		struct Iter<I> {
			start: usize,
			end  : usize,
			iter : Peekable<I>,
		}

		impl<'a, I: Iterator<Item = &'a [u8]>> Iterator for Iter<I> {
			type Item = &'a [u8];

			fn next(&'a mut self) -> Option<Self::Item> {
				let next = self.iter.next()?;
				let is_last = self.iter.peek().is_none();

				let start = self.start;
				self.start = 0;

				Some(
					if is_last {
						&next[start..self.end]
					} else {
						&next[start..]
					}
				)
			}
		}

		Iter {
			start: *start,
			end  : *end,
			iter : slice.iter()
						.map(|seg| seg.mem.data())
						.peekable()
		}
	}

	/// Finds the index of a `byte`.
	pub fn find_byte(&self, byte: u8) -> Option<usize> {
		self.iter().position(|b| b == byte)
	}

	/// Finds the byte index of a UTF-8 [`char`], returning `None` if invalid UTF-8
	/// is encountered.
	pub fn find_utf8_char(&self, char: char) -> Option<usize> {
		let mut off = 0;
		for text in self.decode_utf8() {
			if let Some(pos) = text.find(char) {
				return Some(off + pos)
			}

			off += text.len();
		}
		None
	}

	/// Returns a slice up to invalid UTF-8, and the decode error if any.
	pub fn valid_utf8(&self) -> (Self, Option<Utf8Error>) {
		let mut end = 0;
		let mut err = None;
		for data in self.slice_iter() {
			if let Err(error) = from_utf8(data) {
				end += error.valid_up_to();
				err = Some(error);
				break
			} else {
				end += data.len();
			}
		}
		(self[..end], err)
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
		self.slice_iter().map(|data| from_utf8(data).expect("invalid UTF-8"))
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
}

impl IntoIterator for SegSlice<'_> {
	type Item = u8;
	type IntoIter = impl Iterator<Item = u8>;

	fn into_iter(self) -> Self::IntoIter { self.iter() }
}

impl Index<usize> for SegSlice<'_> {
	type Output = u8;

	fn index(&self, index: usize) -> &u8 {
		let Some(ref value) = self.get(index) else {
			panic!("index {index} out of bounds")
		};
		value
	}
}

impl Index<Range<usize>> for SegSlice<'_> {
	type Output = Self;

	fn index(&self, index: Range<usize>) -> &Self::Output {
		let (s, e) = if index.is_empty() {
			(self.end, self.end)
		} else if !index.contains(&self.len()) {
			(self.start + index.start, self.start + index.end)
		} else {
			panic!("range {index} out of bounds")
		};

		&SegSlice::new(s, e, self.slice)
	}
}

impl Index<RangeTo<usize>> for SegSlice<'_> {
	type Output = Self;

	fn index(&self, index: RangeTo<usize>) -> &Self::Output {
		&self[0..index.end]
	}
}

impl Index<RangeFrom<usize>> for SegSlice<'_> {
	type Output = Self;

	fn index(&self, index: RangeFrom<usize>) -> &Self::Output {
		&self[index.start..self.len()]
	}
}

impl Index<RangeInclusive<usize>> for SegSlice<'_> {
	type Output = Self;

	fn index(&self, index: RangeInclusive<usize>) -> &Self::Output {
		&self[index.start()..index.end() + 1]
	}
}

impl Index<RangeToInclusive<usize>> for SegSlice<'_> {
	type Output = Self;

	fn index(&self, index: RangeToInclusive<usize>) -> &Self::Output {
		&self[0..index.end + 1]
	}
}
