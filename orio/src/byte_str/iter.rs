// SPDX-License-Identifier: Apache-2.0

use std::iter::{Copied, Flatten, FusedIterator};
use std::slice::Iter;

pub type Slices<'a, 'b> = Copied<Iter<'b, &'a [u8]>>;

#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct Bytes<'a, 'b> {
	iter: Flatten<Slices<'a, 'b>>,
	len: usize
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
