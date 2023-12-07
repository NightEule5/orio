// SPDX-License-Identifier: Apache-2.0

use std::cmp::min;
use std::collections::VecDeque;
use std::mem;
use std::ops::{Index, IndexMut};
use crate::pool::{MutPool, Pool};
use super::Seg;

// Todo: since shared segments may become writable later (when an Rc is dropped),
//  currently the counted limit is a lower bound. Maybe add a separate "unwritable"
//  limit count?

/// A shareable, segmented ring buffer. Cloning shares segments in linear (`O(n)`)
/// time.
#[derive(Clone, Debug, Default)]
pub(crate) struct RBuf<T> {
	buf: VecDeque<T>,
	/// The number of readable segments in the buffer.
	len: usize,
	/// The total size of space occupied by non-empty segments, including unusable
	/// gaps between partial segments. For simplicity, this also counts the back
	/// segment's limit.
	size: usize,
	/// The number of readable bytes in the buffer.
	count: usize,
	/// The number of writable bytes in the buffer.
	limit: usize,
}

impl<'a, const N: usize> RBuf<Seg<'a, N>> {
	/// Returns the number of readable segments in the buffer.
	pub fn len(&self) -> usize { self.len }

	/// Returns the number of segments in the buffer, counting empty segments.
	pub fn capacity(&self) -> usize { self.buf.len() }

	/// Returns the number of bytes in the buffer.
	pub fn count(&self) -> usize { self.count }

	/// Returns the number of bytes that can be written to the buffer.
	pub fn limit(&self) -> usize { self.limit }

	/// Returns the fragmentation length.
	pub fn fragment_len(&self) -> usize {
		self.size - self.count - self.back_limit()
	}

	/// Returns `true` if the buffer is empty.
	pub fn is_empty(&self) -> bool { self.len == 0 }

	/// Returns `true` if the buffer contains empty segments.
	pub fn has_empty(&self) -> bool { self.len < self.capacity() }

	/// Pushes `seg` to the front of the buffer.
	pub fn push_front(&mut self, seg: Seg<'a, N>) {
		if self.is_empty() {
			self.push_empty(seg);
			return;
		}
		self.size += seg.size();
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

		self.size += seg.size();
		self.len += 1;
		if !self.is_empty() {
			self.limit -= self.back_limit();
		}
		self.count += seg.len();
		self.limit += seg.limit();
		self.buf.push_back(seg);
	}

	/// Pops a readable segment from the front of the buffer.
	pub fn pop_front(&mut self) -> Option<Seg<'_, N>> {
		if !self.is_empty() {
			let seg = self.buf.pop_front()?;
			self.count -= seg.len();
			if self.len == 1 {
				self.limit -= self.back_limit();
			}
			self.len -= 1;
			self.size -= seg.size();
			Some(seg)
		} else {
			None
		}
	}

	/// Pops a writable segment from the back of the buffer.
	pub fn pop_back(&mut self) -> Option<Seg<'_, N>> {
		let index = self.back_index();
		if self.is_empty() || self.buf[index].is_full() || self.buf[index].is_shared() {
			return self.pop_empty()
		}

		let seg = if self.has_empty() {
			self.buf.swap_remove_back(index)?
		} else {
			self.buf.remove(index)?
		};

		self.len -= 1;
		self.size -= seg.size();
		self.count -= seg.len();
		self.limit -= seg.limit();
		Some(seg)
	}

	/// Rearranges segments into one contiguous slice, returning that slice. Using
	/// this slice invalidates the buffer, and must be followed by [`invalidate`].
	///
	/// [`invalidate`]: Self::invalidate
	pub fn make_contiguous(&mut self) -> &mut [Seg<'a, N>] {
		self.buf.make_contiguous()
	}

	/// Invalidates and recalculates the counts.
	pub fn invalidate(&mut self) {
		let mut last_limit = None;

		for seg in self.buf {
			self.size += seg.size();

			if seg.is_empty() {
				self.limit += seg.limit();
			} else {
				self.len += 1;
				self.count += seg.len();
				last_limit = Some(seg.limit());
			}
		}

		if let Some(ll) = last_limit {
			self.limit += ll;
		}
	}

	/// Returns a cursor for reading segments.
	pub fn read(&mut self, pool: &mut impl Pool<N>) -> Option<ReadCursor<'a, '_, N, impl Pool<N>>> {
		(!self.is_empty()).then(|| ReadCursor::new(self, pool))
	}

	/// Drains up to `count` segments from the buffer.
	pub fn drain(&mut self, count: usize) -> impl Iterator<Item = Seg<'_, N>> + '_ {
		// Drain all segments
		if count >= self.capacity() {
			self.len = 0;
			self.size = 0;
			self.count = 0;
			self.limit = 0;
		} else {
			let (size, count) =
				self.buf
					.iter()
					.take(count)
					.map(|seg| (seg.size(), seg.limit()))
					.reduce(|(mut s_sum, mut l_sum), (s_cur, l_cur)| {
						s_sum += s_cur;
						l_sum += l_cur;
						(s_sum, l_sum)
					})
					.unwrap_or_default();
			self.size -= size;
			self.count -= count;

			if count >= self.back_index() {
				self.limit -= self.back_limit();
			}

			self.limit -=
				self.buf
					.iter()
					.take(count.saturating_sub(self.len))
					.map(Seg::limit)
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
		self.limit -= self.buf.range(range.clone()).map(Seg::limit).sum::<usize>();
		self.buf.drain(range)
	}
	
	/// Drains all empty segments from the buffer.
	pub fn drain_all_empty(&mut self) -> impl Iterator<Item = Seg<'a, N>> + '_ {
		self.drain_empty(self.capacity() - self.len)
	}

	/// Iterates over written segments.
	pub fn iter(&self) -> impl Iterator<Item = &Seg<'_, N>> + '_ {
		self.buf.iter().take(self.len)
	}

	/// Iterates mutably over written segments.
	pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Seg<'a, N>> + '_ {
		self.buf.iter_mut().take(self.len)
	}

	/// Returns a pair of slices which contain the buffer segments, in order, with
	/// written segments at the front and empty segments at the back. Using these
	/// invalidates the buffer, and must be followed by [`invalidate`].
	///
	/// [`invalidate`]: Self::invalidate
	pub fn as_mut_slices(&mut self) -> (&mut [Seg<'a, N>], &mut [Seg<'a, N>]) {
		self.buf.as_mut_slices()
	}
}

impl<'a, const N: usize> RBuf<Seg<'a, N>> {
	fn back_index(&self) -> usize { self.len - 1 }

	pub(crate) fn back_limit(&self) -> usize {
		if self.is_empty() {
			0
		} else {
			self.buf[self.back_index()].limit()
		}
	}

	fn push_empty(&mut self, seg: Seg<'a, N>) {
		self.limit += seg.limit();
		self.buf.push_back(seg);
	}

	fn pop_empty(&mut self) -> Option<Seg<'a, N>> {
		if self.has_empty() {
			let empty = self.buf.pop_back()?;
			self.limit -= empty.limit();
			Some(empty)
		} else {
			None
		}
	}

	/// Rotates empty segments to the back.
	fn rotate_back(&mut self, count: usize) {
		self.buf.rotate_left(count);
		self.len -= count;
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
			self.limit -= self.back_limit();
			self.len += non_empty_len;
			self.limit += self.back_limit();
		}

		self.size +=
			self.buf
				.range(start..end)
				.map(Seg::size)
				.sum::<usize>();
		self.limit +=
			self.buf
				.range(start + non_empty_len..end)
				.map(Seg::limit)
				.sum::<usize>();

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
	fn index_mut(&mut self, index: usize) -> &mut Self::Output {
		&mut self.buf[index]
	}
}

#[derive(Debug)]
pub(crate) struct ReadCursor<'a: 'b, 'b, const N: usize, P: Pool<N>> {
	buf: &'b mut RBuf<Seg<'a, N>>,
	pool: &'b mut P,
	off: usize,
	len: usize,
	dirty: bool,
	empty_len: usize,
	cur_size:  usize,
	cur_count: usize,
	cur_limit: usize,
}

impl<'a: 'b, 'b, const N: usize, P: Pool<N>> ReadCursor<'a, 'b, N, P> {
	pub fn get(&self) -> &Seg<'a, N> {
		&self.buf.buf[self.off]
	}

	pub fn get_mut(&mut self) -> &mut Seg<'a, N> {
		&mut self.buf.buf[self.off]
	}

	pub fn has_next(&mut self) -> bool { !self.is_back() }
	pub fn has_prev(&mut self) -> bool { self.off > 0 }

	pub fn next(&mut self) -> Option<&mut Seg<'a, N>> {
		self.update();
		if !self.has_next() { return None }
		self.off += 1;
		self.reset();
		Some(self.get_mut())
	}

	pub fn prev(&mut self) -> Option<&mut Seg<'a, N>> {
		self.update();
		if !self.has_prev() { return None }
		self.off -= 1;
		self.reset();
		Some(self.get_mut())
	}

	pub fn front(&mut self) -> Option<&mut Seg<'a, N>> {
		self.update();
		if self.len == 0 { return None }
		self.off = 0;
		self.reset();
		Some(self.get_mut())
	}

	pub fn swap(&mut self, seg: &mut Seg<'a, N>) -> bool {
		if self.len > 0 {
			mem::swap(self.get_mut(), seg);
			true
		} else {
			false
		}
	}
}

impl<'a: 'b, 'b, const N: usize, P: Pool<N>> ReadCursor<'a, 'b, N, P> {
	fn new(buf: &'b mut RBuf<Seg<'a, N>>, pool: &'b mut P) -> Self {
		let len = buf.len;
		let front = &buf.buf[0];
		let cur_size = front.size();
		let cur_count = front.len();
		let cur_limit = front.limit();

		Self {
			buf,
			pool,
			off: 0,
			len,
			dirty: true,
			empty_len: 0,
			cur_size,
			cur_count,
			cur_limit
		}
	}

	fn is_back(&self) -> bool {
		self.off + 1 == self.len
	}

	fn reset(&mut self) {
		let cur = self.get();
		let cur_size = cur.size();
		let cur_count = cur.len();
		let cur_limit = cur.limit();
		self.dirty = true;
		self.cur_size = cur_size;
		self.cur_count = cur_count;
		self.cur_limit = cur_limit;
	}

	fn update(&mut self) {
		if !self.dirty { return }

		let seg = self.get();
		let delta_size  = seg.size () as isize - self.cur_size  as isize;
		let delta_count = seg.len  () as isize - self.cur_count as isize;
		let delta_limit = seg.limit() as isize - self.cur_limit as isize;
		if seg.is_empty() {
			self.empty_len += 1;
		} else if self.cur_count == 0 {
			self.empty_len -= 1;
		}

		let is_back = self.is_back();
		let RBuf { size, count, limit, .. } = self.buf;
		*size  = size .saturating_add_signed(delta_size );
		*count = count.saturating_add_signed(delta_count);
		if is_back {
			*limit = limit.saturating_add_signed(delta_limit);
		}

		self.dirty = false;
	}

	fn trim_or_rotate_empty(&mut self) {
		if let Ok(mut pool) = self.pool.try_borrow() {
			for seg in self.buf.drain(self.empty_len) {
				pool.collect_one(seg);
			}
		} else {
			let empty_limit: usize =
				self.buf
					.buf
					.range(..self.empty_len)
					.map(Seg::limit)
					.sum();
			self.buf.limit += empty_limit;
			self.buf.rotate_back(self.empty_len);
			self.off = 0;
			self.len = self.buf.len();
			self.empty_len = 0;
		}
	}
}

impl<'a: 'b, 'b, const N: usize, P: Pool<N>> Drop for ReadCursor<'a, 'b, N, P> {
	fn drop(&mut self) {
		self.update();
		self.trim_or_rotate_empty();
	}
}
