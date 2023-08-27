// SPDX-License-Identifier: Apache-2.0

use std::cmp::min;
use std::collections::VecDeque;
use crate::new::{Element, MutPool, Seg};

/// A ring buffer of [`Seg`]ments.
#[derive(Clone, Debug, Default)]
pub(crate) struct SegRing<'a, const N: usize, T: Element> {
	ring: VecDeque<Seg<'a, N, T>>,
	/// The number of segments written.
	len: usize,
	/// The number of elements stored.
	count: usize,
	/// The number of elements that can be written before all segments are full.
	limit: usize,
	/// The length of fragmentation, the total free space locked between partially
	/// written and read segments.
	frag_len: usize,
}

impl<'a, const N: usize, T: Element> SegRing<'a, N, T> {
	/// Returns the number of segments written.
	pub fn len(&self) -> usize { self.len }
	/// Returns the number of elements that can be written before claiming.
	pub fn limit(&self) -> usize { self.limit }
	/// Returns the number of elements written.
	pub fn count(&self) -> usize { self.count }
	/// Returns the length of fragmentation, the total free space locked between
	/// partially written and read segments.
	pub fn frag_len(&self) -> usize { self.frag_len }
	/// Returns `true` if no data is written.
	pub fn is_empty(&self) -> bool { self.count == 0 }
	/// Returns `true` if data is written.
	pub fn is_not_empty(&self) -> bool { self.count > 0 }
	/// Returns `true` if the buffer contains empty segments.
	fn has_empty(&self) -> bool { self.len < self.ring.len() }
	/// Returns a count of empty segments in the buffer.
	fn empty_count(&self) -> usize { self.ring.len() - self.len }

	/// Pushes a partially read segment to the front of the buffer.
	pub fn push_front(&mut self, seg: Seg<'a, N, T>) {
		if seg.is_empty() {
			self.push_empty(seg);
			return
		}

		self.len += 1;
		self.count += seg.len();

		if self.is_empty() {
			self.limit += seg.limit();
		} else {
			self.frag_len += seg.limit();
		}

		self.ring.push_front(seg);
	}

	/// Pushes a written or empty segment to the back of the deque.
	pub fn push_back(&mut self, seg: Seg<'a, N, T>) {
		if seg.is_empty() {
			self.push_empty(seg);
		} else {
			self.push_laden(seg);
		}
	}

	/// Pops the front segment from the deque for reading.
	pub fn pop_front(&mut self) -> Option<Seg<'a, N, T>> {
		if self.is_empty() { return None }

		let seg = self.ring.pop_front()?;

		if self.is_empty() {
			self.limit -= seg.limit();
		} else {
			self.frag_len -= seg.limit();
		}

		self.len -= 1;
		self.count -= seg.len();
		Some(seg)
	}

	/// Pops the back-most unfilled segment from the deque for writing.
	pub fn pop_back(&mut self) -> Option<Seg<'a, N, T>> {
		let seg = if self.has_empty() {
			// Faster to replace the popped segment with a fresh one from the back
			// if possible.
			self.ring.swap_remove_back(self.len)?
		} else {
			self.ring.pop_back()?
		};

		self.len += 1;
		self.limit -= seg.limit();
		self.count -= seg.len();
		Some(seg)
	}

	/// Reserves at least `count` elements of free memory from `pool`.
	pub fn reserve(&mut self, mut count: usize, pool: &mut impl MutPool<N, T>) {
		count = count.saturating_sub(self.limit);
		if count > 0 {
			pool.claim_size(self, count);
		}
	}

	/// Clears the buffer, returning all its segments to `pool`.
	pub fn clear(&mut self, pool: &mut impl MutPool<N, T>) {
		pool.collect(self.ring.drain(..));
		self.len = 0;
		self.count = 0;
		self.limit = 0;
		self.frag_len = 0;
	}

	/// Returns up to `count` empty segments to `pool`.
	pub fn trim(&mut self, mut count: usize, pool: &mut impl MutPool<N, T>) {
		count = min(count, self.empty_count());
		self.limit -= count * N;
		let len = self.len;
		pool.collect(self.ring.drain(len..len + count));
	}

	/// Iterates over written segments.
	pub fn iter(&self) -> impl Iterator<Item = &Seg<'a, N, T>> {
		self.ring
			.iter()
			.take(self.len)
	}

	/// Iterates over written segments as slices.
	pub fn slices(&self) -> impl Iterator<Item = &[T]> {
		self.iter()
			.map(|s| s.as_slice())
	}
}

impl<'a, const N: usize, T: Element> SegRing<'a, N, T> {
	fn last(&self) -> Option<&Seg<'_, N, T>> {
		if self.is_empty() {
			None
		} else {
			Some(&self.ring[self.len - 1])
		}
	}

	fn push_empty(&mut self, seg: Seg<'a, N, T>) {
		self.limit += N;
		self.ring.push_back(seg);
	}

	fn push_laden(&mut self, seg: Seg<'a, N, T>) {
		// Convert the last segment's limit to fragmentation.
		let last_lim = self.last().map(Seg::limit).unwrap_or_default();
		self.limit    -= last_lim;
		self.frag_len += last_lim;
		// Update the quantities with the new segment and push.
		self.frag_len += seg.off;
		self.limit += seg.limit();
		self.count += seg.len();
		self.ring.insert(self.len, seg);
		self.len += 1;
	}
}

impl<'a, const N: usize, T: Element> Extend<Seg<'a, N, T>> for SegRing<'a, N, T> {
	fn extend<I: IntoIterator<Item = Seg<'a, N, T>>>(&mut self, iter: I) {
		let Self { ring, limit, .. } = self;
		let cur_len = ring.len();
		ring.extend(iter);
		let new_len = ring.len();
		*limit += (new_len - cur_len) * N;
	}

	fn extend_one(&mut self, item: Seg<'a, N, T>) {
		self.push_empty(item);
	}

	fn extend_reserve(&mut self, additional: usize) {
		self.ring.reserve(additional);
	}
}
