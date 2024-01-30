// SPDX-License-Identifier: Apache-2.0

use std::cmp::{min, Ordering};
use std::collections::{vec_deque, VecDeque};
use std::iter::Skip;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut, Index, IndexMut, RangeBounds};
use std::ptr::NonNull;
use std::slice;
use all_asserts::debug_assert_le;
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

pub struct SlotMut<'a, 'b, const N: usize> {
	ring: NonNull<RBuf<Seg<'a, N>>>,
	seg: &'b mut Seg<'a, N>,
	start_len: usize,
	_lifetime_b: PhantomData<&'b Seg<'a, N>>
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

impl<'a: 'b, 'b, const N: usize> Deref for SlotMut<'a, 'b, N> {
	type Target = Seg<'a, N>;

	fn deref(&self) -> &Self::Target {
		self.seg
	}
}

impl<'a: 'b, 'b, const N: usize> DerefMut for SlotMut<'a, 'b, N> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		self.seg
	}
}

impl<'a: 'b, 'b, const N: usize> Drop for SlotMut<'a, 'b, N> {
	fn drop(&mut self) {
		let Self { ring, seg: back, start_len, .. } = self;
		let is_empty = *start_len == 0;
		match back.len().cmp(start_len) {
			Ordering::Less => {
				// Segment was emptied
				if !is_empty && back.is_empty() {
					unsafe {
						ring.as_mut().dec_len(1);
					}
				}

				let consumed = *start_len - back.len();
				unsafe {
					ring.as_mut().dec_count(consumed);
				}
			}
			Ordering::Greater => {
				// Segment was filled
				if is_empty {
					unsafe {
						ring.as_mut().inc_len(1);
					}
				}

				let added = back.len() - *start_len;
				unsafe {
					ring.as_mut().inc_count(added);
				}
			}
			_ => { }
		}
	}
}

impl<'a, const N: usize> RBuf<Seg<'a, N>> {
	/// Returns the number of readable segments in the buffer.
	pub fn len(&self) -> usize { self.len }

	/// Returns the number of segments in the buffer, counting empty segments.
	pub fn capacity(&self) -> usize { self.buf.len() }

	/// Returns the total number of bytes that can be written to the buffer.
	pub fn byte_capacity(&self) -> usize {
		self.buf
			.iter()
			.map(Seg::size)
			.sum()
	}

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

	/// Returns a drop-guarded mutable reference to the front segment.
	pub fn front_mut(&mut self) -> Option<SlotMut<'a, '_, N>> {
		let ring = self.into();
		self.buf
			.front_mut()
			.filter(|seg| seg.is_not_empty())
			.map(|seg| {
				let start_len = seg.len();
				SlotMut {
					ring,
					seg,
					start_len,
					_lifetime_b: PhantomData
				}
			})
	}

	/// Returns a drop-guarded mutable reference to the back segment.
	pub fn back_mut(&mut self) -> Option<SlotMut<'a, '_, N>> {
		self.back_index().map(|i| {
			let ring = self.into();
			let back = &mut self.buf[i];
			let start_len = back.len();
			SlotMut {
				ring,
				seg: back,
				start_len,
				_lifetime_b: PhantomData
			}
		})
	}

	/// Pushes `seg` to the front of the buffer.
	#[allow(unused)]
	pub fn push_front(&mut self, seg: Seg<'a, N>) {
		if seg.is_empty() {
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
			self.count -=
				self.buf
					.iter()
					.take(count)
					.map(Seg::len)
					.sum::<usize>();

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
	
	/// Iterates over shared segments in `range`.
	pub fn share_range<R: RangeBounds<usize>>(&self, range: R) -> RangeIter<'a, '_, N> {
		let range = slice::range(range, ..self.count);
		RangeIter {
			iter: self.buf.range(..self.len),
			start: range.start,
			count: range.len(),
		}
	}

	/// Returns an iterator over segment slices in `range`.
	pub fn iter_slices_in_range<R: RangeBounds<usize>>(&self, range: R) -> SliceRangeIter<'a, '_, N> {
		let range = slice::range(range, ..self.count);
		let (skip_len, first_offset) = self.segment_index(range.start);
		let count = range.len();
		SliceRangeIter {
			iter: self.iter().skip(skip_len),
			first_offset,
			index: 0,
			count,
			cur_count: 0,
			current: None,
		}
	}

	pub fn iter_slices(&self) -> SliceIter<'a, '_, N> {
		SliceIter {
			iter: self.iter(),
			current: None,
		}
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

	/// Counts segments with at least `count` bytes of total writable capacity,
	/// returning the segment count and the remaining bytes written to the last
	/// segment.
	pub fn count_writable(&self, mut count: usize) -> (usize, usize) {
		let mut remaining_count = 0;
		(
			self.buf
				.range(self.len..)
				.position(|seg|
					count > 0 && {
						if seg.limit() <= count {
							count -= self.limit();
						} else {
							remaining_count = count;
							count = 0;
						}
						true
					}
				).unwrap_or(0),
			remaining_count
		)
	}

	pub fn writable_index(&self) -> usize {
		self.back_index()
			.filter(|&i| !self.buf[i].is_full())
			.unwrap_or(self.len)
	}

	pub fn is_back_partial_writable(&self) -> bool {
		self.back().is_some_and(|seg| !seg.is_full() && !seg.is_empty())
	}

	pub fn iter_all_writable(&mut self) -> vec_deque::IterMut<Seg<'a, N>> {
		let start = self.writable_index();
		self.buf.range_mut(start..)
	}

	pub fn iter_writable(&mut self, count: usize) -> vec_deque::IterMut<Seg<'a, N>> {
		let start = self.writable_index();
		self.buf.range_mut(start..start + count)
	}

	/// Grows the buffer and its segments by `count` bytes.
	pub unsafe fn grow(&mut self, count: usize) {
		let mut seg_count = 0;
		let is_back_writable = self.is_back_partial_writable();
		let mut counted = 0;
		for seg in self.buf.range_mut(self.len.saturating_sub(1)..) {
			if counted == count {
				break
			}

			let grow_len = seg.limit().min(count);
			seg.set_len(seg.len() + grow_len);
			counted += grow_len;
			seg_count += 1;
		}

		if is_back_writable && seg_count > 0 {
			seg_count -= 1;
		}

		self.inc_len(seg_count);
		self.inc_count(count);
	}
	
	/// Sets the tracked length.
	pub unsafe fn set_len(&mut self, len: usize) {
		debug_assert_eq!(
			len,
			self.buf
				.iter()
				.rposition(Seg::is_not_empty)
				.map(|i| i + 1)
				.unwrap_or_default()
		);
		self.len = len;
	}

	/// Increments the tracked length after writing.
	unsafe fn inc_len(&mut self, len: usize) {
		self.set_len(self.len + len);
	}

	/// Decrements the tracked length after reading.
	pub unsafe fn dec_len(&mut self, len: usize) {
		use all_asserts::assert_le;
		debug_assert_le!(len, self.len);
		self.set_len(self.len - len);
	}
	
	/// Sets the tracked count.
	pub unsafe fn set_count(&mut self, count: usize) {
		self.count = count;
	}

	/// Increments the tracked count after writing.
	unsafe fn inc_count(&mut self, count: usize) {
		self.set_count(self.count + count);
	}

	/// Decrements the tracked count after reading.
	pub unsafe fn dec_count(&mut self, count: usize) {
		use all_asserts::assert_le;
		debug_assert_le!(count, self.count);
		self.set_count(self.count - count);
	}
}

impl<T> RBuf<T> {
	/// Iterates over written segments.
	pub fn iter(&self) -> vec_deque::Iter<T> {
		self.buf.range(..self.len)
	}

	/// Iterates mutably over written segments.
	pub fn iter_mut(&mut self) -> vec_deque::IterMut<T> {
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
		(!self.is_empty())
			.then(|| self.len - 1)
			.filter(|&i| !self.buf[i].is_full())
			.or_else(|| self.has_empty().then_some(self.len))
	}

	fn push_empty(&mut self, seg: Seg<'a, N>) {
		if seg.is_exclusive() {
			self.buf.push_back(seg);
		}

		// Let shared segments drop
	}

	fn pop_empty(&mut self) -> Option<Seg<'a, N>> {
		if self.has_empty() {
			let empty = self.buf.pop_back()?;
			Some(empty)
		} else {
			None
		}
	}

	fn push_many<T: IntoIterator<Item = Seg<'a, N>>>(&mut self, iter: T) {
		let start = self.len;
		let old_capacity = self.capacity();
		// Temporarily rotate empty segments to the front before extending, in case
		// iter contains written segments. The ensures new written segments stay in
		// chronological order front-to-back tailed by empty segments.
		self.buf.rotate_right(start);
		self.buf.extend(iter);

		// Count segments starting from the old capacity until the last written
		// segment, which is the number of written segments added.
		let new_len = self.buf
						  .range(old_capacity..)
						  .rposition(Seg::is_not_empty);
		// Push the new length if any were written.
		if let Some(new_len) = new_len {
			self.len += new_len;
		}

		// Rotate the empty segments back.
		self.rotate_back(start);
	}

	fn segment_index(&self, byte_index: usize) -> (usize, usize) {
		let mut offset = 0;
		for (i, seg) in self.iter().enumerate() {
			let remaining = byte_index - offset;
			if seg.len() > remaining {
				return (i, remaining)
			}

			offset += seg.len();
		}
		(self.len, 0)
	}
}

impl<'a, const N: usize> Extend<Seg<'a, N>> for RBuf<Seg<'a, N>> {
	/// Extends the ring buffer with elements from `iter`, inserting written segments
	/// at [`len`] and pushing empty segments to the back. Any empty segments between
	/// written segments are not moved.
	///
	/// This operation is implemented with rotation, so it should be used sparingly.
	///
	/// [`len`]: Self::len
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

pub struct RangeIter<'a: 'b, 'b, const N: usize> {
	iter: vec_deque::Iter<'b, Seg<'a, N>>,
	start: usize,
	count: usize,
}

pub struct SliceRangeIter<'a: 'b, 'b, const N: usize> {
	iter: Skip<vec_deque::Iter<'b, Seg<'a, N>>>,
	first_offset: usize,
	index: usize,
	count: usize,
	cur_count: usize,
	current: Option<(&'b [u8], &'b [u8])>
}

impl<'a: 'b, 'b, const N: usize> Iterator for RangeIter<'a, 'b, N> {
	type Item = Seg<'a, N>;

	fn next(&mut self) -> Option<Self::Item> {
		if self.count == 0 {
			return None
		}
		
		let mut cur = self.iter.find_map(|seg| {
			if seg.len() > self.start {
				let shared = seg.share(self.start..);
				self.start = 0;
				Some(shared)
			} else {
				self.start -= seg.len();
				None
			}
		})?;
		self.count -= cur.truncate(self.count);
		Some(cur)
	}
}

impl<'a: 'b, 'b, const N: usize> Iterator for SliceRangeIter<'a, 'b, N> {
	type Item = &'b [u8];

	fn next(&mut self) -> Option<Self::Item> {
		if let Some((_, b)) = self.current.take() {
			if !b.is_empty() {
				return Some(b)
			}
		}

		let remaining = self.count - self.cur_count;
		if remaining == 0 {
			return None
		}

		let offset = if self.index == 0 { self.first_offset } else { 0 };
		let seg = self.iter.next()?;
		let range = offset..remaining.min(seg.len()) + offset;
		self.cur_count += range.len();
		self.index += 1;
		let (a, b) = seg.as_slices_in_range(range);
		self.current = Some((a, b));
		Some(a)
	}
}

impl<'a: 'b, 'b, const N: usize> DoubleEndedIterator for SliceRangeIter<'a, 'b, N> {
	fn next_back(&mut self) -> Option<Self::Item> {
		if let Some((a, b)) = self.current.take() {
			self.cur_count -= a.len() + b.len();
			return Some(a)
		}

		if self.cur_count == 0 {
			return None
		}

		let offset = if self.index == 0 { self.first_offset } else { 0 };
		let seg = self.iter.next_back()?;
		let range = offset..self.cur_count.min(seg.len()) + offset;
		self.index = self.index.saturating_sub(1);
		let (a, b) = seg.as_slices_in_range(range);
		if b.is_empty() {
			self.cur_count -= a.len();
			Some(a)
		} else {
			self.current = Some((a, b));
			Some(b)
		}
	}
}

pub struct SliceIter<'a: 'b, 'b, const N: usize> {
	iter: vec_deque::Iter<'b, Seg<'a, N>>,
	current: Option<(&'b [u8], &'b [u8])>
}

impl<'a: 'b, 'b, const N: usize> Iterator for SliceIter<'a, 'b, N> {
	type Item = &'b [u8];

	fn next(&mut self) -> Option<Self::Item> {
		if let Some((_, b)) = self.current.take() {
			if !b.is_empty() {
				return Some(b)
			}
		}

		let (a, b) = self.iter.next()?.as_slices();
		self.current = Some((a, b));
		Some(a)
	}
}

impl<'a: 'b, 'b, const N: usize> DoubleEndedIterator for SliceIter<'a, 'b, N> {
	fn next_back(&mut self) -> Option<Self::Item> {
		if let Some((a, _)) = self.current.take() {
			return Some(a)
		}

		let (a, b) = self.iter.next_back()?.as_slices();
		if b.is_empty() {
			Some(a)
		} else {
			self.current = Some((a, b));
			Some(b)
		}
	}
}
