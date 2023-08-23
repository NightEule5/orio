// SPDX-License-Identifier: Apache-2.0

mod memory;

use std::collections::VecDeque;
use std::cmp::min;
use std::iter::Sum;
use std::ops::RangeBounds;
use std::slice;
use itertools::Itertools;
use crate::pool::Pool;
use memory::Memory;

pub const SIZE: usize = 8 * 1024;

// Todo: count fragmentation after reading as well as after writing.

/// A ring buffer of [`Segment`]s. Written segments are placed in sequence at the
/// front, empty segments at the back. While reading, segments are popped front to
/// back until an empty segment or the end of the buffer. While writing, empty
/// segments are popped from the back and written segments are inserted after the
/// last written segment.
#[derive(Clone, Debug, Default)]
pub(crate) struct SegRing {
	/// The backing ring buffer.
	ring: VecDeque<Segment>,
	/// The number of written segments.
	len: usize,
	/// The number of bytes that can be written before the buffer is full.
	limit: usize,
	/// The number of bytes in the buffer.
	count: usize,
	/// The degree of fragmentation, measuring the total free space locked between
	/// partially written segments.
	frag: usize,
}

impl SegRing {
	/// Returns the number of segments written.
	pub fn len(&self) -> usize { self.len }
	/// Returns the number of bytes that can be written before claiming.
	pub fn limit(&self) -> usize { self.limit }
	/// Returns the number of bytes written.
	pub fn count(&self) -> usize { self.count }
	/// Returns the degree of fragmentation.
	pub fn frag(&self) -> usize { self.frag }

	/// Returns `true` if no data is written.
	pub fn is_empty(&self) -> bool { self.count == 0 }
	/// Returns `true` if data is written.
	pub fn is_not_empty(&self) -> bool { self.count > 0 }
	/// Returns `true` if the deque contains empty segments.
	fn has_empty(&self) -> bool { self.len < self.ring.len() }
	/// Returns a count of empty segments in the deque.
	fn empty_count(&self) -> usize { self.ring.len() - self.len }

	/// Returns the last non-empty segment, if any.
	pub fn last(&self) -> Option<&Segment> {
		if self.is_empty() {
			None
		} else {
			Some(&self.ring[self.len - 1])
		}
	}

	/// Pushes a partially read segment to the front of the deque.
	pub fn push_front(&mut self, seg: Segment) {
		if seg.is_empty() {
			self.push_empty(seg);
			return
		}

		self.len += 1;
		self.count += seg.len();

		if self.is_empty() {
			self.limit += seg.limit();
		} else {
			self.frag += seg.limit();
		}

		self.ring.push_front(seg);
	}

	/// Pushes a written or empty segment to the back of the deque.
	pub fn push_back(&mut self, seg: Segment) {
		if seg.is_empty() {
			self.push_empty(seg);
		} else {
			self.push_laden(seg);
		}
	}

	fn push_empty(&mut self, seg: Segment) {
		self.limit += SIZE;
		self.ring.push_back(seg);
	}

	fn push_laden(&mut self, seg: Segment) {
		let last_lim = self.last().map(Segment::limit).unwrap_or_default();
		self.limit -= last_lim;
		self.frag += last_lim;
		self.limit += seg.limit();
		self.count += seg.len();
		self.ring.insert(self.len, seg);
		self.len += 1;
	}

	/// Pops the front segment from the deque for reading.
	pub fn pop_front(&mut self) -> Option<Segment> {
		if self.is_empty() { return None }

		let seg = self.ring.pop_front()?;

		if self.is_empty() {
			self.limit -= seg.limit();
		} else {
			self.frag -= seg.limit();
		}

		self.len -= 1;
		self.count -= seg.len();
		Some(seg)
	}

	/// Pops the back-most unfilled segment from the deque for writing.
	pub fn pop_back(&mut self) -> Option<Segment> {
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

	/// Reads segments from the front of the deque, rotating empty segments to the
	/// back.
	pub fn read<T>(
		&mut self,
		read: impl FnOnce(&mut [Segment]) -> T
	) -> T {
		if self.is_empty() {
			return read(&mut [])
		}

		#[derive(Debug, Default)]
		struct Snapshot {
			count: usize,
			limit: usize
		}

		impl Sum for Snapshot {
			fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
				iter.reduce(|mut sum, Snapshot { count, limit }| {
					sum.count += count;
					sum.limit += limit;
					sum
				}).unwrap_or_default()
			}
		}

		impl Snapshot {
			fn take(segments: &[Segment]) -> Vec<Snapshot> {
				segments.iter()
						.map(|seg|
							Self {
								count: seg.len(),
								limit: seg.limit()
							}
						)
						.collect()
			}
		}

		let (mut front, _) = self.ring.as_mut_slices();
		front = &mut front[..self.len];

		let mut snap = Snapshot::take(front);
		let result = read(front);

		let n = front.iter()
					 .take_while(|seg| Segment::is_empty(seg))
					 .count();

		// If all segments were read, remove the last segment's limit from the
		// total and its "snapshot".
		if self.len > 0 && n == self.len {
			let ref mut last = snap[n - 1];
			self.limit -= last.limit;
			last.limit = 0;
		}

		let Snapshot { count, limit } = snap.into_iter().take(n).sum();
		self.count -= count;
		self.frag -= limit;
		self.consume_front(n);

		result
	}

	fn consume_front(&mut self, count: usize) {
		self.len -= count;
		self.limit += count * SIZE;
		self.ring.rotate_left(count);
	}

	/// Writes to segments from the back of the deque, rotating full segments to
	/// the front.
	pub fn write<T>(
		&mut self,
		write: impl FnOnce(&mut [Segment]) -> T
	) -> T {
		let mut empty_count = self.empty_count();
		let count = {
			let mut len = 0;
			if let Some(last) = self.last() {
				if !last.is_full() {
					empty_count += 1;
					len = last.len();
				}
			}
			len
		};
		self.limit += count;
		self.count -= count;

		self.ring.rotate_right(empty_count);
		self.limit -= empty_count * SIZE;

		let (mut front, _) = self.ring.as_mut_slices();
		front = &mut front[..empty_count];
		let result = write(front);

		let len = front.iter()
					   .position(|seg| seg.is_empty())
					   .unwrap_or(front.len());
		self.len = len;
		for (i, seg) in front[..len].iter().enumerate() {
			if i < len.saturating_sub(1) {
				self.frag += seg.limit();
			}
			self.count += seg.len();
		}

		self.limit += if len == 0 { 0 } else { front[len - 1].limit() };
		self.limit += (front.len() - len) * SIZE;

		let rot = front.len();
		self.ring.rotate_left(rot);

		result
	}

	/// Reserves at least `count` bytes of segments, increasing [`Self::limit`] to
	/// `[n,n+N)`.
	pub fn reserve(&mut self, count: usize, pool: &mut impl Pool) {
		pool.claim_size(self, count.saturating_sub(self.limit))
	}
	
	/// Clears and collects all segments into `pool`.
	pub fn clear(&mut self, pool: &mut impl Pool) {
		pool.collect(
			self.ring
				.drain(..)
				.update(Segment::clear)
		)
	}

	/// Collects up to `count` empty segments into `pool`.
	pub fn trim(&mut self, mut count: usize, pool: &mut impl Pool) {
		count = min(count, self.empty_count());
		if count == 0 { return }

		self.limit = count * SIZE;
		let len = self.len;
		pool.collect(self.ring.drain(len..len + count));
	}

	/// Iterates over written segments.
	pub fn iter(&self) -> impl Iterator<Item = &Segment> {
		self.ring
			.iter()
			.take(self.len)
	}

	/// Iterates over written segments a slices.
	pub fn slices(&self) -> impl Iterator<Item = &[u8]> {
		self.iter()
			.map(|s| s.data(..))
	}
}

impl Extend<Segment> for SegRing {
	fn extend<T: IntoIterator<Item = Segment>>(&mut self, iter: T) {
		let Self { ring, limit, .. } = self;
		let len = ring.len();

		ring.extend(iter);

		*limit += (ring.len() - len) * SIZE;
	}

	fn extend_reserve(&mut self, additional: usize) {
		self.ring.reserve(additional)
	}
}

#[cfg(test)]
mod ring_test {
	use std::error::Error;
	use crate::pool::VoidPool;
	use crate::segment::{SegRing, SIZE};

	#[test]
	fn write_read() -> Result<(), Box<dyn Error>> {
		let ref mut pool = VoidPool::default();
		let mut ring = SegRing::default();

		ring.reserve(7 * SIZE, pool);
		assert_eq!(ring.len, 0);
		assert_eq!(ring.limit, 7 * SIZE);
		assert_eq!(ring.count, 0);
		assert_eq!(ring.frag, 0);

		ring.write(|data| {
			for seg in &mut data[..3] {
				seg.grow(SIZE / 2);
			}
		});

		assert_eq!(ring.len, 3, "length after write");
		assert_eq!(ring.limit, 4 * SIZE + SIZE / 2, "limit after write");
		assert_eq!(ring.count, 3 * SIZE / 2, "count after write");
		assert_eq!(ring.frag, SIZE, "fragmentation after write");

		ring.read(|data| {
			for seg in &mut data[..3] {
				seg.consume(SIZE / 2);
			}
		});

		assert_eq!(ring.len, 0, "length after read");
		assert_eq!(ring.limit, 7 * SIZE, "limit after read");
		assert_eq!(ring.count, 0, "count after read");
		assert_eq!(ring.frag, 0, "fragmentation after read");

		Ok(())
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
	pub fn data<R: RangeBounds<usize>>(&self, range: R) -> &[u8] {
		let range = slice::range(range, ..self.len());
		&self.mem.data()[range]
	}
	/// Returns a slice of unwritten bytes in the segment. [`grow`][] must be
	/// called to complete a write operation. Forks shared memory.
	///
	/// [`grow`]: Self::grow
	pub fn data_mut<R: RangeBounds<usize>>(&mut self, range: R) -> &mut [u8] {
		let range = slice::range(range, ..self.limit());
		&mut self.mem.data_mut()[range]
	}
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

		byte_count = min(byte_count, self.len());
		byte_count = target.push_slice(self.data(offset..byte_count));
		byte_count
	}

	/// Copies all data from the current segment into a `target`, returning the
	/// number of bytes copied. Shared memory will only be forked in `target`.
	pub fn copy_all_into(&self, target: &mut Self, offset: usize) -> usize {
		if offset >= self.len() { return 0 }
		target.push_slice(self.data(offset..))
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
mod mem_test {
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
