// SPDX-License-Identifier: Apache-2.0

use std::cmp::min;
use std::collections::VecDeque;
use std::ops::{Index, IndexMut};
use super::Seg;

/// A shareable, segmented ring buffer. Cloning shares segments in linear (`O(n)`)
/// time.
#[derive(Clone, Debug, Eq)]
pub(crate) struct RBuf<T> {
	pub(crate) buf: VecDeque<T>,
	/// The number of readable segments in the buffer.
	len: usize,
	/// The number of readable bytes in the buffer.
	count: usize,
}

impl<T> Default for RBuf<T> {
	fn default() -> Self {
		Self::new()
	}
}

impl<'d, const N: usize> From<Vec<Seg<'d, N>>> for RBuf<Seg<'d, N>> {
	fn from(buf: Vec<Seg<'d, N>>) -> Self {
		assert!(
			buf.iter().is_partitioned(Seg::is_not_empty),
			"segment vector must be partitioned into non-empty and empty segments"
		);
		let len = buf.partition_point(Seg::is_not_empty);
		let count = buf[..len].iter().map(Seg::len).sum();

		Self {
			buf: buf.into(),
			len,
			count,
		}
	}
}

impl<T> RBuf<T> {
	/// Creates a new, empty ring buffer.
	pub const fn new() -> Self {
		Self {
			buf: VecDeque::new(),
			len: 0,
			count: 0,
		}
	}
}

impl<'a, const N: usize> RBuf<Seg<'a, N>> {
	/// Returns the number of readable segments in the buffer.
	pub fn len(&self) -> usize { self.len }

	/// Returns the number of segments in the buffer, counting empty segments.
	pub fn capacity(&self) -> usize { self.buf.len() }

	/// Returns the number of bytes in the buffer.
	pub fn count(&self) -> usize { self.count }

	/// Returns the number of bytes that can be written to the buffer.
	pub fn limit(&self) -> usize {
		self.buf
			.range(self.len.saturating_sub(1)..)
			.map(Seg::limit)
			.sum()
	}

	/// Returns `true` if the buffer is empty.
	pub fn is_empty(&self) -> bool { self.len == 0 }

	/// Returns `true` if the buffer contains empty segments.
	pub fn has_empty(&self) -> bool { self.len < self.capacity() }

	/// Returns a reference to the back segment.
	pub fn back(&self) -> Option<&Seg<'a, N>> {
		Some(&self.buf[self.back_index()?])
	}

	/// Pushes `seg` to the front of the buffer.
	pub fn push_front(&mut self, seg: Seg<'a, N>) {
		if self.is_empty() {
			self.push_empty(seg);
			return;
		}
		self.len += 1;
		self.count += seg.len();
		self.buf.push_front(seg);
	}

	/// Pushes `seg` to the back of the buffer.
	pub fn push_back(&mut self, seg: Seg<'a, N>) {
		if seg.is_empty() {
			self.push_empty(seg);
			return
		}

		self.len += 1;
		self.count += seg.len();
		self.buf.push_back(seg);
	}

	/// Pops a readable segment from the front of the buffer.
	pub fn pop_front(&mut self) -> Option<Seg<'a, N>> {
		if !self.is_empty() {
			let seg = self.buf.pop_front()?;
			self.count -= seg.len();
			self.len -= 1;
			Some(seg)
		} else {
			None
		}
	}

	/// Pops a writable segment from the back of the buffer.
	pub fn pop_back(&mut self) -> Option<Seg<'a, N>> {
		let is_full_or_shared = || {
			let back = self.back().unwrap();
			back.is_full() || back.is_shared()
		};
		if self.is_empty() || is_full_or_shared() {
			return self.pop_empty()
		}

		let index = self.back_index().unwrap();
		let seg = if self.has_empty() {
			self.buf.swap_remove_back(index)?
		} else {
			self.buf.remove(index)?
		};

		self.len -= 1;
		self.count -= seg.len();
		Some(seg)
	}

	/// Allocates `count` segments to the back of the buffer.
	pub fn allocate(&mut self, count: usize) {
		self.buf.reserve(count);
		for _ in 0..count {
			self.buf.push_back(Seg::default());
		}
	}

	/// Consumes `count` bytes from the internal count.
	pub fn consume(&mut self, count: usize) {
		self.count -= count;
	}

	/// Drains up to `count` segments from the buffer.
	pub fn drain(&mut self, count: usize) -> impl Iterator<Item = Seg<'a, N>> + '_ {
		// Drain all segments
		if count >= self.capacity() {
			self.len = 0;
			self.count = 0;
		} else {
			let count =
				self.buf
					.iter()
					.take(count)
					.map(Seg::limit)
					.sum::<usize>();
			self.count -= count;

			if count <= self.len {
				self.len -= count;
			} else {
				self.len = 0;
			}
		}

		self.buf.drain(..min(count, self.capacity()))
	}

	/// Drains up to `count` empty segments from the buffer.
	pub fn drain_empty(&mut self, count: usize) -> impl Iterator<Item = Seg<'a, N>> + '_ {
		let mut range = self.len..self.capacity();
		let len = range.len();
		range.start += len - min(len, count);
		self.buf.drain(range)
	}
	
	/// Drains all empty segments from the buffer.
	pub fn drain_all_empty(&mut self) -> impl Iterator<Item = Seg<'a, N>> + '_ {
		self.drain_empty(self.capacity() - self.len)
	}

	/// Returns a pair of slices which contain the buffer segments, in order, with
	/// written segments at the front and empty segments at the back. Using these
	/// may invalidate the buffer, and must be followed by [`invalidate`].
	///
	/// [`invalidate`]: Self::invalidate
	#[allow(dead_code)] // May be used later
	pub fn as_mut_slices(&mut self) -> (&mut [Seg<'a, N>], &mut [Seg<'a, N>]) {
		self.buf.as_mut_slices()
	}

	/// Splits slice segments into sub-segments of length `N` or shorter.
	pub fn split_slice_segments(&mut self) {
		let mut i = 0;
		while i < self.len {
			if let Some((chunks, remainder)) = self.buf[i].split_off_slice() {
				let mut added = chunks.len();
				if !remainder.is_empty() {
					added += 1;
				}
				let chunks = chunks.iter().map(|chunk| Seg::from_slice(chunk));
				let remainder = Seg::from_slice(remainder);

				self.buf.reserve(added);
				if i == self.buf.len() - 1 {
					// We're at the end of the buffer, so use extend.
					self.buf.extend(chunks);
					if remainder.is_not_empty() {
						self.buf.push_back(remainder);
					}
				} else {
					// No easy way to insert an iterator, so: rotate, extend, rotate
					// back.
					self.buf.rotate_left(i);
					self.buf.extend(chunks);
					if remainder.is_not_empty() {
						self.buf.push_back(remainder);
					}
					self.buf.rotate_right(i + added);
					self.len += added;
				}
			}

			i += 1;
		}
	}
}

impl<T> RBuf<T> {
	/// Iterates over written segments.
	pub fn iter(&self) -> impl Iterator<Item = &T> + '_ {
		self.buf.iter().take(self.len)
	}

	/// Iterates mutably over written segments.
	pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> + '_ {
		self.buf.range_mut(..self.len)
	}

	/// Rotates empty segments to the back.
	pub fn rotate_back(&mut self, count: usize) {
		self.buf.rotate_left(count);
		self.len -= count;
	}
}

impl<'a, const N: usize> RBuf<Seg<'a, N>> {
	fn back_index(&self) -> Option<usize> {
		(!self.is_empty()).then(|| self.len - 1)
	}

	fn push_empty(&mut self, seg: Seg<'a, N>) {
		self.buf.push_back(seg);
	}

	fn pop_empty(&mut self) -> Option<Seg<'a, N>> {
		if self.has_empty() {
			let empty = self.buf.pop_back()?;
			Some(empty)
		} else {
			None
		}
	}
}

impl<'a, const N: usize> RBuf<Seg<'a, N>> {
	fn push_many<T: IntoIterator<Item = Seg<'a, N>>>(&mut self, iter: T) {
		let start = self.len;
		// Temporarily rotate empty segments to the front before extending, in case
		// iter contains non-empty segments.
		self.buf.rotate_right(start);
		self.buf.extend(iter);
		let end = self.capacity();

		// Partition non-empty segments ahead of empty segments.
		let mut non_empty_len = 0;
		for i in start..end {
			let seg = &self.buf[i];
			if seg.is_not_empty() && i > non_empty_len {
				self.buf.swap(i, start + non_empty_len);
				non_empty_len += 1;
			}
		}

		if non_empty_len > 0 {
			self.len += non_empty_len;
		}

		// Rotate the empty segments back.
		self.rotate_back(start);
	}
}

impl<'a, const N: usize> Extend<Seg<'a, N>> for RBuf<Seg<'a, N>> {
	fn extend<T: IntoIterator<Item = Seg<'a, N>>>(&mut self, iter: T) {
		self.push_many(iter);
	}

	fn extend_one(&mut self, seg: Seg<'a, N>) {
		self.push_back(seg);
	}

	fn extend_reserve(&mut self, additional: usize) {
		self.buf.reserve(additional);
	}
}

impl<T> IntoIterator for RBuf<T> {
	type Item = T;
	type IntoIter = <VecDeque<T> as IntoIterator>::IntoIter;

	fn into_iter(self) -> Self::IntoIter {
		self.buf.into_iter()
	}
}

impl<T> Index<usize> for RBuf<T> {
	type Output = T;

	fn index(&self, index: usize) -> &Self::Output {
		&self.buf[index]
	}
}

impl<T> IndexMut<usize> for RBuf<T> {
	/// Gets a mutable reference to a segment at `index`. Using this reference may
	/// invalidate the buffer, and must be followed by [`invalidate`].
	///
	/// [`invalidate`]: Self::invalidate
	fn index_mut(&mut self, index: usize) -> &mut Self::Output {
		&mut self.buf[index]
	}
}

impl<T: PartialEq> PartialEq for RBuf<T> {
	fn eq(&self, other: &Self) -> bool {
		self.iter().eq(other.iter())
	}
}
