// SPDX-License-Identifier: Apache-2.0

use std::cmp::min;
use std::collections::VecDeque;
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

impl<const N: usize> RBuf<Seg<'_, N>> {
	/// The number of readable segments in the buffer.
	pub fn len(&self) -> usize { self.len }

	/// The number of segments in the buffer, counting empty segments.
	pub fn capacity(&self) -> usize { self.buf.len() }

	/// Returns the fragmentation length.
	pub fn fragment_len(&self) -> usize {
		self.size - self.count - self.back_limit()
	}

	/// Returns `true` if the buffer is empty.
	pub fn is_empty(&self) -> bool { self.len == 0 }

	/// Returns `true` if the buffer contains empty segments.
	pub fn has_empty(&self) -> bool { self.len < self.capacity() }

	/// Pushes `seg` to the front of the buffer.
	pub fn push_front(&mut self, seg: Seg<'_, N>) {
		self.size += seg.size();
		if self.is_empty() {
			self.limit += seg.limit();
		}
		self.len += 1;
		self.count += seg.len();
		self.buf.push_front(seg);
	}

	/// Pushes `seg` to the back of the buffer.
	pub fn push_back(&mut self, seg: Seg<'_, N>) {
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
		if self.is_empty() || self.buf[index].is_full() {
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

	/// Drains up to `count` segments from the buffer.
	pub fn drain(&mut self, count: usize) -> impl Iterator<Item = Seg<'_, N>> + '_ {
		// Drain all segments
		if count >= self.capacity() {
			self.len   = 0;
			self.size  = 0;
			self.count = 0;
			self.limit = 0;
		} else {
			(self.size, self.count) -=
				self.buf
					.iter()
					.take(min(count, self.len))
					.map(|seg| (seg.size(), seg.limit()))
					.sum();

			if count >= self.back_index() {
				self.limit -= self.back_limit();
			}

			self.limit -=
				self.buf
					.iter()
					.take(count.saturating_sub(self.len))
					.map(Seg::limit)
					.sum();

			if count <= self.len {
				self.len -= count;
			} else {
				self.len = 0;
			}
		}

		self.buf.drain(..min(count, self.capacity()))
	}

	/// Iterates over written segments.
	pub fn iter(&self) -> impl Iterator<Item = &Seg<'_, N>> + '_ {
		self.buf.iter().take(self.len)
	}

	/// Iterates mutably over written segments.
	pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Seg<'_, N>> + '_ {
		self.buf.iter_mut().take(self.len)
	}
}

impl<const N: usize> RBuf<Seg<'_, N>> {
	fn back_index(&self) -> usize { self.len - 1 }

	fn back_limit(&self) -> usize {
		if self.is_empty() {
			0
		} else {
			self.buf[self.back_index()].limit()
		}
	}

	fn push_empty(&mut self, seg: Seg<'_, N>) {
		self.limit += seg.limit();
		self.buf.push_back(seg);
	}

	fn pop_empty(&mut self) -> Option<Seg<'_, N>> {
		if self.has_empty() {
			let empty = self.buf.pop_back()?;
			self.limit -= empty.limit();
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
		self.buf.rotate_left(start);
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
				.sum();
		self.limit +=
			self.buf
				.range(start + non_empty_len..end)
				.map(Seg::limit)
				.sum();

		// Rotate the empty segments back.
		self.buf.rotate_right(start);
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
