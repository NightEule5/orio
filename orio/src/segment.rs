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
use std::collections::VecDeque;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::rc::Rc;
use crate::DEFAULT_SEGMENT_SIZE;

// Memory

/// A sharable, fixed-size chunk of memory for [`Segment`]. Memory is copy-on-write
/// when shared, directly mutable when fully-owned. This way, expensive copies can
/// be avoided as much as possible; simple IO between buffers is almost zero-cost.
/// In addition, memory is pinned to the heap to avoid moves.
#[derive(Clone)]
struct Memory<const N: usize>(Rc<Pin<Box<InnerMemory<N>>>>);

impl<const N: usize> Memory<N> {
	fn empty() -> Self { [0; N].into() }

	/// Returns a shared copy reference to the memory.
	fn share(&self) -> Self { self.clone() }
}

impl<const N: usize> Default for Memory<N> {
	fn default() -> Self { Self::empty() }
}

impl<const N: usize> From<[u8; N]> for Memory<N> {
	#[inline]
	fn from(value: [u8; N]) -> Self {
		Self(Rc::new(Box::pin(
			InnerMemory {
				data: value,
				len: 0,
			}
		)))
	}
}

impl<const N: usize> Deref for Memory<N> {
	type Target = InnerMemory<N>;
	fn deref(&self) -> &Self::Target {
		let Self(inner) = self;
		inner
	}
}

impl<const N: usize> DerefMut for Memory<N> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		let Self(inner) = self;
		Rc::make_mut(inner).deref_mut()
	}
}

#[derive(Copy, Clone)]
struct InnerMemory<const N: usize> {
	data: [u8; N],
	len: usize,
}

impl<const N: usize> InnerMemory<N> {
	/// Returns a slice of the data available for reading.
	fn data(&self, pos: usize) -> &[u8] {
		let to = pos + self.len;
		debug_assert!((pos..to).contains(&pos));
		&self.data[pos..to]
	}

	/// Returns a mutable slice of the data available for writing.
	fn data_mut(&mut self, pos: usize) -> &mut [u8] {
		let pos = pos + self.len;
		debug_assert!((0..=N).contains(&pos));
		&mut self.data[pos..]
	}

	fn len(&self) -> usize { self.len }

	fn inc_len(&mut self, n: usize) { self.len = min(self.len + n, N) }
	fn reset(&mut self) {
		self.len = 0;
	}
}

// Segment

/// A group [`Segment`]s contained in a ring buffer, with empty segments pushed to
/// the back and laden segments in front. To read and write, segments are pushed
/// and popped from the ring buffer.
pub struct Segments<const N: usize = DEFAULT_SEGMENT_SIZE> {
	end: usize,
	lim: usize,
	cnt: usize,
	ring: VecDeque<Segment<N>>,
}

impl<const N: usize> Segments<N> {
	/// Returns the number of bytes contained in all segments.
	pub fn count(&self) -> usize { self.cnt }
	/// Returns the number of bytes that can be written to the segments before the
	/// next claim operation.
	pub fn limit(&self) -> usize { self.lim }

	/// Pushes a segment to the ring buffer. Segments with data are appended after
	/// the last non-empty segment, empty segments are pushed to the back.
	pub fn push(&mut self, seg: Segment<N>) {
		if seg.is_empty() {
			self.lim += N;
			self.push_empty(seg);
		} else {
			self.cnt += seg.len();
			self.lim += seg.lim();
			self.lim -= self.get().lim();
			self.push_laden(seg);
		}
	}

	/// Pops the back-most unfilled [`Segment`] from the ring buffer. Used for
	/// writing.
	pub fn pop_back(&mut self) -> Option<Segment<N>> {
		let seg = if self.has_empty() {
			// Faster to replace the popped segment with an fresh one from the back
			// if possible.
			self.ring.swap_remove_back(self.end)
		} else {
			self.ring.pop_back()
		};

		let (len, lim) = seg.as_ref().map_or((0, 0), |seg| (seg.len(), seg.lim()));
		self.cnt -= len;
		self.lim -= lim;

		seg
	}

	/// Pops the front [`Segment`] from the ring buffer. Used for reading.
	pub fn pop_front(&mut self) -> Option<Segment<N>> {
		let seg = self.ring.pop_front();

		self.cnt -= seg.as_ref().map_or(0, |seg| seg.len());

		seg
	}

	/// Reserves at least `count` bytes of segments, increasing [`Self::limit`] to
	/// `[n,n+N)`.
	pub fn reserve(&mut self, count: usize) {
		self.claim_size(count);
	}

	/// Fills partial segments to free space, recycling empty segments after.
	pub fn compress(&mut self) {
		todo!()
	}

	fn get(&self) -> &Segment<N> {
		&self.ring[self.end]
	}

	fn has_empty(&self) -> bool {
		self.end < self.ring.len() - 1
	}

	fn push_empty(&mut self, seg: Segment<N>) {
		self.ring.push_back(seg);
	}

	fn push_laden(&mut self, seg: Segment<N>) {
		let full = seg.is_full();
		self.ring.insert(self.end, seg);
		self.end += 1;

		// Always keep an unfilled segment on hand.
		if full {
			self.claim();
		}
	}

	fn claim(&mut self) {
		// self.push_empty(claimed);
		todo!()
	}

	fn claim_size(&mut self, min_size: usize) {
		self.ring.reserve(min_size / N + (min_size % N > 0) as usize);
		todo!()
	}

	fn recycle(&self, seg: Segment<N>) {
		todo!()
	}
}

/// A fixed-size buffer segment.
pub struct Segment<const N: usize = DEFAULT_SEGMENT_SIZE> {
	mem: Memory<N>,
	pos: usize,
}

impl<const N: usize> Segment<N> {
	fn new(mem: Memory<N>) -> Self {
		Self {
			mem,
			pos: 0,
		}
	}

	/// Returns a new empty segment.
	pub fn empty() -> Self { Self::new(Memory::empty()) }

	/// Returns a new segment with copy-on-write memory shared from the current
	/// segment.
	pub fn share(&self) -> Self { Self::new(self.mem.share()) }

	/// Returns `true` if the segment is empty.
	pub fn is_empty(&self) -> bool { self.len() == 0 }
	/// Returns `true` if the segment is full.
	pub fn is_full (&self) -> bool { self.lim() == 0 }

	/// Returns a slice of the data available for reading.
	pub fn data(&self) -> &[u8] { self.mem.data(self.pos) }
	/// Returns a mutable slice of the data available for writing.
	pub fn data_mut(&mut self) -> &mut [u8] { self.mem.data_mut(self.pos) }

	/// Returns the position, from `[0,N]`.
	pub fn pos(&self) -> usize { self.pos }
	/// Returns the length, from `[0,N]`.
	pub fn len(&self) -> usize { self.mem.len }
	/// Returns the number of bytes that can be written to this segment.
	pub fn lim(&self) -> usize { N - (self.pos() + self.len()) }

	/// Clears the segment.
	pub fn clear(&mut self) {
		self.mem.reset();
		self.pos = 0;
	}
}

impl<const N: usize> From<[u8; N]> for Segment<N> {
	#[inline]
	fn from(value: [u8; N]) -> Self {
		Self::new(value.into())
	}
}

impl<const N: usize> From<&Memory<N>> for Segment<N> {
	fn from(value: &Memory<N>) -> Self {
		Self::new(value.share())
	}
}

impl<const N: usize> Default for Segment<N> {
	fn default() -> Self { Self::empty() }
}

/*
type FullRc<T> = StaticRc<CycNode<T>, 2, 2>;
type HalfRc<T> = StaticRc<CycNode<T>, 1, 2>;
type SplitRc<T> = (HalfRc<T>, HalfRc<T>);

struct CycNode<T> {
	value: T,
	prev: Option<HalfRc<T>>,
	next: Option<HalfRc<T>>,
}

impl<T> CycNode<T> {
	fn new(value: T) -> Self {
		Self {
			value,
			prev: None,
			next: None,
		}
	}

	fn is_orphaned(&self) -> bool {
		self.prev.is_none() &&
		self.next.is_none()
	}

	fn entangled(self) -> SplitRc<T> {
		FullRc::split(FullRc::new(self))
	}

	fn collapse(a: HalfRc<T>, b: HalfRc<T>) -> Self {
		FullRc::into_inner(FullRc::join(a, b))
	}

	fn link_next(mut self, mut node: Self) -> Self {
		assert!(
			node.is_orphaned(),
			"Multi-node chains cannot be merged. All node refs must be removed \
			before linking."
		);

		let (a, b) = node.entangled();

		// Replace the head's next reference to the tail node, which wraps around,
		// with a reference to the appending node.
		if let Some(mut tail) = self.next.replace(a) {
			// Replace the tail's prev reference to the head node with a reference
			// to the appending node.
			if let Some(head) = tail.prev.replace(b) {
				// Refer the appending node back to the previous head.
				let _ = node.prev.insert(head);
				// Refer the appending node forward to the tail.
				let _ = node.next.insert(tail);
			}
		} else {
			// The head is the only node, it becomes both the new tail (next) and
			// the prev.
			//       ╭─────────────────────╮
			//  ╭────↓────╮ ╭->╭─────────╮ │
			//╭─prev   next─╯╭─prev   next─╯
			//│ ╰─────────╯<-╯ ╰────↑────╯
			//╰─────────────────────╯
			let (head_a, head_b) = self.entangled();
			let _ = node.prev.insert(head_a);
			let _ = node.next.insert(head_b);
		}

		node
	}
}

impl<T> Deref for CycNode<T> {
	type Target = T;
	fn deref(&self) -> &Self::Target {
		&self.value
	}
}

impl<T> DerefMut for CycNode<T> {
	fn deref_mut(&mut self) -> &mut Self::Target {
		&mut self.value
	}
}*/
