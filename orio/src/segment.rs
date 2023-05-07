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

mod memory;

use std::collections::VecDeque;
use std::cmp::min;
use itertools::FoldWhile::{Continue, Done};
use itertools::Itertools;
use crate::pool::Pool;
use memory::Memory;

pub const SIZE: usize = 8 * 1024;

/// A ring buffer of [`Segment`]s. Written segments are placed in sequence at the
/// front, empty segments at the back. While reading, segments are popped front to
/// back until an empty segment or the end of the buffer. While writing, empty
/// segments are popped from the back and written segments are inserted after the
/// last written segment.
#[derive(Clone, Debug, Default)]
pub(crate) struct SegmentRing {
	/// The backing ring buffer.
	ring: VecDeque<Segment>,
	/// The number of written segments.
	length: usize,
	/// The number of elements that can be written before the buffer is full.
	limit: usize,
	/// The number of elements in the buffer.
	count: usize,
}

impl SegmentRing {
	/// Returns the number of elements contained in all segments.
	pub fn count(&self) -> usize { self.count }
	/// Returns the number of elements that can be written to the segments before
	/// the next claim operation.
	pub fn limit(&self) -> usize { self.limit }
	/// Returns the number of written segments.
	pub(crate) fn len(&self) -> usize { self.length }
	/// Returns `true` if the buffer has no elements.
	pub fn is_empty(&self) -> bool { self.length == 0 }
	/// Returns `true` if the total void size exceeds `threshold`.
	pub(crate) fn void(&self, threshold: usize) -> bool {
		if threshold == 0 { return true }
		if self.count == 0 { return false }

		let last = self.length - 1;
		let mut voids = self.iter()
							.enumerate()
							.map(|(i, seg)|
								seg.mem.off() + if i < last {
									seg.limit()
								} else {
									0
								}
							);

		// Abort as soon as the sum exceeds the threshold, to avoid iterating all
		// segments unnecessarily.
		voids.fold_while(0, |mut sum, cur| {
			sum += cur;
			if sum < threshold {
				Continue(sum)
			} else {
				Done(sum)
			}
		}).is_done()
	}

	/// Pushes to the front of the buffer. Used for reading in case a segment can
	/// only be partially read.
	pub fn push_front(&mut self, seg: Segment) {
		self.count += seg.len();
		self.length += 1;
		self.ring.push_front(seg);
	}

	/// Pushes an empty segment to the back of the buffer.
	pub fn push_empty(&mut self, seg: Segment) {
		debug_assert!(seg.is_empty(), "only empty segments should be pushed to the back");
		self.limit += SIZE;
		self.ring.push_back(seg);
	}

	/// Inserts a written segment after the last in the buffer.
	pub fn push_laden(&mut self, seg: Segment) {
		assert!(!seg.is_empty(), "only laden segments should be inserted");
		let lim_change = self.get().map(Segment::limit).unwrap_or_default();
		let Self { ring, length, limit, count } = self;

		*limit -= lim_change;
		*limit += seg.limit();
		*count += seg.len();
		ring.insert(*length, seg);
		*length += 1;
	}

	/// Pops the back-most unfilled segment from the ring buffer. Used for writing.
	pub fn pop_back(&mut self) -> Option<Segment> {
		let seg = if self.has_empty() {
			// Faster to replace the popped segment with a fresh one from the back
			// if possible.
			self.ring.swap_remove_back(self.length)?
		} else {
			self.ring.pop_back()?
		};

		let Self { length, limit, count, .. } = self;
		*length -= 1;
		*count -= seg.len();
		*limit -= seg.limit();
		Some(seg)
	}

	/// Pops the front segment from the ring buffer. Used for reading.
	pub fn pop_front(&mut self) -> Option<Segment> {
		if self.is_empty() { return None }

		let Self { length, count, .. } = *self;
		let seg = self.ring.pop_front()?;

		debug_assert!(length > 0, "no segments after successful pop");
		debug_assert!(
			count >= seg.len(),
			"count ({count}) not large enough to contain the popped segment with \
			count {}",
			seg.len()
		);

		self.length -= 1;
		self.count -= seg.len();
		Some(seg)
	}

	pub fn extend_empty(&mut self, segments: impl IntoIterator<Item = Segment>) {
		let len = self.ring.len();
		self.ring.extend(segments);
		let count = self.ring.len() - len;

		self.limit += count * SIZE;
	}

	/// Iterates over written segments front to back.
	pub fn iter(&self) -> impl Iterator<Item = &Segment> {
		self.ring
			.iter()
			.take(self.length)
	}

	/// Reserves at least `count` bytes of segments, increasing [`Self::limit`] to
	/// `[n,n+N)`.
	pub fn reserve(&mut self, mut count: usize, pool: &mut impl Pool) {
		let Self { ring, .. } = self;
		let len = ring.len();

		pool.claim_size(ring, count);

		count = ring.len() - len;
		self.limit += count * SIZE;
	}

	/// Collects all segments into `pool`.
	pub fn clear(&mut self, pool: &mut impl Pool) {
		let Self { ring, length, limit, count } = self;
		pool.collect(ring.drain(..));
		*length = 0;
		*limit = 0;
		*count = 0;
	}

	/// Collects empty segments into `pool`.
	pub fn trim(&mut self, pool: &mut impl Pool, count: usize) {
		if !self.has_empty() { return }

		let Self { ring, length, limit, .. } = self;
		*limit -= (ring.len() - min(*length, count)) * SIZE;
		pool.collect(ring.drain(*length..).take(count));
	}

	fn has_empty(&self) -> bool { self.length < self.ring.len() }

	fn get(&self) -> Option<&Segment> {
		(!self.is_empty()).then(|| &self.ring[self.length])
	}
}

impl IntoIterator for SegmentRing {
	type Item = Segment;
	type IntoIter = <VecDeque<Segment> as IntoIterator>::IntoIter;

	fn into_iter(self) -> Self::IntoIter {
		self.ring.into_iter()
	}
}

impl Extend<Segment> for SegmentRing {
	fn extend<T: IntoIterator<Item = Segment>>(&mut self, iter: T) {
		self.extend_empty(iter);
	}

	fn extend_one(&mut self, item: Segment) {
		self.push_empty(item);
	}

	fn extend_reserve(&mut self, additional: usize) {
		self.ring.reserve(additional);
	}
}

/// A fixed-size size memory segment. Segment memory can be safely shared between
/// buffers; modifying shared segments will cause them to *fork* their memory (they
/// copy-on-write). Empty segments are reused by returning them to a global pool,
/// called *collection*.
///
/// Handling segments is not recommended. For a higher-level API, see [`Buffer`][1].
///
/// [1]: crate::Buffer
#[derive(Clone, Debug, Default)]
pub struct Segment {
	mem: Memory<u8, SIZE>,
}

impl Segment {
	fn new(mem: Memory<u8, SIZE>) -> Self {
		Self { mem }
	}

	/// Returns `true` if the segment is empty.
	pub fn is_empty(&self) -> bool { self.mem.is_empty() }
	/// Returns `true` if the segment is full.
	pub fn is_full(&self) -> bool { self.limit() == 0 }
	/// Returns the memory offset.
	pub(crate) fn off(&self) -> usize { self.mem.off() }
	/// Returns the number of bytes written to the segment.
	pub fn len(&self) -> usize { self.mem.cnt() }
	/// Returns the number of bytes that can be written to the segment.
	pub fn limit(&self) -> usize { self.mem.lim() }
	/// Returns a slice of bytes written to the segment. [`consume`][] must be
	/// called to complete a read operation.
	///
	/// [`consume`]: Self::consume
	pub fn data(&self) -> &[u8] { self.mem.data() }
	/// Returns a slice of unwritten bytes in the segment. [`grow`][] must be
	/// called to complete a write operation. Forks shared memory.
	///
	/// [`grow`]: Self::grow
	pub fn data_mut(&mut self) -> &mut [u8] { self.mem.data_mut() }
	/// Returns `true` if the segment is shared.
	pub fn is_shared(&self) -> bool { self.mem.is_shared() }

	/// Creates a new segment with memory shared from the current segment.
	pub fn share_all(&self) -> Self {
		Self::new(self.mem.share_all())
	}

	/// Creates a new segment with up to `count` bytes of memory shared from the
	/// current segment.
	pub fn share(&self, byte_count: usize) -> Self {
		Self::new(self.mem.share(byte_count))
	}

	/// Copies shared data into owned memory.
	pub fn fork(&mut self) {
		self.mem.fork();
	}

	/// Consumes `count` bytes after reading, without forking shared memory.
	pub fn consume(&mut self, byte_count: usize) { self.mem.consume(byte_count) }

	/// Truncates to `count` bytes, without forking shared memory.
	pub fn truncate(&mut self, byte_count: usize) { self.mem.truncate(byte_count) }

	/// Grows by `count` bytes after writing, forking if shared.
	pub fn grow(&mut self, byte_count: usize) {
		self.mem.fork();
		self.mem.grow(byte_count)
	}

	/// Clears the segment data, without forking shared memory.
	pub fn clear(&mut self) { self.mem.clear() }

	/// Shifts data back to offset zero, forking if shared.
	pub fn shift(&mut self) { self.mem.shift() }

	/// Pushes a byte onto the end of the segment, return `true` if the segment is
	/// not full. This operation forks shared memory.
	pub fn push(&mut self, byte: u8) -> bool { self.mem.push(byte) }

	/// Pops a byte from the start of the segment. This operation is non-forking.
	pub fn pop(&mut self) -> Option<u8> { self.mem.pop() }

	/// Pushes a slice of bytes onto the end of the segment, returning the number
	/// of bytes written. This operation forks shared memory.
	pub fn push_slice(&mut self, bytes: &[u8]) -> usize {
		self.mem.push_slice(bytes)
	}

	/// Pops data from the start of the segment into `bytes`, returning the number
	/// of bytes read. This operation is non-forking.
	pub fn pop_into_slice(&mut self, bytes: &mut [u8]) -> usize {
		self.mem.pop_into_slice(bytes)
	}

	/// Moves up to `byte_count` bytes from the current segment into a `target`,
	/// returning the number of bytes moved. Shared memory will only be forked in
	/// `target`.
	pub fn move_into(&mut self, target: &mut Self, offset: usize, mut byte_count: usize) -> usize {
		byte_count = self.copy_into(target, offset, byte_count);
		self.consume(byte_count);
		byte_count
	}

	/// Moves all data from the current segment into a `target`, returning the
	/// number of bytes moved. Shared memory will only be forked in `target`.
	pub fn move_all_into(&mut self, target: &mut Self, offset: usize) -> usize {
		let count = self.copy_all_into(target, offset);
		self.consume(count);
		count
	}

	/// Copies up to `byte_count` bytes from the current segment into a `target`,
	/// returning the number of bytes copied. Shared memory will only be forked in
	/// `target`.
	pub fn copy_into(&self, target: &mut Self, offset: usize, mut byte_count: usize) -> usize {
		if offset >= self.len() { return 0 }

		let data = self.data();
		byte_count = min(byte_count, data.len());
		byte_count = target.push_slice(&data[offset..byte_count]);
		byte_count
	}

	/// Copies all data from the current segment into a `target`, returning the
	/// number of bytes copied. Shared memory will only be forked in `target`.
	pub fn copy_all_into(&self, target: &mut Self, offset: usize) -> usize {
		if offset >= self.len() { return 0 }
		target.push_slice(&self.data()[offset..])
	}
	
	/// Inserts `other` at the front of the current segment.
	pub(crate) fn prefix_with(&mut self, other: &mut Self) {
		assert!(other.len() + self.len() <= SIZE, "`other` is too large");
		
		let len = other.len() + self.len();
		self.mem.shift_right(other.len());
		self.clear();
		other.move_all_into(self, 0);
		self.grow(len);
	}
}

#[cfg(test)]
mod test {
	use quickcheck::{Arbitrary, Gen};
	use quickcheck_macros::quickcheck;
	use super::memory::Memory;

	#[derive(Copy, Clone, Debug)]
	struct ArbArray([u8; 256]);

	impl Arbitrary for ArbArray {
		fn arbitrary(g: &mut Gen) -> Self {
			let mut array = [0; 256];
			for i in 0..256 {
				array[i] = u8::arbitrary(g);
			}
			Self(array)
		}
	}

	#[quickcheck]
	fn memory(ArbArray(values): ArbArray) {
		let mut mem: Memory<_, 256> = Memory::default();

		for i in 0..128 {
			assert!(mem.push(values[i]), "partial write: index {i} push");
			assert_eq!(mem.off(), 0, "partial write: index {i} offset");
			assert_eq!(mem.len(), i + 1, "partial write: index {i} length");
			assert_eq!(mem.cnt(), i + 1, "partial write: index {i} count");
		}

		for i in 0..128 {
			assert_eq!(mem.pop(), Some(values[i]), "partial read: index {i} pop");
			assert_eq!(mem.off(), i + 1, "partial read: index {i} offset");
			assert_eq!(mem.len(), 128, "partial read: index {i} length");
			assert_eq!(mem.cnt(), 127 - i, "partial read: index {i} count");
		}

		for i in 128..256 {
			assert!(mem.push(values[i]), "full write: index {i} push");
			assert_eq!(mem.off(), 128, "full write: index {i} offset");
			assert_eq!(mem.len(), i + 1, "full write: index {i} length");
			assert_eq!(mem.cnt(), i + 1 - 128, "full write: index {i} count");
		}

		for i in 128..256 {
			assert_eq!(mem.pop(), Some(values[i]), "full read: index {i} pop");
			assert_eq!(mem.off(), i + 1, "full read: index {i} offset");
			assert_eq!(mem.len(), 256, "full read: index {i} length");
			assert_eq!(mem.cnt(), 255 - i, "full read: index {i} count");
		}
	}

	/// Tests memory sharing and forking.
	#[quickcheck]
	fn memory_share(ArbArray(values): ArbArray) {
		let mut mem_a: Memory<_, 256> = Memory::default();
		mem_a.push_slice(&values);

		let mut mem_b = mem_a.share(128);

		let mut dst = [0; 128];
		assert_eq!(
			mem_b.pop_into_slice(&mut dst),
			128,
			"mem_b should fully pop a slice of size 128"
		);
		assert_eq!(mem_b.off(), mem_b.len(), "mem_b should have 128 bytes read");
		assert_eq!(mem_b.cnt(), 0, "mem_b should be empty");
		assert_eq!(&dst, &values[..128]);
		assert_eq!(
			mem_b.push_slice(&dst),
			128,
			"mem_b should fully push a 128 byte slice by forking"
		);
		assert_eq!(mem_b.data(), &dst, "mem_b data should match previously popped data");
		assert_eq!(mem_b.off(), 0, "mem_b should be shifted due to forking");
		assert_eq!(mem_b.len(), 128, "mem_b should have a length of 128 bytes");
		assert_eq!(mem_b.cnt(), 128, "mem_b should contain 128 bytes");
		assert_eq!(mem_a.data(), &values, "mem_a should be untouched after modifying mem_b");

		mem_a.clear();
		assert_eq!(mem_a.off(), 0, "mem_a should be cleared");
		assert_eq!(mem_a.len(), 0, "mem_a should be cleared");
		assert_eq!(mem_a.cnt(), 0, "mem_a should be cleared");
		assert_eq!(mem_b.data(), &dst, "mem_b should be untouched after modifying mem_a");
	}
}
