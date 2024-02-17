// SPDX-License-Identifier: Apache-2.0

mod ring;
mod block_deque;
mod buffer;
mod util;

pub(crate) use ring::*;

use std::cmp::min;
use std::ops::{Index, IndexMut, RangeBounds};
use std::{mem, slice};
use std::mem::MaybeUninit;
use all_asserts::assert_ge;
use block_deque::BlockDeque;
pub(crate) use block_deque::{buf as alloc_block, Block};
use buffer::Buf;
use util::SliceExt;
use crate::pool::Pool;

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
#[derive(Clone, Debug, Eq)]
pub struct Seg<'d, const N: usize = SIZE>(Buf<'d, N>);

impl<'d, const N: usize, T: Into<Buf<'d, N>>> From<T> for Seg<'d, N> {
	fn from(value: T) -> Self { Self(value.into()) }
}

impl<const N: usize> Default for Seg<'_, N> {
	#[inline]
	fn default() -> Self { Self::new_block() }
}

impl<'d, const N: usize> Seg<'d, N> {
	/// Allocates a new block segment.
	pub fn new_block() -> Self {
		Self(Buf::Block(BlockDeque::new()))
	}

	/// Creates a segment containing `slice`.
	pub const fn from_slice(slice: &'d [u8]) -> Self {
		Self(Buf::Slice(slice))
	}

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
	/// Opposite of [`is_exclusive`].
	///
	/// [`is_exclusive`]: Self::is_exclusive
	pub fn is_shared(&self) -> bool {
		match &self.0 {
			Buf::Block(block) => block.is_shared(),
			Buf::Boxed(boxed) => boxed.is_shared(),
			Buf::Slice(_    ) => true,
		}
	}
	/// Returns `true` if the segment contains exclusively-owned data and can be
	/// written to. Opposite of [`is_shared`].
	///
	/// [`is_shared`]: Self::is_shared
	pub fn is_exclusive(&self) -> bool {
		!self.is_shared()
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

	/// Returns a pair of slices, in order, containing the segment contents within
	/// `range`.
	pub fn as_slices_in_range<R: RangeBounds<usize>>(&self, range: R) -> (&[u8], &[u8]) {
		match &self.0 {
			Buf::Block(block) => block.as_slices_in_range(range),
			Buf::Boxed(boxed) => boxed.as_slices_in_range(range),
			Buf::Slice(slice) => {
				let range = slice::range(range, ..slice.len());
				(&slice[range], &[])
			}
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

	/// Writes data from `other` into the segment to fill empty space if the segment
	/// is writable, returning the number of bytes written, or `None` if the segment
	/// contains shared data.
	pub fn write_from<const O: usize>(&mut self, other: &mut Seg<'_, O>) -> Option<usize> {
		let (a, b) = other.as_slices();
		let mut written = self.write(a)?;
		written += self.write(b)?;
		other.consume_unchecked(written);
		Some(written)
	}

	/// Reads the segment's contents into `buf` and consumes the data, returning
	/// the number of bytes read.
	pub fn read(&mut self, buf: &mut [u8]) -> usize {
		match &mut self.0 {
			Buf::Block(block) => block.drain_n(buf),
			_ => {
				let count = buf.copy_from_pair(
					self.as_slices_in_range(
						..buf.len().min(self.len())
					)
				);
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

	/// Pushes `value` to the back of the segment, returning it if the segment is
	/// not writable or full.
	pub fn push(&mut self, value: u8) -> Result<(), u8> {
		match &mut self.0 {
			Buf::Block(block) => block.push_back(value),
			Buf::Boxed(boxed) => {
				boxed.impose();
				let Some(target) = boxed.buf() else {
					return Err(value)
				};
				if target.capacity() - target.len() > 0 {
					target.push_back(value);
					boxed.len += 1;
					Ok(())
				} else {
					Err(value)
				}
			}
			Buf::Slice(_) => Err(value)
		}
	}

	/// Shares the segment's contents within `range`.
	pub fn share<R: RangeBounds<usize>>(&self, range: R) -> Seg<'d, N> {
		let range = slice::range(range, ..self.len());
		let mut seg = self.clone();
		seg. consume_unchecked(range.start);
		seg.truncate_unchecked(range.len());
		seg
	}

	/// Shares the segment's contents.
	pub fn share_all(&self) -> Seg<'d, N> { self.clone() }

	/// Consumes the segment, creating a new segment without borrowed data. Shared
	/// data is left alone. Panics if the segment is larger than the block size.
	pub(crate) fn detach<'de>(
		self,
		pool: &impl Pool<N>
	) -> Seg<'de, N> {
		match self {
			Self(Buf::Block(block)) => Seg(Buf::Block(block)),
			Self(Buf::Boxed(boxed)) => Seg(Buf::Boxed(boxed)),
			Self(Buf::Slice(slice)) => {
				assert_ge!(slice.len(), N);
				let mut target = pool.claim_one().unwrap_or_default();
				assert_eq!(
					target.write(slice).expect("claimed or allocated segment should be writable"),
					slice.len()
				);
				target
			}
		}
	}

	/// Splits a slice segment off into slices of length `N` or shorter.
	pub(crate) fn split_off_slice(&mut self) -> Option<(&'d [[u8; N]], &'d [u8])> {
		if let Self(Buf::Slice(slice)) = self {
			if slice.len() > N {
				let (chunks, remainder) = slice.as_chunks();
				*slice = &chunks[0];
				Some((&chunks[1..], remainder))
			} else {
				None
			}
		} else {
			None
		}
	}

	/// Consumes the segment and returns its inner block of memory, if any. This is
	/// intended for use by pool implementations to collect only blocks and discard
	/// shared data.
	pub fn into_block(self) -> Option<Box<[MaybeUninit<u8>; N]>> {
		if let Buf::Block(block) = self.0 {
			block.into_inner()
		} else {
			None
		}
	}

	/// Iterates over bytes in the segment.
	pub fn iter(&self) -> impl Iterator<Item = &u8> + '_ {
		self.0.iter()
	}
}

impl<'a, const N: usize> Seg<'a, N> {
	pub(crate) unsafe fn set_len(&mut self, count: usize) {
		let Buf::Block(block) = &mut self.0 else { return };
		block.set_len(count)
	}

	pub(crate) unsafe fn inc_len(&mut self, count: usize) {
		let Buf::Block(block) = &mut self.0 else { return };
		block.set_len(block.len() + count);
	}

	pub(crate) fn spare_capacity_mut(&mut self) -> (&mut [MaybeUninit<u8>], &mut [MaybeUninit<u8>]) {
		let Buf::Block(block) = &mut self.0 else {
			return (&mut [], &mut [])
		};
		block.spare_capacity_mut()
	}
}

impl<'d, const N: usize> Index<usize> for Seg<'d, N> {
	type Output = u8;

	fn index(&self, index: usize) -> &u8 {
		let (a, b) = self.as_slices();
		if index < a.len() {
			&a[index]
		} else {
			&b[index]
		}
	}
}

impl<'d, const N: usize> IndexMut<usize> for Seg<'d, N> {
	fn index_mut(&mut self, index: usize) -> &mut u8 {
		let (a, b) = self.as_mut_slices().expect(
			"segment should be exclusive"
		);
		if index < a.len() {
			&mut a[index]
		} else {
			&mut b[index]
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

impl<const N: usize, const O: usize> PartialEq<Seg<'_, O>> for Seg<'_, N> {
	fn eq(&self, other: &Seg<'_, O>) -> bool {
		self.0 == other.0
	}
}

impl<const N: usize> PartialEq<[u8]> for Seg<'_, N> {
	fn eq(&self, other: &[u8]) -> bool {
		&self.0 == other
	}
}

impl<const N: usize, T: AsRef<[u8]>> PartialEq<T> for Seg<'_, N> {
	fn eq(&self, other: &T) -> bool {
		self == other.as_ref()
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
		assert_eq!(seg.len(), len, "len == {len}");
		assert_eq!(seg.as_slices(), (SLICE, &[][..]), "contained bytes should match written bytes");
	}
}
