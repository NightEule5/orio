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

use std::cmp::{max, min};
use std::ops::Range;
use std::pin::Pin;
use std::rc::Rc;
use crate::element::StreamElement;

/// A block of heap-allocated, pinned memory of size [`N`].
#[derive(Clone, Debug)]
struct Block<T: StreamElement, const N: usize> {
	/// The pinned, boxed data array.
	data: Pin<Box<[T; N]>>,
	/// The offset of valid data, advanced while reading.
	offset: usize,
	/// The length of valid data including the offset, advanced while writing.
	length: usize,
}

impl<T: StreamElement, const N: usize> Block<T, N> {
	fn off(&self) -> usize { self.offset }
	fn len(&self) -> usize { self.length }
	fn range(&self) -> Range<usize> {
		self.off()..self.len()
	}

	fn data(&self) -> &[T] {
		&self.data[self.range()]
	}

	fn data_mut(&mut self) -> &mut [T] {
		let len = self.len();
		&mut self.data[len..]
	}

	fn consume(&mut self, count: usize) -> usize {
		self.offset = min(self.offset + count, self.length);
		self.offset
	}

	fn truncate(&mut self, count: usize) -> usize {
		self.length = max(self.length.saturating_sub(count), self.offset);
		self.length
	}

	fn grow(&mut self, count: usize) -> usize {
		self.length = min(self.length + count, N);
		self.length
	}

	fn reset(&mut self) {
		self.offset = 0;
		self.length = 0;
	}

	fn shift(&mut self) {
		let range = self.range();
		self.data.copy_within(range, 0);
		let off = self.off();
		self.offset = 0;
		self.length -= off;
	}
	
	fn shift_right(&mut self, count: usize) {
		assert!(self.len() + count < N);

		let range = self.range();
		let off = self.off();
		self.data.copy_within(range, off + count);
		self.offset += count;
		self.length += count;
	}
}

impl<T: StreamElement, const N: usize> Default for Block<T, N> {
	fn default() -> Self { Box::pin([T::default(); N]).into() }
}

impl<T: StreamElement, const N: usize> From<Pin<Box<[T; N]>>> for Block<T, N> {
	fn from(data: Pin<Box<[T; N]>>) -> Self {
		Self {
			data,
			offset: 0,
			length: 0
		}
	}
}

impl<T: StreamElement, const N: usize> AsRef<[T]> for Block<T, N> {
	fn as_ref(&self) -> &[T] { self.data() }
}

impl<T: StreamElement, const N: usize> AsMut<[T]> for Block<T, N> {
	fn as_mut(&mut self) -> &mut [T] { self.data_mut() }
}

#[derive(Clone, Debug, Default)]
pub struct Memory<T: StreamElement, const N: usize> {
	block: Rc<Block<T, N>>,
	offset: usize,
	length: usize,
}

impl<T: StreamElement, const N: usize> Memory<T, N> {
	pub fn off(&self) -> usize { self.offset }
	pub fn len(&self) -> usize { self.length }
	pub fn cnt(&self) -> usize { self.length - self.off() }
	pub fn lim(&self) -> usize { N - self.length }

	pub fn is_empty(&self) -> bool {
		self.offset >= self.length
	}

	/// Returns a slice of readable data.
	pub fn data(&self) -> &[T] {
		let block = &*self.block;
		&block.data[self.off()..self.len()]
	}

	/// Returns a slice of unwritten data.
	pub fn data_mut(&mut self) -> &mut [T] {
		self.fork();
		let len = self.len();
		&mut self.block().data[len..]
	}

	/// Shares all of this memory.
	pub fn share_all(&self) -> Self { self.clone() }

	/// Shares this memory with at most `count` bytes accessible.
	pub fn share(&self, count: usize) -> Self {
		let mut mem = self.share_all();
		mem.truncate(count);
		mem
	}

	/// Returns `true` if this memory is shared.
	pub fn is_shared(&self) -> bool { Rc::strong_count(&self.block) > 1 }

	/// Copies shared data into owned memory. Has no effect on already owned memory.
	pub fn fork(&mut self) -> bool {
		if self.is_shared() {
			let mut dst = Block::default();
			let src = self.data();
			let len = src.len();
			dst.as_mut()[..len].copy_from_slice(src);

			self.offset = 0;
			self.length = len;
			self.block = Rc::new(dst);
			true
		} else {
			false
		}
	}

	/// Consumes `count` elements after reading.
	pub fn consume(&mut self, count: usize) {
		self.offset = if self.is_shared() {
			min(self.offset + count, self.length)
		} else {
			self.block().consume(count)
		}
	}

	/// Truncates to `count` elements.
	pub fn truncate(&mut self, count: usize) {
		self.length = if self.is_shared() {
			max(self.length.saturating_sub(count), self.offset)
		} else {
			self.block().truncate(count)
		}
	}

	/// Grows by `count` elements after writing.
	pub fn grow(&mut self, count: usize) {
		self.length = self.block().grow(count);
	}

	/// Clears all data without forking.
	pub fn clear(&mut self) {
		self.offset = 0;
		self.length = 0;
		if !self.is_shared() {
			self.block().reset();
		}
	}

	/// Shifts data such that the offset is zero, forking if shared.
	pub fn shift(&mut self) {
		// Forked memory is already shifted.
		if !self.fork() && self.offset > 0 {
			self.sync_loc();
			self.block().shift();
		}
	}
	
	/// Shifts data to the right by `count` bytes, forking if shared.
	pub fn shift_right(&mut self, count: usize) {
		// The fork function shifts data, we can just clone since we're moving data
		// afterward anyway.
		let mem = Rc::make_mut(&mut self.block);
		mem.offset = self.offset;
		mem.length = self.length;
		mem.shift_right(count);
		self.offset = mem.offset;
		self.length = mem.length;
	}

	/// Pushes one element to the memory, returning `true` if it could be written.
	pub fn push(&mut self, value: T) -> bool {
		if self.cnt() == N {
			return false
		}

		self.data_mut()[0] = value;
		self.grow(1);
		true
	}

	/// Pops one element from the memory.
	pub fn pop(&mut self) -> Option<T> {
		if self.is_empty() {
			return None
		}

		let value = self.data()[0];
		self.consume(1);
		Some(value)
	}

	/// Pushes a slice of elements to the memory, returning the number of bytes
	/// written.
	pub fn push_slice(&mut self, values: &[T]) -> usize {
		let cnt = min(self.lim(), values.len());
		if cnt > 0 {
			self.data_mut()[..cnt].copy_from_slice(&values[..cnt]);
			self.grow(cnt);
			cnt
		} else {
			0
		}
	}

	/// Pops elements into a slice from the memory, returning the number of bytes
	/// read.
	pub fn pop_into_slice(&mut self, values: &mut [T]) -> usize {
		let cnt = min(self.len(), values.len());
		if cnt > 0 {
			values[..cnt].copy_from_slice(&self.data()[..cnt]);
			self.consume(cnt);
			cnt
		} else {
			0
		}
	}

	/// Gets a mutable reference to the memory block. Panics if shared.
	fn block(&mut self) -> &mut Block<T, N> {
		Rc::get_mut(&mut self.block).expect("block must be owned")
	}

	/// Moves location data to an owned block.
	fn sync_loc(&mut self) {
		if self.is_shared() { return }

		let Self { offset, length, .. } = *self;
		let block = self.block();
		block.offset = offset;
		block.length = length;
	}
}
