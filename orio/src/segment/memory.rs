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
use std::collections::Bound;
use std::ops::{Add, Index, IndexMut, Range, RangeBounds, RangeFrom, RangeFull, RangeTo, SubAssign};
use std::pin::Pin;
use std::rc::Rc;

// Location

#[derive(Copy, Clone)]
struct Loc<const N: usize> {
	start: usize,
	end: usize,
}

impl<const N: usize> Loc<N> {
	fn new(start: usize, end: usize) -> Self {
		Self {
			start,
			end,
		}
	}

	fn range(&self) -> Range<usize> { self.start..self.end }
	fn after(&self) -> RangeFrom<usize> { self.end.. }

	/// Returns the length of the location range.
	fn len(&self) -> usize { self.end - self.start }

	/// Shrinks the location range from the left (reading).
	fn shrink_left(&mut self, n: usize) {
		let start = self.start + n;
		if start <= N {
			self.start = start;
		}
	}

	/// Grows the location range from the right (writing).
	fn grow_right(&mut self, n: usize) {
		let end = self.end + n;
		if end <= N {
			self.end = end;
		}
	}

	/// Truncates the location range to at most `n` in length.
	fn truncate(&mut self, n: usize) {
		self.end = min(self.start + n, self.end);
	}

	fn reset(&mut self) {
		let Self { start, end } = self;
		*start = 0;
		*end   = N;
	}
}

impl<const N: usize> Default for Loc<N> {
	fn default() -> Self { Self::new(0, N) }
}

impl<const N: usize> From<Range<usize>> for Loc<N> {
	fn from(value: Range<usize>) -> Self {
		Self::new(
			min(value.start, N),
			min(value.end,   N),
		)
	}
}

impl<const N: usize> From<RangeFull> for Loc<N> {
	fn from(_: RangeFull) -> Self { Self::default() }
}

impl<const N: usize> From<RangeTo<usize>> for Loc<N> {
	fn from(value: RangeTo<usize>) -> Self {
		Self::new(
			0,
			min(value.end, N),
		)
	}
}

impl<const A: usize, const B: usize> Add<Loc<B>> for Loc<A> {
	type Output = Self;

	fn add(self, rhs: Loc<B>) -> Self {
		let Self { start: sa, end: ea } = self;
		let Loc  { start: sb, end: eb } = rhs;
		(min(sb + sa, ea)..min(eb + sa, ea)).into()
	}
}

impl<const N: usize> Add<RangeTo<usize>> for Loc<N> {
	type Output = Self;

	fn add(self, rhs: RangeTo<usize>) -> Self {
		let Self { start, end: end_a } = self;
		let RangeTo { end: end_b } = rhs;
		(start..min(end_a + start, end_b)).into()
	}
}

impl<const N: usize> SubAssign<usize> for Loc<N> {
	fn sub_assign(&mut self, rhs: usize) {
		self.start -= rhs;
		self.end   -= rhs;
	}
}

impl<const N: usize> RangeBounds<usize> for Loc<N> {
	fn start_bound(&self) -> Bound<&usize> { Bound::Included(&self.start) }
	fn   end_bound(&self) -> Bound<&usize> { Bound::Excluded(&self.end  ) }

	fn contains<U>(&self, item: &U) -> bool where usize: PartialOrd<U>,
												  U: ?Sized + PartialOrd<usize> {
		item >= &self.start &&
		item < &self.end
	}
}

// Memory

#[derive(Clone)]
struct MemoryData<const N: usize> {
	data: Pin<Box<[u8; N]>>,
	/// The bounds of written data. Can only be modified for owned memory, because
	/// invalidating shared data to the left would result in data loss, and adding
	/// data to the right would break the read-only contract.
	bounds: Loc<N>,
}

impl<const N: usize> MemoryData<N> {
	#[inline]
	fn new(data: Pin<Box<[u8; N]>>, loc: Loc<N>) -> Self {
		Self {
			data,
			bounds: loc,
		}
	}
	
	fn consume(&mut self, n: usize) {
		self.bounds.shrink_left(n);
	}

	fn add(&mut self, n: usize) {
		self.bounds.grow_right(n);
	}

	fn clear(&mut self) {
		self.bounds.reset();
	}
}

impl<const N: usize> Default for MemoryData<N> {
	#[inline]
	fn default() -> Self {
		Self::new(Box::pin([0; N]), Loc::default())
	}
}

impl<const N: usize> Index<usize> for MemoryData<N> {
	type Output = u8;
	fn index(&self, index: usize) -> &u8 {
		&self.data[index]
	}
}

impl<const N: usize> IndexMut<usize> for MemoryData<N> {
	fn index_mut(&mut self, index: usize) -> &mut u8 {
		&mut self.data[index]
	}
}

impl<const N: usize> Index<Loc<N>> for MemoryData<N> {
	type Output = [u8];

	fn index(&self, index: Loc<N>) -> &[u8] {
		&self.data[(self.bounds + index).range()]
	}
}

impl<const N: usize> IndexMut<Loc<N>> for MemoryData<N> {
	fn index_mut(&mut self, index: Loc<N>) -> &mut [u8] {
		&mut self.data[(self.bounds + index).range()]
	}
}

impl<const N: usize> Index<Range<usize>> for MemoryData<N> {
	type Output = [u8];

	fn index(&self, index: Range<usize>) -> &[u8] {
		&self.data[index]
	}
}

impl<const N: usize> IndexMut<Range<usize>> for MemoryData<N> {
	fn index_mut(&mut self, index: Range<usize>) -> &mut [u8] {
		&mut self.data[index]
	}
}

/// A sharable, fixed-size chunk of memory for [`Segment`]. Memory is copy-on-write
/// when shared, directly mutable when fully-owned. This way, expensive copies can
/// be avoided as much as possible; simple IO between buffers is almost zero-cost.
/// In addition, memory is pinned to the heap to avoid moves.
#[derive(Clone, Default)]
pub struct Memory<const N: usize> {
	data: Rc<MemoryData<N>>,
	loc: Loc<N>,
}

impl<const N: usize> Memory<N> {
	fn new(data: Rc<MemoryData<N>>, loc: Loc<N>) -> Self {
		Self {
			data,
			loc,
		}
	}

	fn start(&self) -> usize { self.loc.start }
	fn end  (&self) -> usize { self.loc.end   }

	pub fn off_start(&self) -> usize { self.start() + self.data.bounds.start }
	pub fn off_end  (&self) -> usize { self.end  () + self.data.bounds.end   }

	fn range_of(&self, n: usize) -> Range<usize> {
		self.start()..n + self.start()
	}

	/// Returns the length of this memory, the number of bytes that can be read.
	pub fn len(&self) -> usize { self.loc.len() }

	/// Returns the limit of this memory, the number of bytes that can be written.
	pub fn lim(&self) -> usize { N - self.off_end() }

	/// Shares all of this memory.
	pub fn share_all(&self) -> Self { self.clone() }

	/// Shares this memory with at most `byte_count` bytes accessible.
	pub fn share(&self, byte_count: usize) -> Self {
		let mut mem = self.share_all();
		mem.loc.truncate(byte_count);
		mem
	}

	/// Returns `true` if this memory is shared.
	pub fn is_shared(&self) -> bool { Rc::strong_count(&self.data) > 1 }

	/// Copies shared data into owned memory. Has no effect on already owned memory.
	pub fn fork(&mut self) -> bool {
		// Don't use make_mut because we're also shifting data while copying.
		if self.is_shared() {
			let forked = Box::pin([0; N]);
			let data = self.data();
			let range = ..data.len();
			&mut forked[range].copy_from_slice(data);

			self.loc = range.into();
			self.data = Rc::new(MemoryData::new(forked, self.loc));

			true
		} else {
			false
		}
	}

	/// Returns a slice of the data available for reading.
	pub fn data(&self) -> &[u8] { &self.data[self.loc] }

	/// Returns a mutable slice of the data available for writing, forking it if
	/// shared.
	pub fn data_mut(&mut self) -> &mut [u8] {
		self.fork();
		&mut self.data[self.loc]
	}

	/// Consumes data via a closure, shrinking the location range left by the
	/// returned byte count. Shorthand for:
	/// ```no_run
	/// let data = mem.data();
	/// let n = data.len();
	/// for byte in data.into_iter() {
	/// 	// do something
	/// 	print!("{byte}");
	/// }
	///
	/// mem.consume(n);
	/// ```
	pub fn consume_data(&mut self, consume: impl FnOnce(&[u8]) -> usize) {
		self.consume(consume(self.data()));
	}

	/// Adds data via a closure, growing the location range right by the returned
	/// byte count. Shorthand for:
	/// ```no_run
	/// let data = mem.data_mut();
	/// let n = data.len();
	/// for byte in data.iter_mut() {
	/// 	// do something
	/// 	*byte = 0;
	/// }
	///
	/// mem.add(n);
	/// ```
	pub fn add_data(&mut self, add: impl FnMut(&mut [u8]) -> usize) {
		self.add(add(self.data_mut()));
	}

	/// Pushes one byte to the memory, returning `true` if it could be written.
	pub fn push(&mut self, byte: u8) -> bool {
		if self.lim() > 0 {
			self.fork();
			self.data[self.end()] = byte;
			self.add(1);
			true
		} else {
			false
		}
	}

	/// Pops one byte from the memory.
	pub fn pop(&mut self) -> Option<u8> {
		if self.len() > 0 {
			let byte = Some(self.data[self.start()]);
			self.consume(1);
			byte
		} else {
			None
		}
	}

	/// Pushes a slice of bytes to the memory, returning the number of bytes written.
	pub fn push_slice(&mut self, bytes: &[u8]) -> usize {
		let cnt = min(self.lim(), bytes.len());
		if cnt > 0 {
			self.fork();
			&mut self.data[self.range_of(cnt)].copy_from_slice(&bytes[..cnt]);
			self.add(cnt);
			cnt
		} else {
			0
		}
	}

	/// Pops bytes into a slice from the memory, returning the number of bytes read.
	pub fn pop_into_slice(&mut self, bytes: &mut [u8]) -> usize {
		let cnt = min(self.len(), bytes.len());
		if cnt > 0 {
			&mut bytes[..cnt].copy_from_slice(&self.data[self.range_of(cnt)]);
			self.consume(cnt);
			cnt
		} else {
			0
		}
	}

	/// Consumes `n` bytes after reading.
	pub fn consume(&mut self, n: usize) {
		if !self.is_shared() {
			self.data.consume(n);
		}
		self.loc.shrink_left(n);
	}

	/// Adds `n` bytes after writing.
	pub fn add(&mut self, n: usize) {
		self.data.add(n);
		self.loc.grow_right(n);
	}

	/// Clears the memory, forking it if shared.
	pub fn clear(&mut self) {
		self.fork();
		self.data.clear();
		self.loc.reset();
	}

	/// Shifts the memory such that it starts at `0`, forking it if shared.
	pub fn shift(&mut self) {
		let n = self.loc.start;
		if n == 0 { return; }

		// Forked memory is already shifted.
		if !self.fork() {
			self.data.data.copy_within(self.data.bounds + self.loc, 0);
			self.loc         -= n;
			self.data.bounds -= n;
		}
	}

	/// Moves data from this memory to another, forking the other memory if shared.
	/// Returns the number of bytes moved.
	pub fn move_into(&mut self, other: &mut Memory<N>, byte_count: usize) -> usize {
		let cnt = min(self.len(), byte_count);
		let cnt = other.push_slice(&self.data[self.range_of(cnt)]);
		self.consume(cnt);
		cnt
	}
}

impl<const N: usize> From<[u8; N]> for Memory<N> {
	fn from(value: [u8; N]) -> Self {
		Self::new(
			Rc::new(
				MemoryData::new(
					Box::pin(value),
					Loc::default()
				)
			),
			Loc::default()
		)
	}
}
