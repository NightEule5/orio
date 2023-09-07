// SPDX-License-Identifier: Apache-2.0

mod ring;
mod block_deque;
mod buffer;
mod util;

pub(crate) use ring::*;

use std::cmp::min;
use std::ops::RangeBounds;
use std::{mem, slice};
use block_deque::BlockDeque;
use buffer::Buf;
use util::SliceExt;

pub const SIZE: usize = 8192;

/// A sharable, ring buffer-like memory segment containing borrowed or owned data:
/// - An [`N`]-sized array (called a *block*)
/// - A variable-size vector from a boxed slice, or
/// - A borrowed slice
///
/// Can be cheaply cloned in O(1) time to *share data* between segments, such that
/// read operations avoid costly copies. Data can only be written if the segment is
/// not sharing its data with another segment, otherwise it must be copied to a new
/// buffer before writing.
///
/// Segments are designed to be reused to avoid allocating new memory. Segments can
/// be claimed from the segment [pool](super::pool), and collected into it when
/// finished. It's not recommended to let a segment drop instead of collecting it,
/// unless you're sure it contains shared data.
#[derive(Clone, Debug, Default)]
pub struct Seg<'d, const N: usize = SIZE>(Buf<'d, N>);

impl<'d, const N: usize, T: Into<Buf<'d, N>>> From<T> for Seg<'d, N> {
	fn from(value: T) -> Self { Self(value.into()) }
}

impl<'d, const N: usize> Seg<'d, N> {
	/// Returns the number of bytes in the segment.
	pub fn len(&self) -> usize { self.0.len() }
	/// Returns the number of bytes that can be written to the segment.
	pub fn limit(&self) -> usize { self.0.limit() }
	/// Returns the space, in bytes, occupied by the segment. The result is:
	/// - [`N`] for block segments,
	/// - The deque capacity for boxed segments, or
	/// - The length of slice segments
	pub fn size(&self) -> usize {
		match &self.0 {
			Buf::Block(_) => N,
			Buf::Boxed(boxed) => boxed.buf.capacity(),
			Buf::Slice(slice) => slice.len(),
		}
	}
	/// Returns `true` if the segment is empty.
	pub fn is_empty(&self) -> bool { self.len() == 0 }
	/// Returns `true` if the segment is not empty.
	pub fn is_not_empty(&self) -> bool { !self.is_empty() }
	/// Returns `true` if the segment is full.
	pub fn is_full(&self) -> bool { self.limit() == 0 }
	/// Returns `true` if the segment contains shared data and cannot be written to.
	pub fn is_shared(&self) -> bool {
		match &self.0 {
			Buf::Block(block) => block.is_shared(),
			Buf::Boxed(boxed) => boxed.is_shared(),
			Buf::Slice(_    ) => true,
		}
	}

	/// Clears data from the segment.
	pub fn clear(&mut self) {
		match &mut self.0 {
			Buf::Block(block) => block.clear(),
			Buf::Boxed(boxed) => boxed.clear(),
			Buf::Slice(slice) => *slice = &slice[..0],
		}
	}

	/// Returns a pair of slices, in order, containing the segment contents. If
	/// the segment data is contiguous, all data is contained by the first slice
	/// and the second is empty.
	pub fn as_slices(&self) -> (&[u8], &[u8]) {
		match &self.0 {
			Buf::Block(block) => block.as_slices(),
			Buf::Boxed(boxed) => boxed.as_slices(),
			Buf::Slice(slice) => (slice, &[]),
		}
	}

	/// Returns a pair of mutable slices, in order, containing the segment contents,
	/// or `None` if the segment contains shared data.
	pub fn as_mut_slices(&mut self) -> Option<(&mut [u8], &mut [u8])> {
		match &mut self.0 {
			Buf::Block(block) => block.as_mut_slices(),
			Buf::Boxed(boxed) => boxed.as_mut_slices(),
			_ => None
		}
	}

	/// Makes the segment writable if its contents are shared, by allocating a new
	/// block and copying the shared contents into it. If the data is too large for
	/// a single block, a segment containing the remaining shared data is returned.
	/// Claiming segments from a pool is recommended over this method, to avoid
	/// unnecessary allocation.
	///
	/// Methods requiring unique access, namely [`write`] and [`as_mut_slices`]
	/// *always* succeed after forking.
	///
	/// [`write`]: Self::write
	/// [`as_mut_slices`]: Self::as_mut_slices
	pub fn fork(&mut self) -> Option<Self> {
		if !self.is_shared() { return None }
		let buf = BlockDeque::populated(|buf| self.read(buf));
		let rem = mem::replace(self, buf.into());
		rem.is_not_empty().then_some(rem)
	}

	/// Consumes up to `count` elements, returning the number elements consumed.
	pub fn consume(&mut self, mut count: usize) -> usize {
		count = min(count, self.len());
		self.consume_unchecked(count);
		count
	}

	/// Truncates to a maximum of `count` elements, returning the element count.
	pub fn truncate(&mut self, mut count: usize) -> usize {
		count = min(count, self.len());
		self.truncate_unchecked(count);
		count
	}

	/// Copies the segment's contents into `buf`, returning the number of bytes
	/// copied.
	pub fn copy(&mut self, buf: &mut [u8]) -> usize {
		buf.copy_from_pair(self.as_slices())
	}

	/// Reads the segment's contents into `buf` and consumes the data, returning
	/// the number of bytes read.
	pub fn read(&mut self, buf: &mut [u8]) -> usize {
		match &mut self.0 {
			Buf::Block(block) => block.drain_n(buf),
			_ => {
				let count = buf.copy_from_pair(self.as_slices());
				self.consume_unchecked(count);
				count
			}
		}
	}

	/// Writes the contents of `buf` into the segment, returning the number of bytes
	/// written, or `None` if the segment contains shared data.
	pub fn write(&mut self, buf: &[u8]) -> Option<usize> {
		match &mut self.0 {
			Buf::Block(block) => block.extend_n(buf),
			Buf::Boxed(boxed) => {
				boxed.impose();
				let target = boxed.buf()?;
				let limit = target.capacity() - target.len();
				let count = min(limit, buf.len());
				target.extend(buf);
				boxed.len += count;
				Some(count)
			}
			_ => None
		}
	}

	/// Forks shared memory, then writes the contents of `buf` into the segment,
	/// returning the number of bytes written if successful. If the segment was too
	/// large to cleanly fit into a block, the remaining shared data is returned in
	/// `Err`. See [`fork`] for details.
	///
	/// [`fork`]: Self::fork
	pub fn force_write(&mut self, buf: &[u8]) -> Result<usize, Self> {
		match self.fork() {
			Some(rem) => Err(rem),
			None => Ok(
				self.write(buf)
					.expect("segment should be writable after clean fork")
			)
		}
	}

	/// If the segment is writable, shifts its contents such it fits in one contiguous
	/// slice, returning that slice. Returns `None` if the segment is not writable.
	pub fn shift(&mut self) -> Option<&mut [u8]> {
		match &mut self.0 {
			Buf::Block(block) => block.shift(),
			Buf::Boxed(boxed) => {
				boxed.impose();
				Some(boxed.buf()?.make_contiguous())
			}
			_ => None
		}
	}

	/// Shares the segment's contents within `range`.
	pub fn share<R: RangeBounds<usize>>(&self, range: R) -> Self {
		let range = slice::range(range, ..self.len());
		let mut seg = self.clone();
		seg. consume_unchecked(range.start);
		seg.truncate_unchecked(range.len());
		seg
	}

	/// Shares the segment's contents.
	pub fn share_all(&self) -> Self { self.clone() }

	/// Consumes the segment and returns its inner block of memory, if any. This is
	/// intended for use by pool implementations to collect only blocks and discard
	/// shared data.
	pub fn into_block(self) -> Option<Box<[u8; N]>> {
		if let Buf::Block(block) = self.0 {
			block.into_inner()
		} else {
			None
		}
	}
}

impl<'d, const N: usize> Seg<'d, N> {
	fn consume_unchecked(&mut self, count: usize) {
		match &mut self.0 {
			Buf::Block(block) => block.remove_count(count),
			Buf::Boxed(boxed) => {
				boxed.off += count;
				boxed.len -= count;
				boxed.impose();
			}
			Buf::Slice(slice) => *slice = &slice[min(slice.len(), count)..],
		}
	}

	fn truncate_unchecked(&mut self, count: usize) {
		if count == 0 {
			self.clear();
			return
		}

		match &mut self.0 {
			Buf::Block(block) => block.truncate(count),
			Buf::Boxed(boxed) => {
				boxed.len = count;
				boxed.impose();
			}
			Buf::Slice(slice) => *slice = &slice[..count],
		}
	}
}

#[cfg(test)]
mod test {
	use super::Seg;

	const SLICE: &[u8] = b"Hello World!";

	#[test]
	fn slice_read() {
		let mut word1 = [0; 5];
		let mut word2 = [0; 5];
		let mut seg: Seg = Seg::from(SLICE);
		assert_eq!(seg.read(&mut word1), 5, "should read 5 bytes");
		assert_eq!(seg.consume(1), 1, "should consume 1 byte");
		assert_eq!(seg.read(&mut word2), 5, "should read 5 bytes");
		assert_eq!(seg.consume(1), 1, "should consume 1 byte");
		assert!(seg.is_empty(), "should be empty");
		assert_eq!(&word1, b"Hello");
		assert_eq!(&word2, b"World");
	}

	#[test]
	fn slice_write() {
		let len = SLICE.len();
		let mut seg: Seg = Seg::default();
		assert_eq!(seg.write(SLICE), Some(len), "should write {len} bytes");
		assert_eq!(seg.off, 0, "off == 0");
		assert_eq!(seg.len, len, "len == {len}");
		assert_eq!(seg.as_slice(), SLICE, "contained bytes should match written bytes");
		assert_eq!(seg.off, 0, "off == 0");
		assert_eq!(seg.len, len, "len == {len}");
	}
}