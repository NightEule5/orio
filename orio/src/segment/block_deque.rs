// SPDX-License-Identifier: Apache-2.0

//! A lightweight, fixed-size deque based on [`VecDeque`].
//!
//! [`VecDeque`]: std::collections::VecDeque

use std::{fmt, mem, slice};
use std::cmp::min;
use std::iter::{FusedIterator};
use std::mem::MaybeUninit;
use std::ops::{IndexMut, Range, RangeBounds};
use std::rc::Rc;
use all_asserts::assert_le;

pub type Block<const N: usize = { super::SIZE }> = Box<[MaybeUninit<u8>; N]>;

/// A lightweight, fixed-size deque based on [`VecDeque`].
///
/// Can be cloned in constant time (`O(1)`), sharing the same buffer across two or
/// more clones. Shared data can be read, but exclusivity is required to push new
/// data or modify existing data.
///
/// [`VecDeque`]: std::collections::VecDeque
#[derive(Clone)]
pub struct BlockDeque<const N: usize> {
	buf: Rc<Block<N>>,
	head: usize,
	len: usize
}

pub struct Iter<'a, T: 'a> {
	a: slice::Iter<'a, T>,
	b: slice::Iter<'a, T>,
}

pub fn buf<const N: usize>() -> Block<N> {
	Box::new([MaybeUninit::uninit(); N])
}

fn split_range_mut<T>(slice: &mut [T], mut a: Range<usize>, mut b: Range<usize>) -> (&mut [T], &mut [T]) {
	let is_overlapping = b.contains(&a.start) || (!a.is_empty() && b.contains(&(a.end - 1)));
	assert!(!is_overlapping);

	if a.end <= b.start {
		let (slice_a, slice_b) = slice.split_at_mut(a.end);
		b.start -= slice_a.len();
		b.end   -= slice_a.len();
		(&mut slice_a[a], &mut slice_b[b])
	} else {
		let (slice_b, slice_a) = slice.split_at_mut(b.end);
		a.start -= slice_b.len();
		a.end   -= slice_b.len();
		(&mut slice_a[a], &mut slice_b[b])
	}
}

impl<const N: usize> BlockDeque<N> {
	/// Creates a new deque.
	pub fn new() -> Self { buf().into() }
	pub fn from_array(array: [u8; N]) -> Self {
		let buf = Block::new(
			MaybeUninit::new(array).transpose()
		).into();
		Self { buf, head: 0, len: N }
	}
	/// Creates a new deque with bytes filling by `populate`.
	pub fn populated(populate: impl FnOnce(&mut [u8]) -> usize) -> Self {
		let mut b = buf();
		let len = populate(unsafe {
			MaybeUninit::slice_assume_init_mut(&mut b[..])
		});
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
	/// Returns `true` if the segment is contiguous.
	pub fn is_contiguous(&self) -> bool {
		// Unlike with VecDeque, the capacity is checked by the compiler, so this
		// cannot overflow.
		self.head + self.len <= N
	}

	/// Returns a reference to the element at `index`.
	pub fn get(&self, index: usize) -> Option<&u8> {
		(index < self.len).then(|| unsafe {
			MaybeUninit::assume_init_ref(&self.buf[self.wrap(index)])
		})
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
		self.buf().map(|buf| {
			let (a, b) = split_range_mut(buf, a, b);
			unsafe {
				(MaybeUninit::slice_assume_init_mut(a),
				 MaybeUninit::slice_assume_init_mut(b))
			}
		})
	}

	/// Returns a pair of slices which contain the contents of the deque within
	/// `range`.
	pub fn as_slices_in_range<R: RangeBounds<usize>>(&self, range: R) -> (&[u8], &[u8]) {
		let (a, b) = self.slice_ranges(range, self.len);
		unsafe {
			(MaybeUninit::slice_assume_init_ref(&self.buf[a]),
			 MaybeUninit::slice_assume_init_ref(&self.buf[b]))
		}
	}

	/// Clears the deque.
	pub fn clear(&mut self) {
		self.head = 0;
		self.len = 0;
	}

	/// Removes the first element and returns it, or `None` if the deque is empty.
	pub fn pop_front(&mut self) -> Option<u8> {
		(!self.is_empty()).then(|| {
			let head = self.head;
			self.head = self.wrap(1);
			self.len -= 1;
			unsafe {
				self.buf[head].assume_init()
			}
		})
	}

	/// Removes the last element and returns it, or `None` if the deque is empty.
	pub fn pop_back(&mut self) -> Option<u8> {
		(!self.is_empty()).then(|| {
			self.len -= 1;
			unsafe {
				self.buf[self.wrap(self.len)].assume_init()
			}
		})
	}

	/// Inserts `value` at the front of the deque, returning it if the deque is
	/// full or shared.
	pub fn push_front(&mut self, value: u8) -> Result<(), u8> {
		if self.is_full() { return Err(value) }

		self.head = self.wrap_sub(1);
		self.len += 1;
		let Some(front) = self.index_mut(self.head) else {
			return Err(value)
		};
		*front = value;
		Ok(())
	}

	/// Inserts `value` at the back of the deque, returning it if the deque is
	/// full or shared.
	pub fn push_back(&mut self, value: u8) -> Result<(), u8> {
		if self.is_full() { return Err(value) }
		let Some(back) = self.index_mut(self.wrap(self.len)) else {
			return Err(value)
		};
		*back = value;
		self.len += 1;
		Ok(())
	}

	/// Extends the deque with a slice `values`, returning the number of bytes
	/// written, or `None` if the deque is not writable.
	pub fn extend_n(&mut self, mut values: &[u8]) -> Option<usize> {
		if self.is_empty() {
			let buf = self.buf()?;
			let len = values.len().min(N);
			buf[..len].copy_from_slice(unsafe { mem::transmute(&values[..len]) });
			self.head = 0;
			self.len = len;
			return Some(len)
		}

		let head = self.head;
		let mut count = 0;

		// Tail
		let mut back_idx = self.wrap(self.len);
		if back_idx > head {
			let buf = self.buf()?;
			let dst = &mut buf[back_idx..N];
			let len = min(dst.len(), values.len());
			dst[..len].copy_from_slice(unsafe { mem::transmute(&values[..len]) });
			values = &values[len..];
			self.len += len;
			count = len;
		}

		// Head
		back_idx = self.wrap(self.len);
		if back_idx <= head {
			let buf = self.buf()?;
			let dst = &mut buf[back_idx..head];
			let len = min(dst.len(), values.len());
			dst[..len].copy_from_slice(unsafe { mem::transmute(&values[..len]) });
			self.len += len;
			count += len;
		}

		Some(count)
	}

	/// Extends the deque with a slice `values`, returning the remaining slice, or
	/// the whole slice if the deque is shared.
	pub fn extend<'a>(&mut self, values: &'a [u8]) -> &'a [u8] {
		&values[self.extend_n(values).unwrap_or_default()..]
	}

	/// Drains the deque into a `target` slice, returning the number of bytes read.
	pub fn drain_n(&mut self, mut target: &mut [u8]) -> usize {
		let len = target.len().min(self.len);
		let (a, b) = self.as_slices_in_range(..len);
		target[..a.len()].copy_from_slice(a);
		target = &mut target[a.len()..];
		target[..b.len()].copy_from_slice(b);
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

	/// Sets the deque length to `len`.
	///
	/// # Safety
	///
	/// Setting the length beyond the bounds of written bytes is undefined behavior,
	/// and could add uninitialized bytes.
	pub unsafe fn set_len(&mut self, len: usize) {
		assert_le!(len, N);
		self.len = len;
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
		let tail = len - head_len;
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

	/// Returns the remaining spare capacity of the deque as a pair of `MaybeUninit`
	/// slices. This can be used to fill the deque with data, before marking it as
	/// initialized with [`set_len`]. If the deque is shared, both slices will be
	/// empty.
	///
	/// This is not marked unsafe, because it has no effect on invariants without
	/// [`set_len`].
	///
	/// [`set_len`]: Self::set_len
	pub fn spare_capacity_mut(&mut self) -> (&mut [MaybeUninit<u8>], &mut [MaybeUninit<u8>]) {
		if self.is_empty() {
			self.clear();
		}
		
		let (a, b) = self.spare_capacity_ranges();
		let Some(buf) = self.buf() else {
			return (&mut [], &mut [])
		};
		split_range_mut(buf, a, b)
	}

	/// Consumes the deque, returning inner buffer if *exclusive* (unshared). The
	/// elements of this array are possibly uninitialized; this method is provided
	/// for pools to collect this memory and pass it back to this struct, where
	/// initialization is properly handled.
	pub fn into_inner(self) -> Option<Box<[MaybeUninit<u8>; N]>> { Rc::into_inner(self.buf) }
}

impl<const N: usize> BlockDeque<N> {
	fn index_mut<I, T: ?Sized>(&mut self, idx: I) -> Option<&mut T>
	where [u8]: IndexMut<I, Output = T> {
		self.buf().map(|array| {
			let array = unsafe {
				MaybeUninit::slice_assume_init_mut(array)
			};
			&mut array[idx]
		})
	}

	fn buf(&mut self) -> Option<&mut [MaybeUninit<u8>; N]> {
		Rc::get_mut(&mut self.buf).map(Box::as_mut)
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

	fn spare_capacity_ranges(&self) -> (Range<usize>, Range<usize>) {
		let back_idx = self.wrap(self.len);
		if back_idx >= self.head {
			(back_idx..N, 0..self.head)
		} else {
			(back_idx..self.head, 0..0)
		}
	}
}

impl<const N: usize> Default for BlockDeque<N> {
	fn default() -> Self { Self::new() }
}

impl<const N: usize> From<Box<[MaybeUninit<u8>; N]>> for BlockDeque<N> {
	fn from(buf: Box<[MaybeUninit<u8>; N]>) -> Self {
		Self { buf: Rc::new(buf.into()), head: 0, len: 0 }
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

impl<const N: usize> Eq for BlockDeque<N> { }

impl<const N: usize> PartialEq<[u8]> for BlockDeque<N> {
	fn eq(&self, other: &[u8]) -> bool {
		self.len == other.len() && self.iter().eq(other)
	}
}

impl<const N: usize, T: AsRef<[u8]>> PartialEq<T> for BlockDeque<N> {
	fn eq(&self, other: &T) -> bool {
		self == other.as_ref()
	}
}

impl<const N: usize, const O: usize> PartialEq<BlockDeque<O>> for BlockDeque<N> {
	fn eq(&self, other: &BlockDeque<O>) -> bool {
		self.len() == other.len() &&
		self.iter().eq(other.iter())
	}
}

#[cfg(test)]
mod test {
	use std::fmt;
	use std::fmt::{Debug, Formatter};
	use std::rc::Rc;
	use quickcheck::{Arbitrary, Gen};
	use quickcheck_macros::quickcheck;
	use super::BlockDeque;

	const SLICE: &[u8; 12] = b"Hello World!";

	/// A generated deque with arbitrary offset and length, with its length within
	/// the range set by [`MIN`] and [`MAX`].
	struct TestDeque<const MIN: usize, const MAX: usize> {
		deque: BlockDeque<12>,
		len: usize
	}

	impl<const MIN: usize, const MAX: usize> TestDeque<MIN, MAX> {
		fn new(off: usize, len: usize) -> Self {
			let mut slice = *SLICE;
			slice[len..].fill(0);
			if len > 0 {
				slice.rotate_right(off);
			}
			let mut deque = if len > 0 {
				BlockDeque::from_array(slice)
			} else {
				BlockDeque::new()
			};
			deque.head = off;
			deque.len  = len;
			Self { deque, len }
		}
	}

	impl<const MIN: usize, const MAX: usize> Clone for TestDeque<MIN, MAX> {
		fn clone(&self) -> Self {
			let mut deque = self.deque.clone();
			Rc::make_mut(&mut deque.buf);
			Self { deque, ..*self }
		}
	}

	impl<const MIN: usize, const MAX: usize> Debug for TestDeque<MIN, MAX> {
		fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
			struct Slice<'a>(&'a [u8]);

			impl Debug for Slice<'_> {
				fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
					f.debug_list().entries(self.0.iter()).finish()
				}
			}

			let (a, b) = self.deque.as_slices();
			f.debug_tuple("BlockDeque")
			 .field(&Slice(a))
			 .field(&Slice(b))
			 .finish()
		}
	}

	impl<const MIN: usize, const MAX: usize> Arbitrary for TestDeque<MIN, MAX> {
		fn arbitrary(g: &mut Gen) -> Self {
			let off = if MAX > 0 {
				usize::arbitrary(g) % MAX
			} else {
				usize::arbitrary(g) % 12
			};
			let len = if MAX > 0 {
				MIN + (usize::arbitrary(g) % (MAX + 1 - MIN))
			} else {
				0
			};
			Self::new(off, len)
		}

		fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
			let off = self.deque.head;
			let len = self.deque.len;
			Box::new(
				(MIN..=len).rev().flat_map(move |len|
					(0..off).rev().map(move |off| Self::new(off, len))
				)
			)
		}
	}

	#[quickcheck]
	fn push_front(TestDeque { mut deque, len }: TestDeque<0, 0>) {
		assert_eq!(deque, &[]);

		for (i, &b) in SLICE[len..].iter().rev().enumerate() {
			assert_eq!(deque.push_front(b), Ok(()), "push byte {i}");
			assert_eq!(deque, &SLICE[11 - i..], "byte {i}");
		}

		assert_eq!(deque.push_back(0), Err(0), "deque should be full but push was successful");
	}

	#[quickcheck]
	fn push_back(TestDeque { mut deque, len }: TestDeque<0, 0>) {
		assert_eq!(deque, &[]);

		for (i, &b) in SLICE[len..].iter().enumerate() {
			assert_eq!(deque.push_back(b), Ok(()), "push byte {i}");
			assert_eq!(deque, &SLICE[..=len + i], "byte {i}");
		}

		assert_eq!(deque.push_back(0), Err(0), "deque should be full but push was successful");
	}

	#[quickcheck]
	fn pop_front(TestDeque { mut deque, len }: TestDeque<12, 12>) {
		for (i, &b) in SLICE[..len].iter().enumerate() {
			assert_eq!(deque.pop_front(), Some(b), "pop byte {i}");
		}
	}

	#[quickcheck]
	fn pop_back(TestDeque { mut deque, len }: TestDeque<12, 12>) {
		for (i, &b) in SLICE[..len].iter().rev().enumerate() {
			assert_eq!(deque.pop_back(), Some(b), "pop byte {i}");
		}
	}

	#[quickcheck]
	fn share_read(TestDeque { deque: mut deque_a, len }: TestDeque<12, 12>) {
		let mut deque_b = deque_a.clone();
		for (i, &b) in SLICE[..len].iter().enumerate() {
			assert_eq!(deque_a.pop_front(), Some(b), "pop byte {i} from shared deque A");
			assert_eq!(deque_b.pop_front(), Some(b), "pop byte {i} from shared deque B");
		}
	}

	#[quickcheck]
	fn share_write(TestDeque { deque: mut deque_a, .. }: TestDeque<11, 11>) {
		let mut deque_b = deque_a.clone();
		assert_eq!(deque_a.push_back(0), Err(0), "pushing to shared deque should fail");
		assert_eq!(deque_b.push_back(0), Err(0), "pushing to shared deque should fail");
		drop(deque_b);
		assert_eq!(deque_a.push_back(0), Ok(()), "pushing to previously shared deque should succeed");
	}

	#[quickcheck]
	fn extend(TestDeque { mut deque, len }: TestDeque<0, 12>) {
		let remaining = deque.extend(&SLICE[len..]);
		assert_eq!(deque, SLICE);
		assert_eq!(remaining, &[]);
	}

	#[quickcheck]
	fn drain(TestDeque { mut deque, len }: TestDeque<1, 12>) {
		let mut data = [0; 12];
		let count = deque.drain_n(&mut data);
		let data = &data[..count];
		assert_eq!(data, &SLICE[..len]);
		assert!(deque.is_empty());
	}

	#[quickcheck]
	fn shift(TestDeque { mut deque, len }: TestDeque<1, 12>) {
		assert_eq!(deque.shift().unwrap(), &SLICE[..len]);
	}
}
