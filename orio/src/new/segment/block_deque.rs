// SPDX-License-Identifier: Apache-2.0

//! A lightweight, fixed-size deque based on [`VecDeque`].
//!
//! [`VecDeque`]: std::collections::VecDeque

use std::{fmt, mem, slice};
use std::iter::FusedIterator;
use std::ops::{IndexMut, Range, RangeBounds};
use std::rc::Rc;
use super::util::SliceExt;

/// A lightweight, fixed-size deque based on [`VecDeque`].
///
/// Can be cloned in constant time (`O(1)`), sharing the same buffer across two or
/// more clones. Shared data can be read, but exclusivity is required to push new
/// data or modify existing data.
///
/// [`VecDeque`]: std::collections::VecDeque
#[derive(Clone)]
pub struct BlockDeque<const N: usize> {
	buf: Rc<Box<[u8; N]>>,
	head: usize,
	len: usize
}

pub struct Iter<'a, T: 'a> {
	a: slice::Iter<'a, T>,
	b: slice::Iter<'a, T>,
}

pub fn buf<const N: usize>() -> Box<[u8; N]> {
	// ~3x faster than new_zeroed and Box::new([0; N]) on my machine, when N is
	// 8192, which makes sense, since we're not wasting cycles zeroing memory
	// we're going to write to soon anyway. A speedup from 90ns to 30ns may not
	// matter, that's fast enough and the whole point of this library is to
	// minimize allocations, but why not?
	// Safety: uninitialized values are never read, because length increments
	//  only after pushing a value to that index, which is initialization.
	unsafe { Box::<[_; N]>::new_uninit().assume_init() }
}

impl<const N: usize> BlockDeque<N> {
	/// Creates a new deque.
	pub fn new() -> Self { buf().into() }
	/// Creates a new deque with bytes filling by `populate`.
	pub fn populated(populate: impl FnOnce(&mut [u8; N]) -> usize) -> Self {
		let mut b = buf();
		let len = populate(&mut b);
		Self {
			buf: Rc::new(b),
			head: 0,
			len
		}
	}

	/// Returns the number of elements in the deque.
	pub fn len(&self) -> usize { self.len }
	/// Returns the number of elements that can be pushed before the deque is full.
	pub fn limit(&self) -> usize { N - self.len }
	/// Returns `true` if the deque is empty.
	pub fn is_empty(&self) -> bool { self.len == 0 }
	/// Returns `true` if the deque is full.
	pub fn is_full(&self) -> bool { self.len == N }
	/// Returns `true` if the deque contains shared data.
	pub fn is_shared(&self) -> bool { Rc::strong_count(&self.buf) != 1 }

	/// Returns a reference to the element at `index`.
	pub fn get(&self, index: usize) -> Option<&u8> {
		(index < self.len).then(||
			&self.buf[self.wrap(index)]
		)
	}

	/// Returns a mutable reference to the element at `index`.
	pub fn get_mut(&mut self, index: usize) -> Option<&mut u8> {
		(index < self.len).then(||
			self.index_mut(self.wrap(index))
		).flatten()
	}

	/// Returns a reference to the front element, or `None` if the deque is empty.
	pub fn front(&self) -> Option<&u8> {
		self.get(0)
	}

	/// Returns a mutable reference to the front element, or `None` if the deque is
	/// empty.
	pub fn front_mut(&mut self) -> Option<&mut u8> {
		self.get_mut(0)
	}

	/// Returns a reference to the back element, or `None` if the deque is empty.
	pub fn back(&self) -> Option<&u8> {
		self.get(self.len.wrapping_sub(1))
	}

	/// Returns a mutable reference to the back element, or `None` if the deque is
	/// empty.
	pub fn back_mut(&mut self) -> Option<&mut u8> {
		self.get_mut(self.len.wrapping_sub(1))
	}

	/// Iterates over elements front-to-back.
	pub fn iter(&self) -> Iter<'_, u8> {
		let (a, b) = self.as_slices();
		Iter {
			a: a.iter(),
			b: b.iter()
		}
	}

	/// Returns a pair of slices which contain the contents of the deque.
	pub fn as_slices(&self) -> (&[u8], &[u8]) {
		self.as_slices_in_range(..)
	}

	/// Returns a pair of mutable slices which contains the contents of the deque,
	/// or `None` if the deque is shared.
	pub fn as_mut_slices(&mut self) -> Option<(&mut [u8], &mut [u8])> {
		let (a, b) = self.slice_ranges(.., self.len);
		assert!(b.end <= a.start);

		let (buf_b, buf_a) = self.buf()?.split_at_mut(a.start);
		Some((buf_a, &mut buf_b[b]))
	}

	/// Returns a pair of slices which contain the contents of the deque within
	/// `range`.
	pub fn as_slices_in_range<R: RangeBounds<usize>>(&self, range: R) -> (&[u8], &[u8]) {
		let (a, b) = self.slice_ranges(range, self.len);
		(&self.buf[a], &self.buf[b])
	}

	/// Clears the deque.
	pub fn clear(&mut self) {
		self.head = 0;
		self.len = 0;
	}

	/// Removes the first element and returns it, or `None` if the deque is empty.
	pub fn pop_front(&mut self) -> Option<u8> {
		self.is_empty().then(|| {
			let head = self.head;
			self.head = self.wrap(1);
			self.len -= 1;
			self.buf[head]
		})
	}

	/// Removes the last element and returns it, or `None` if the deque is empty.
	pub fn pop_back(&mut self) -> Option<u8> {
		self.is_empty().then(|| {
			self.len -= 1;
			self.buf[self.wrap(self.len)]
		})
	}

	/// Inserts `value` at the front of the deque, returning it if the deque is
	/// full or shared.
	pub fn push_front(&mut self, value: u8) -> Option<u8> {
		if self.is_full() { return Some(value) }

		self.head = self.wrap_sub(1);
		self.len += 1;
		*self.index_mut(self.head)? = value;
		None
	}

	/// Inserts `value` at the back of the deque, returning it if the deque is
	/// full or shared.
	pub fn push_back(&mut self, value: u8) -> Option<u8> {
		if self.is_full() { return Some(value) }
		*self.index_mut(self.wrap(self.head))? = value;
		self.len += 1;
		None
	}

	/// Extends the deque with a slice `values`, returning the number of bytes
	/// written, or `None` if the deque is not writable.
	pub fn extend_n(&mut self, values: &[u8]) -> Option<usize> {
		let (a, b) = self.slice_ranges(.., self.len);
		let a_off = if b.is_empty() {
			a.end // start + len
		} else {
			b.end // tail_len
		};

		let buf = self.buf()?;
		let (empty_b, mut empty_a) = buf.split_at_mut(a.start);
		empty_a = &mut empty_a[a_off..];

		let len_a = empty_a.len();
		let len = if len_a > values.len() {
			empty_a.copy_from_slice(&values[..len_a]);
			len_a
		} else {
			let (src_a, src_b) = values.split_at(len_a);
			empty_a.copy_from_slice(src_a);
			empty_b[..src_a.len()].copy_from_slice(src_b);
			len_a + src_a.len()
		};

		self.len += len;
		Some(len)
	}

	/// Extends the deque with a slice `values`, returning the remaining slice, or
	/// the whole slice if the deque is shared.
	pub fn extend<'a>(&mut self, values: &'a [u8]) -> &'a [u8] {
		&values[self.extend_n(values).unwrap_or_default()..]
	}

	/// Drains the deque into a `target` slice, returning the number of bytes read.
	pub fn drain_n(&mut self, target: &mut [u8]) -> usize {
		target.copy_from_pair(self.as_slices());
		let len = target.len();
		self.remove_count(len);
		len
	}

	/// Drains the deque into a `target` slice, returning the unfilled slice.
	pub fn drain<'a>(&mut self, target: &'a mut [u8]) -> &'a mut [u8] {
		let n = self.drain_n(target);
		&mut target[n..]
	}

	/// Removes `count` bytes from the deque.
	pub fn remove_count(&mut self, count: usize) {
		assert!(count <= self.len);
		self.head = self.wrap(count);
		self.len -= count
	}

	/// Truncates the deque to `count` bytes, removing bytes from the back.
	pub fn truncate(&mut self, count: usize) {
		assert!(count <= self.len);
		self.len = count;
	}

	/// Shifts the internal memory of the deque such that it fits it one contiguous
	/// slice, returning the slice if successful or `None` if the deque is shared.
	///
	/// This allows [`as_slices`] and [`as_mut_slices`] to return the whole deque
	/// contents in the first slice.
	///
	/// [`as_slices`]: Self::as_slices
	/// [`as_mut_slices`]: Self::as_mut_slices
	pub fn shift(&mut self) -> Option<&mut [u8]> {
		// Huh, didn't know this worked.
		let &mut Self { head, len, .. } = self;

		if self.is_contiguous() {
			return self.index_mut(head..head + len);
		}

		let free = N - len;
		let head_len = N - head;
		let head_rng = head..head + head_len;
		let tail = N - head_len;
		let tail_len = tail;

		let buf = self.buf()?;
		self.head = if free >= head_len {
			buf.copy_within(..tail_len, head_len);
			buf.copy_within(head_rng, 0);
			0
		} else if free >= tail_len {
			buf.copy_within(head_rng, tail);
			buf.copy_within(..tail_len, tail + head_len);
			tail
		} else if head_len > tail_len {
			if free != 0 {
				buf.copy_within(..tail_len, free);
			}
			buf[free..].rotate_left(tail_len);
			free
		} else {
			if free != 0 {
				buf.copy_within(head_rng, tail_len);
			}
			buf[..len].rotate_right(head_len);
			0
		};

		let &mut Self { head, len, .. } = self;
		self.index_mut(head..head + len)
	}

	/// Consumes the deque, returning inner buffer if *exclusive* (unshared). The
	/// elements of this array are possibly uninitialized; this method is provided
	/// for pools to collect this memory and pass it back to this struct, where
	/// initialization is properly handled.
	pub fn into_inner(self) -> Option<Box<[u8; N]>> { Rc::into_inner(self.buf) }
}

impl<const N: usize> BlockDeque<N> {
	fn index_mut<I, T: ?Sized>(&mut self, idx: I) -> Option<&mut T>
	where [u8; N]: IndexMut<I, Output = T> {
		self.buf().map(|array| &mut array[idx])
	}

	fn buf(&mut self) -> Option<&mut [u8; N]> {
		Rc::get_mut(&mut self.buf).map(Box::as_mut)
	}

	fn is_contiguous(&self) -> bool {
		// Unlike with VecDeque, the capacity is checked by the compiler, so this
		// cannot overflow.
		self.head + self.len > N
	}

	fn wrap(&self, idx: usize) -> usize {
		Self::wrap_idx(self.head.wrapping_add(idx))
	}

	fn wrap_sub(&self, idx: usize) -> usize {
		Self::wrap_idx(self.head.wrapping_sub(idx).wrapping_add(N))
	}

	fn wrap_idx(idx: usize) -> usize {
		assert!(idx < N || (idx - N) < N);
		if idx >= N {
			idx - N
		} else {
			idx
		}
	}

	fn slice_ranges<R: RangeBounds<usize>>(&self, range: R, mut len: usize) -> (Range<usize>, Range<usize>) {
		let range = slice::range(range, ..len);
		len = range.len();

		if len == 0 {
			(0..0, 0..0)
		} else {
			let start = self.wrap(range.start);
			let head_len = N - start;

			if head_len >= len {
				(start..start + len, 0..0)
			} else {
				let tail_len = len - head_len;
				(start..N, 0..tail_len)
			}
		}
	}
}

impl<const N: usize> Default for BlockDeque<N> {
	fn default() -> Self { Self::new() }
}

impl<const N: usize> From<Box<[u8; N]>> for BlockDeque<N> {
	fn from(buf: Box<[u8; N]>) -> Self {
		Self { buf: Rc::new(buf), head: 0, len: 0 }
	}
}

impl<const N: usize> fmt::Debug for BlockDeque<N> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_list().entries(self.iter()).finish()
	}
}

impl<'a, T: 'a> Iterator for Iter<'a, T> {
	type Item = &'a T;

	fn next(&mut self) -> Option<&'a T> {
		let Self { a, b } = self;
		a.next().or_else(|| {
			mem::swap(a, b);
			a.next()
		})
	}

	fn size_hint(&self) -> (usize, Option<usize>) {
		let len = self.len();
		(len, Some(len))
	}

	fn last(mut self) -> Option<&'a T> { self.next_back() }

	fn fold<B, F>(self, mut acc: B, mut f: F) -> B
	where F: FnMut(B, Self::Item) -> B {
		acc = self.a.fold(acc, &mut f);
		self.b.fold(acc, &mut f)
	}
}

impl<'a, T: 'a> DoubleEndedIterator for Iter<'a, T> {
	fn next_back(&mut self) -> Option<&'a T> {
		let Self { a, b } = self;
		a.next().or_else(|| {
			mem::swap(a, b);
			b.next_back()
		})
	}

	fn rfold<B, F>(self, mut acc: B, mut f: F) -> B
	where F: FnMut(B, Self::Item) -> B {
		acc = self.b.rfold(acc, &mut f);
		self.a.rfold(acc, &mut f)
	}
}

impl<T> ExactSizeIterator for Iter<'_, T> {
	fn len(&self) -> usize {
		self.a.len() + self.b.len()
	}
}

impl<T> FusedIterator for Iter<'_, T> { }
