// SPDX-License-Identifier: Apache-2.0

use std::borrow::Cow;
use std::cmp::min;
use std::iter::{Copied, Flatten, FusedIterator};
use std::ops::Range;
use std::slice::Iter;
use super::ByteStr;

pub type Slices<'a, 'b> = Copied<Iter<'b, &'a [u8]>>;

#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct Bytes<'a, 'b> {
	iter: Flatten<Slices<'a, 'b>>,
	len: usize
}

#[must_use = "iterators are lazy and do nothing unless consumed"]
pub(super) struct SlicesInRange<'a, 'b> {
	iter: Slices<'a, 'b>,
	start: usize,
	count: usize
}

impl<'a, 'b> Bytes<'a, 'b> {
	pub(super) fn new(slices: Slices<'a, 'b>, len: usize) -> Self {
		Self {
			iter: slices.flatten(),
			len
		}
	}
}

impl<'a: 'b, 'b> Iterator for Bytes<'a, 'b> {
	type Item = &'b u8;

	fn next(&mut self) -> Option<Self::Item> {
		self.len = self.len.saturating_sub(1);
		self.iter.next()
	}

	fn size_hint(&self) -> (usize, Option<usize>) {
		(self.len, Some(self.len))
	}
}

impl<'a: 'b, 'b> DoubleEndedIterator for Bytes<'a, 'b> {
	fn next_back(&mut self) -> Option<Self::Item> {
		let prev = self.iter.next_back()?;
		self.len += 1;
		Some(prev)
	}
}

impl<'a: 'b, 'b> ExactSizeIterator for Bytes<'a, 'b> { }
impl<'a: 'b, 'b> FusedIterator for Bytes<'a, 'b> { }
// unsafe impl<'a: 'b, 'b> TrustedLen for Bytes<'a, 'b> { }

impl<'a, 'b> SlicesInRange<'a, 'b> {
	pub fn new(range: Range<usize>, iter: Slices<'a, 'b>) -> Self {
		Self {
			iter,
			start: range.start,
			count: range.len()
		}
	}

	pub fn into_byte_str(self, utf8: Option<Cow<'a, str>>) -> ByteStr<'a> {
		let len = self.count;
		let mut data = Vec::with_capacity(self.iter.len());
		data.extend(self);
		ByteStr { data, utf8, len }
	}
}

impl<'a: 'b, 'b> Iterator for SlicesInRange<'a, 'b> {
	type Item = &'a [u8];

	fn next(&mut self) -> Option<Self::Item> {
		self.iter.find_map(|mut slice|
			if slice.len() <= self.start {
				self.start -= slice.len();
				None
			} else {
				let len = min(slice.len() - self.start, self.count);
				slice = &slice[self.start..][..len];
				self.start = 0;
				self.count -= len;
				Some(slice)
			}
		)
	}
}
