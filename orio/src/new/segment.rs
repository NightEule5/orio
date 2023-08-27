// SPDX-License-Identifier: Apache-2.0

use std::cmp::min;
use std::ops::RangeBounds;
use std::pin::Pin;
use std::rc::Rc;
use std::{mem, slice};
use std::borrow::Cow;
use std::ptr::slice_from_raw_parts_mut;
use super::element::Element;
use super::pool::MutPool;

pub const SIZE: usize = 8192;

pub(crate) type Block<const N: usize, T> = Pin<Box<[T; N]>>;
pub(crate) type SharedBlock<const N: usize, T> = Rc<Block<N, T>>;

#[derive(Clone)]
enum Buf<'d, const N: usize, T> {
	Empty,
	Block(SharedBlock<N, T>),
	Boxed(Rc<Box<[T]>>),
	Slice(&'d [T]),
}

impl<const N: usize, T> Buf<'_, N, T> {
	fn as_writable(&mut self) -> Option<&mut Block<N, T>> {
		if let Buf::Block(block) = self {
			Rc::get_mut(block)
		} else {
			None
		}
	}

	fn into_writable(self) -> Option<Block<N, T>> {
		if let Buf::Block(block) = self {
			Rc::into_inner(block)
		} else {
			None
		}
	}
}

impl<'d, const N: usize, T> From<SharedBlock<N, T>> for Buf<'d, N, T> {
	fn from(value: SharedBlock<N, T>) -> Self { Self::Block(value) }
}

impl<'d, const N: usize, T> From<Rc<Box<[T]>>> for Buf<'d, N, T> {
	fn from(value: Rc<Box<[T]>>) -> Self { Self::Boxed(value) }
}

impl<'d, const N: usize, T> From<Box<[T]>> for Buf<'d, N, T> {
	fn from(value: Box<[T]>) -> Self {
		Rc::new(value).into()
	}
}

impl<'d, const N: usize, T> From<&'d [T]> for Buf<'d, N, T> {
	fn from(value: &'d [T]) -> Self { Self::Slice(value) }
}

/// A sharable memory segment containing borrowed or owned data:
/// - An [`N`]-sized array (called a *block*)
/// - A variable-size boxed array, or
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
#[derive(Clone)]
pub struct Seg<'d, const N: usize = SIZE, T: Element = u8> {
	buf: Buf<'d, N, T>,
	off: usize,
	len: usize
}

impl<const N: usize, T: Element> Seg<'_, N, T> {
	/// Returns the length of data contained in the segment.
	pub fn len(&self) -> usize {
		match &self.buf {
			Buf::Empty => 0,
			Buf::Slice(slice) => slice.len(),
			_ => self.len.saturating_sub(self.off)
		}
	}

	/// Returns `true` if the segment is empty.
	pub fn is_empty(&self) -> bool { self.len() == 0 }

	/// Clears all data from the segment.
	pub fn clear(&mut self) {
		if self.is_shared() {
			// Buffer is shared, we can safely drop it.
			self.buf = Buf::Empty;
		}

		self.off = 0;
		self.len = 0;
	}

	/// Returns a slice over the readable portion of the segment.
	pub fn as_slice(&self) -> &[T] {
		match &self.buf {
			Buf::Empty => &[],
			Buf::Block(block) => &block[self.off..self.len],
			Buf::Boxed(boxed) => &boxed[self.off..self.len],
			Buf::Slice(slice) => slice
		}
	}

	/// Returns a mutable slice over the writable portion of the segment.
	pub fn as_mut_slice(&mut self) -> Option<&mut [T]> {
		let block = self.buf.as_writable()?;
		Some(&mut block[self.len..])
	}

	/// Returns `true` if the segment can be written to.
	pub fn is_writable(&self) -> bool {
		matches!(&self.buf, Buf::Block(block) if Rc::strong_count(block) == 1)
	}

	/// Returns `true` if the segment contains shared data.
	pub fn is_shared(&self) -> bool { !self.is_writable() }

	/// Makes the segment writable by copying its elements to a fresh memory block
	/// claimed from `pool`, if not already writable. If the length of contained
	/// data exceeds the block size, [`N`], the remainder will be returned in a
	/// segment wrapped in `Some`. Otherwise, `None` is returned.
	///
	/// [`write`][] and [`as_mut_slice`][] *always* succeed after the segment is
	/// made writable.
	///
	/// [`write`]: Self::write
	/// [`as_mut_slice`]: Self::as_mut_slice
	pub fn fork(&mut self, pool: &mut dyn MutPool<N, T>) -> Option<Self> {
		if self.is_writable() { return None }

		let mut dst = pool.claim_one();
		dst.clear();
		let dst_slice = dst.as_mut_slice().expect("claimed segment should be writable");

		dst.len = self.read(dst_slice);
		let rem = mem::replace(self, dst);

		if rem.is_empty() {
			pool.collect_one(rem);
			None
		} else {
			Some(rem)
		}
	}

	/// Consumes up to `count` elements, returning the number elements consumed.
	pub fn consume(&mut self, mut count: usize) -> usize {
		let len = if let Buf::Slice(slice) = self.buf {
			slice.len()
		} else {
			self.len.saturating_sub(self.off)
		};
		count = min(len, count);
		self.consume_unchecked(count);
		count
	}

	/// Truncates to a maximum of `count` elements, returning the element count.
	pub fn truncate(&mut self, mut count: usize) -> usize {
		let len = if let Buf::Slice(slice) = self.buf {
			slice.len()
		} else {
			self.len.saturating_sub(self.off)
		};
		count = min(len, count);
		self.trunc_unchecked(count);
		count
	}

	/// Grows the segment by `count` elements if writable, returning the number of
	/// elements it grows by.
	pub fn grow(&mut self, mut count: usize) -> Option<usize> {
		self.is_writable().then(|| {
			count = min(count, N);
			self.len += count;
			count
		})
	}

	/// Shifts the segment data left to offset zero if writable.
	pub fn shift(&mut self) {
		if let Buf::Block(block) = &mut self.buf {
			if let Some(array) = Rc::get_mut(block) {
				array.copy_within(self.off..self.len, 0);
				self.len -= self.off;
				self.off = 0;
			}
		}
	}

	/// Reads data into `buf`, returning the number elements written.
	pub fn read(&mut self, buf: &mut [T]) -> usize {
		let slice = self.as_slice();
		let count = min(slice.len(), buf.len());
		buf[..count].copy_from_slice(&slice[..count]);
		self.consume_unchecked(count);
		count
	}

	/// Writes data from `buf` if the segment is writable, returning the number of
	/// elements written. This operation will fail if the backing buffer is not a
	/// block, or shared with another segment.
	pub fn write(&mut self, buf: &[T]) -> Option<usize> {
		let slice = self.as_mut_slice()?;
		let count = min(slice.len(), buf.len());
		slice[..count].copy_from_slice(&buf[..count]);
		self.len += count;
		Some(count)
	}

	/// Writes data from `buf`, returning the number of elements written. If the
	/// segment is not writable, data will be copied into a new buffer before the
	/// write operation is completed. If the length of contained data exceeds the
	/// block size, [`N`], the remainder will be returned in a segment wrapped in
	/// `Err`.
	pub fn force_write(&mut self, buf: &[T], pool: &mut dyn MutPool<N, T>) -> Result<usize, Self> {
		match self.fork(pool) {
			Some(rem) => Err(rem),
			None => Ok(
				self.write(buf)
					.expect(
						"internal buffer should be a unique block reference after \
						`make_writable`"
					)
			)
		}

	}

	/// Returns a new segment sharing data within `range` with this segment.
	pub fn share<R: RangeBounds<usize>>(&self, range: R) -> Self {
		let range = slice::range(range, ..self.len());
		let mut seg = self.clone();
		seg.consume_unchecked(range.start);
		seg.  trunc_unchecked(range.len());
		seg
	}

	/// Returns a new segment sharing all its data with this segment. This is the
	/// same as `clone`.
	pub fn share_all(&self) -> Self { self.clone() }
}

impl<'d, const N: usize, T: Element> Seg<'d, N, T> {
	fn new(buf: Buf<'d, N, T>) -> Self {
		let len = match buf {
			Buf::Boxed(ref array) => array.len(),
			Buf::Slice(ref slice) => slice.len(),
			_ => 0
		};

		Self { buf, off: 0, len }
	}

	fn consume_unchecked(&mut self, count: usize) {
		if count + self.off >= self.len {
			self.clear()
		} else if let Buf::Slice(slice) = &mut self.buf {
			*slice = &slice[count..];
		} else {
			self.off += count;
		}
	}

	fn trunc_unchecked(&mut self, count: usize) {
		if count == 0 {
			self.clear()
		} else if let Buf::Slice(slice) = &mut self.buf {
			*slice = &slice[..count];
		} else {
			self.len -= count;
		}
	}

	pub(crate) fn into_block(self) -> Option<Block<N, T>> {
		self.buf.into_writable()
	}
}

impl<'d, const N: usize, T: Element> From<SharedBlock<N, T>> for Seg<'d, N, T> {
	fn from(value: SharedBlock<N, T>) -> Self {
		Self::new(value.into())
	}
}

impl<'d, const N: usize, T: Element> From<Block<N, T>> for Seg<'d, N, T> {
	fn from(value: Block<N, T>) -> Self {
		Rc::new(value).into()
	}
}

impl<'d, const N: usize, T: Element> From<Box<[T; N]>> for Seg<'d, N, T> {
	fn from(value: Box<[T; N]>) -> Self {
		Pin::new(value).into()
	}
}

impl<'d, const N: usize, T: Element> From<Rc<Box<[T]>>> for Seg<'d, N, T> {
	fn from(value: Rc<Box<[T]>>) -> Self {
		Self::new(value.into())
	}
}

impl<'d, const N: usize, T: Element> From<Box<[T]>> for Seg<'d, N, T> {
	fn from(value: Box<[T]>) -> Self {
		Self::new(Rc::new(value).into())
	}
}

impl<'d, const N: usize, T: Element> From<Vec<T>> for Seg<'d, N, T> {
	fn from(value: Vec<T>) -> Self {
		// No reallocation. Wrong. Bad.
		if value.len() == value.capacity() {
			value.into_boxed_slice().into()
		} else {
			// SAFETY: the decomposed Vec's pointer is wrapped in a boxed array of
			// the same length, ensuring its contents will be freed. Its length is
			// used for the resulting segment, so uninitialized data is never read
			// and undefined behavior is averted.
			unsafe {
				// Hack the Vec to reconstruct its buffer without the reallocation
				// done by into_boxed_slice.
				let (ptr, len, cap) = value.into_raw_parts();
				let raw = slice_from_raw_parts_mut(ptr, cap);
				let buf = Box::from_raw(raw).into();
				Self { buf, off: 0, len }
			}
		}
	}
}

impl<'d, const N: usize, T: Element> From<&'d [T]> for Seg<'d, N, T> {
	fn from(value: &'d [T]) -> Self {
		Self::new(value.into())
	}
}

impl<'d, const N: usize> From<&'d str> for Seg<'d, N, u8> {
	fn from(value: &'d str) -> Self {
		value.as_bytes().into()
	}
}

impl<'d, const N: usize> From<String> for Seg<'d, N, u8> {
	fn from(value: String) -> Self {
		value.into_bytes().into()
	}
}

impl<'d, const N: usize> From<Cow<'d, str>> for Seg<'d, N, u8> {
	fn from(value: Cow<'d, str>) -> Self {
		match value {
			Cow::Borrowed(value) => value.into(),
			Cow::Owned   (value) => value.into()
		}
	}
}

#[cfg(feature = "bytes")]
impl<'d, const N: usize> From<&'d bytes::Bytes> for Seg<'d, N, u8> {
	fn from(value: &'d bytes::Bytes) -> Self {
		value.as_ref().into()
	}
}
