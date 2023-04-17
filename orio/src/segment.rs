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
use crate::DEFAULT_SEGMENT_SIZE;
use crate::pool::{Pool, Result};

/// A group [`Segment`]s contained in a ring buffer, with empty segments pushed to
/// the back and laden segments in front. To read and write, segments are pushed
/// and popped from the ring buffer.
pub struct Segments<const N: usize = DEFAULT_SEGMENT_SIZE> {
	len: usize,
	lim: usize,
	cnt: usize,
	ring: VecDeque<Segment<N>>,
}

impl<const N: usize> Default for Segments<N> {
	fn default() -> Self { Self::new() }
}

impl<const N: usize> Segments<N> {
	pub fn new() -> Self {
		Self {
			len: 0,
			lim: 0,
			cnt: 0,
			ring: VecDeque::new(),
		}
	}

	/// Returns the number of bytes contained in all segments.
	pub fn count(&self) -> usize { self.cnt }
	/// Returns the number of bytes that can be written to the segments before the
	/// next claim operation.
	pub fn limit(&self) -> usize { self.lim }

	/// Pushes a segment to the ring buffer. Segments with data are appended after
	/// the last non-empty segment, empty segments are pushed to the back.
	pub fn push(&mut self, seg: Segment<N>) {
		if seg.is_empty() {
			self.push_empty(seg);
		} else {
			self.push_laden(seg);
		}
	}

	/// Pops the back-most unfilled [`Segment`] from the ring buffer. Used for
	/// writing.
	pub fn pop_back(&mut self) -> Option<Segment<N>> {
		let seg = if self.has_empty() {
			// Faster to replace the popped segment with a fresh one from the back
			// if possible.
			self.ring.swap_remove_back(self.len as usize)
		} else {
			self.ring.pop_back()
		};

		if seg.is_some() {
			self.len -= 1;
		}

		let (len, lim) = seg.as_ref().map_or((0, 0), |seg| (seg.len(), seg.lim()));
		self.cnt -= len;
		self.lim -= lim;

		seg
	}

	/// Pops the front [`Segment`] from the ring buffer. Used for reading.
	pub fn pop_front(&mut self) -> Option<Segment<N>> {
		let seg = self.ring.pop_front()?;
		self.len -= 1;
		self.cnt -= seg.len();
		Some(seg)
	}

	/// Reserves at least `count` bytes of segments, increasing [`Self::limit`] to
	/// `[n,n+N)`.
	pub fn reserve<P: Pool<N>>(&mut self, count: usize, pool: &mut P) -> Result {
		pool.claim_size(self, count)
	}

	/// Recycles all empty segments.
	pub fn trim<P: Pool<N>>(&mut self, pool: &mut P) -> Result {
		let range = self.len..self.ring.len();
		self.lim -= range.len() * N;
		pool.recycle(self.ring.drain(range))
	}

	/// Recycles all segments.
	pub fn clear<P: Pool<N>>(&mut self, pool: &mut P) -> Result {
		pool.recycle(self.ring.drain(..))
	}

	/// Pushes empty segments to the back of the ring buffer.
	pub fn extend_empty(&mut self, segments: impl IntoIterator<Item = Segment<N>>) {
		self.ring.extend(segments);
	}

	/// Fills partial segments to free space, optionally forcing compression of
	/// shared segments (triggering a copy).
	/// Todo: infer the force option with the void factor.
	pub fn compress(&mut self, force: bool) {
		let mut dst = VecDeque::with_capacity(self.ring.len());
		let mut empty = Vec::new();
		let mut prev = None;
		while let Some(mut curr) = self.pop_front()
									   .or_else(|| prev.take()) {
			if curr.is_empty() {
				empty.push(curr);
			} else if let Some(mut base) = prev.take() {
				if force || !base.mem.is_shared() {
					base.shift();
					base.move_into(&mut curr, N);

					if base.is_full() {
						dst.push_back(base);
					} else {
						let _ = prev.insert(base);
					}

					if curr.is_empty() || prev.is_some() {
						empty.push(curr);
					} else {
						let _ = prev.insert(curr);
					}
				} else if let Some(mut empty) = empty.pop() {
					// Move the shared memory to an empty segment, drop it, and
					// insert as the base.
					empty.move_into(&mut base, N);
					let _ = prev.insert(empty);
				} else {
					dst.push_back(base);
					let _ = prev.insert(curr);
				}
			} else {
				dst.push_back(curr);
			}
		}

		self.len = dst.len();
		self.lim = dst.back().map_or(0, Segment::lim) + empty.len() * N;
		dst.extend(empty);
		self.ring = dst;
	}

	fn get(&self) -> &Segment<N> {
		&self.ring[self.len]
	}

	fn has_empty(&self) -> bool { self.len < self.ring.len() }

	fn push_front(&mut self, seg: Segment<N>) {
		self.ring.push_front(seg);
		self.len += 1;
	}

	fn push_empty(&mut self, seg: Segment<N>) {
		self.lim += N;
		self.ring.push_back(seg);
	}

	fn push_laden(&mut self, seg: Segment<N>) {
		let cur_lim = if self.len == 0 { 0 } else { self.get().lim() };
		self.cnt += seg.len();
		self.lim += seg.lim();
		self.lim -= cur_lim;

		self.len += 1;
		self.ring.insert(self.len, seg);
	}
}

/// A fixed-size buffer segment.
pub struct Segment<const N: usize = DEFAULT_SEGMENT_SIZE> {
	mem: memory::Memory<N>,
}

impl<const N: usize> Segment<N> {
	fn new(mem: memory::Memory<N>) -> Self {
		Self { mem }
	}

	/// Returns a new empty segment.
	pub fn empty() -> Self { Self::new(memory::Memory::default()) }

	/// Returns a new segment with copy-on-write shared memory from the current
	/// segment.
	pub fn share_all(&self) -> Self { Self::new(self.mem.share_all()) }

	/// Returns a new segment with copy-on-write shared memory of length `byte_count`
	/// from the current segment.
	pub fn share(&self, byte_count: usize) -> Self { Self::new(self.mem.share(byte_count)) }

	/// Returns `true` if the segment is empty.
	pub fn is_empty(&self) -> bool { self.len() == 0 }
	/// Returns `true` if the segment is full.
	pub fn is_full (&self) -> bool { self.lim() == 0 }

	/// Returns a slice of the data available for reading.
	pub fn data(&self) -> &[u8] { self.mem.data() }
	/// Returns a mutable slice of the data available for writing.
	pub fn data_mut(&mut self) -> &mut [u8] { self.mem.data_mut() }

	/// Returns the position, from `[0,N]`.
	pub fn pos(&self) -> usize { self.mem.off_start() }
	/// Returns the length, from `[0,N]`.
	pub fn len(&self) -> usize { self.mem.len() }
	/// Returns the number of bytes that can be written to this segment.
	pub fn lim(&self) -> usize { self.mem.lim() }

	/// Clears the segment.
	pub fn clear(&mut self) {
		self.mem.clear();
	}

	/// Shifts the data back such that `pos` is 0.
	pub fn shift(&mut self) {
		self.mem.shift();
	}

	/// Consumes `n` bytes after reading.
	pub fn consume(&mut self, n: usize) {
		self.mem.consume(n);
	}

	/// Adds `n` bytes after writing.
	pub fn add(&mut self, n: usize) {
		self.mem.add(n);
	}

	/// Moves `byte_count` bytes into another segment, returning the number of bytes moved.
	pub fn move_into(&mut self, other: &mut Self, byte_count: usize) -> usize {
		self.mem.move_into(&mut other.mem, byte_count)
	}

	/// Pushes one byte to the segment, returning `true` if it could be written.
	pub fn push(&mut self, byte: u8) -> bool {
		self.mem.push(byte)
	}

	/// Pops one byte from the segment.
	pub fn pop(&mut self) -> Option<u8> {
		self.mem.pop()
	}

	/// Pushes a slice of bytes to the segment, returning the number of bytes
	/// written.
	pub fn push_slice(&mut self, bytes: &[u8]) -> usize {
		self.mem.push_slice(bytes)
	}

	/// Pops bytes into a slice from the segment, returning the number of bytes
	/// read.
	pub fn pop_into_slice(&mut self, bytes: &mut [u8]) -> usize {
		self.mem.pop_into_slice(bytes)
	}
}

impl<const N: usize> From<[u8; N]> for Segment<N> {
	#[inline]
	fn from(value: [u8; N]) -> Self {
		Self::new(value.into())
	}
}

impl<const N: usize> From<&memory::Memory<N>> for Segment<N> {
	fn from(value: &memory::Memory<N>) -> Self {
		Self::new(value.share_all())
	}
}

impl<const N: usize> Default for Segment<N> {
	fn default() -> Self { Self::empty() }
}
