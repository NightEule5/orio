// SPDX-License-Identifier: Apache-2.0

use std::collections::vec_deque;
use std::io;
use std::io::{BorrowedBuf, ErrorKind, IoSliceMut, Read};
use std::iter::FilterMap;
use std::mem::MaybeUninit;
use std::ops::RangeTo;
use num_traits::PrimInt;
use crate::{Buffer, BufferResult, ResultContext, Seg, StreamResult as Result};
use crate::buffer::index_out_of_bounds;
use crate::BufferContext::{Drain, Fill};
use crate::streams::{BufSink, Sink, Source};
use crate::pool::Pool;
use crate::segment::RBuf;
use crate::StreamContext::Write;

impl<'d, const N: usize, P: Pool<N>> Buffer<'d, N, P> {
	/// Pushes a string reference to the buffer without copying its data. This is
	/// a version of [`write_utf8`] optimized for large strings, with the caveat
	/// that `value` **must** outlive the buffer.
	///
	/// [`write_utf8`]: Buffer::write_utf8
	pub fn push_utf8(&mut self, value: &'d str) {
		self.push_slice(value.as_bytes());
	}

	/// Pushes a slice reference to the buffer without copying its data. This is
	/// a version of [`write_slice`] optimized for large slices, with the caveat
	/// that `value` **must** outlive the buffer.
	///
	/// [`write_slice`]: Buffer::write_slice
	pub fn push_slice(&mut self, value: &'d [u8]) {
		// If the slice length is below the borrow threshold, try writing the slice
		// before using borrowing as a fallback.
		if value.len() >= self.borrow_threshold ||
			self.write_slice(value).is_err() {
			self.push_segment(Seg::from_slice(value));
		}
	}

	/// Pushes a segment to the buffer.
	pub fn push_segment(&mut self, value: Seg<'d, N>) {
		self.data.push_back(value);
	}
}

impl<'d, const N: usize, P: Pool<N>> Sink<'d, N> for Buffer<'d, N, P> {
	fn drain(&mut self, source: &mut Buffer<'d, N, impl Pool<N>>, count: usize) -> BufferResult<usize> {
		source.fill(self, count).context(Drain)
	}

	fn drain_all(&mut self, source: &mut Buffer<'d, N, impl Pool<N>>) -> BufferResult<usize> {
		source.fill_all(self).context(Drain)
	}
}

impl<'d, const N: usize, P: Pool<N>> BufSink<'d, N> for Buffer<'d, N, P> {
	fn drain_all_buffered(&mut self) -> BufferResult {
		Ok(())
	}

	fn drain_buffered(&mut self) -> BufferResult {
		Ok(())
	}

	fn write_slice(&mut self, mut value: &[u8]) -> Result<usize> {
		let mut count = 0;
		self.reserve(value.len()).context(Write)?;
		while !value.is_empty() {
			count += self.data.write_back(
				&mut value,
				"buffer should have writable segments after reserve"
			);
		}
		Ok(count)
	}

	fn write_u8(&mut self, value: u8) -> Result {
		self.reserve(1).context(Write)?;
		let mut seg = self.data.back_mut().expect(
			"buffer should have writable segments after reserve"
		);
		seg.push(value).expect("back segment should be writable");
		Ok(())
	}
}

impl<const N: usize, P: Pool<N>> Buffer<'_, N, P> {
	/// Writes bytes from a slice at position `pos`, replacing existing bytes.
	///
	/// # Panics
	///
	/// Panics if any of the segments containing the position are shared, or if
	/// the position is out of bounds.
	pub fn write_slice_at(&mut self, mut pos: usize, buf: &[u8]) -> Result<usize> {
		if pos > self.count() {
			index_out_of_bounds(pos, self.count())
		} else if pos == self.count() {
			return self.write_slice(buf)
		}

		let count = self.count() - pos;
		let interior_count = buf.len().min(count);
		let mut interior = &buf[..interior_count];
		let exterior = &buf[interior_count..];

		for seg in self.data.iter_mut() {
			if seg.len() <= pos {
				pos -= seg.len();
			} else {
				let len = buf.len().min(seg.len());
				seg.copy_from(pos, &interior[..len])
					.expect("segment should be writable");
				interior = &interior[len..];
			}
		}

		let count = interior.len() + self.write_slice(exterior)?;
		Ok(count)
	}

	/// Writes a [`u8`] at position `pos`, replacing the existing value at that
	/// position.
	///
	/// # Panics
	///
	/// Panics if the segment containing the position is shared, or if the position
	/// is out of bounds.
	pub fn write_u8_at(&mut self, pos: usize, value: u8) -> Result {
		if pos > self.count() {
			index_out_of_bounds(pos, self.count())
		} else if pos == self.count() {
			self.write_u8(value)
		} else {
			self[pos] = value;
			Ok(())
		}
	}

	/// Writes an [`i8`] at position `pos`, replacing the existing value at that
	/// position.
	///
	/// # Panics
	///
	/// Panics if the segment containing the position is shared, or if the position
	/// is out of bounds.
	#[inline]
	pub fn write_i8_at(&mut self, pos: usize, value: i8) -> Result {
		self.write_u8_at(pos, value as u8)
	}

	/// Writes a big-endian [`u16`] at position `pos`, replacing the existing value
	/// at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_u16_at(&mut self, pos: usize, value: u16) -> Result {
		self.write_int_at(pos, value)
	}

	/// Writes a little-endian [`u16`] at position `pos`, replacing the existing
	/// value at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_u16_le_at(&mut self, pos: usize, value: u16) -> Result {
		self.write_int_le_at(pos, value)
	}

	/// Writes a big-endian [`i16`] at position `pos`, replacing the existing value
	/// at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_i16_at(&mut self, pos: usize, value: i16) -> Result {
		self.write_int_at(pos, value)
	}

	/// Writes a little-endian [`i16`] at position `pos`, replacing the existing
	/// value at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_i16_le_at(&mut self, pos: usize, value: i16) -> Result {
		self.write_int_le_at(pos, value)
	}

	/// Writes a big-endian [`u32`] at position `pos`, replacing the existing value
	/// at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_u32_at(&mut self, pos: usize, value: u32) -> Result {
		self.write_int_at(pos, value)
	}

	/// Writes a little-endian [`u32`] at position `pos`, replacing the existing
	/// value at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_u32_le_at(&mut self, pos: usize, value: u32) -> Result {
		self.write_int_le_at(pos, value)
	}

	/// Writes a big-endian [`i32`] at position `pos`, replacing the existing value
	/// at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_i32_at(&mut self, pos: usize, value: i32) -> Result {
		self.write_int_at(pos, value)
	}

	/// Writes a little-endian [`i32`] at position `pos`, replacing the existing
	/// value at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_i32_le_at(&mut self, pos: usize, value: i32) -> Result {
		self.write_int_le_at(pos, value)
	}

	/// Writes a big-endian [`u64`] at position `pos`, replacing the existing value
	/// at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_u64_at(&mut self, pos: usize, value: u64) -> Result {
		self.write_int_at(pos, value)
	}

	/// Writes a little-endian [`u64`] at position `pos`, replacing the existing
	/// value at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_u64_le_at(&mut self, pos: usize, value: u64) -> Result {
		self.write_int_le_at(pos, value)
	}

	/// Writes a big-endian [`i64`] at position `pos`, replacing the existing value
	/// at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_i64_at(&mut self, pos: usize, value: i64) -> Result {
		self.write_int_at(pos, value)
	}

	/// Writes a little-endian [`i64`] at position `pos`, replacing the existing
	/// value at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_i64_le_at(&mut self, pos: usize, value: i64) -> Result {
		self.write_int_le_at(pos, value)
	}

	/// Writes a big-endian [`usize`] at position `pos`, replacing the existing value
	/// at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_usize_at(&mut self, pos: usize, value: usize) -> Result {
		self.write_u64_at(pos, value as u64)
	}

	/// Writes a little-endian [`usize`] at position `pos`, replacing the existing
	/// value at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_usize_le_at(&mut self, pos: usize, value: usize) -> Result {
		self.write_u64_le_at(pos, value as u64)
	}

	/// Writes a big-endian [`isize`] at position `pos`, replacing the existing value
	/// at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_isize_at(&mut self, pos: usize, value: isize) -> Result {
		self.write_i64_at(pos, value as i64)
	}

	/// Writes a little-endian [`isize`] at position `pos`, replacing the existing
	/// value at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_isize_le_at(&mut self, pos: usize, value: isize) -> Result {
		self.write_i64_le_at(pos, value as i64)
	}

	/// Writes a big-endian [`u128`] at position `pos`, replacing the existing value
	/// at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_u128_at(&mut self, pos: usize, value: u128) -> Result {
		self.write_int_at(pos, value)
	}

	/// Writes a little-endian [`u128`] at position `pos`, replacing the existing
	/// value at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_u128_le_at(&mut self, pos: usize, value: u128) -> Result {
		self.write_int_le_at(pos, value)
	}

	/// Writes a big-endian [`i128`] at position `pos`, replacing the existing value
	/// at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_i128_at(&mut self, pos: usize, value: i128) -> Result {
		self.write_int_at(pos, value)
	}

	/// Writes a little-endian [`i128`] at position `pos`, replacing the existing
	/// value at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_i128_le_at(&mut self, pos: usize, value: i128) -> Result {
		self.write_int_le_at(pos, value)
	}

	/// Writes a big-endian integer at position `pos`, replacing the existing value
	/// at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_int_at<T: PrimInt + bytemuck::Pod>(&mut self, pos: usize, value: T) -> Result {
		self.write_pod_at(pos, value.to_be())
	}

	/// Writes a little-endian integer at position `pos`, replacing the existing
	/// value at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_int_le_at<T: PrimInt + bytemuck::Pod>(&mut self, pos: usize, value: T) -> Result {
		self.write_pod_at(pos, value.to_le())
	}

	/// Writes an arbitrary [`Pod`] data type at position `pos`, replacing the
	/// existing value at that position.
	///
	/// # Panics
	///
	/// Panics if the segments containing the position are shared, or if the
	/// position is out of bounds.
	#[inline]
	pub fn write_pod_at<T: bytemuck::Pod>(&mut self, pos: usize, value: T) -> Result {
		self.write_slice_at(pos, bytemuck::bytes_of(&value))?;
		Ok(())
	}
}

/// Iterates over writable segments in a buffer, returning mutable slices of their
/// spare capacity.
struct SpareCapacityIter<'a: 'b, 'b, const N: usize> {
	count: usize,
	seg_iter: vec_deque::IterMut<'b, Seg<'a, N>>,
	last_slice: Option<&'b mut [MaybeUninit<u8>]>,
}

impl<'a: 'b, 'b, const N: usize> Iterator for SpareCapacityIter<'a, 'b, N> {
	type Item = &'b mut [MaybeUninit<u8>];

	fn next(&mut self) -> Option<Self::Item> {
		let mut slice = self.last_slice.take().or_else(|| {
			if self.count == 0 {
				return None
			}

			let (a, b) = self.seg_iter.find(|seg|
				!seg.is_full()
			)?.spare_capacity_mut();
			self.last_slice = Some(b).filter(|b| !b.is_empty());
			Some(a)
		})?;

		if slice.len() < self.count {
			self.count -= slice.len();
		} else {
			slice = &mut slice[..self.count];
			self.count = 0;
		}
		Some(slice)
	}
}

impl<'a: 'b, 'b, const N: usize> SpareCapacityIter<'a, 'b, N> {
	fn collect_io_slices(self) -> Vec<IoSliceMut<'b>> {
		let mut vec = Vec::with_capacity(self.seg_iter.len() * 2);
		vec.extend(
			self.map(|slice|
				IoSliceMut::new(unsafe {
					// Safety: IO slices are only written to, never read.
					MaybeUninit::slice_assume_init_mut(slice)
				})
			)
		);
		vec
	}

	fn map_into_bufs(self) -> FilterMap<Self, fn(&'b mut [MaybeUninit<u8>]) -> Option<BorrowedBuf<'b>>> {
		self.filter_map(|b| (!b.is_empty()).then_some(b.into()))
	}
}

impl<'a, const N: usize, P: Pool<N>> Buffer<'a, N, P> {
	fn spare_capacity(
		&mut self,
		RangeTo { end }: RangeTo<usize>
	) -> SpareCapacityIter<'a, '_, N> {
		SpareCapacityIter {
			count: end,
			seg_iter: self.data.iter_all_writable(),
			last_slice: None,
		}
	}

	/// Fills the buffer by reading up to `count` bytes from a `reader`, stopping
	/// when no bytes are read. May optionally use [`Read::read_vectored`] if the
	/// reader supports it, currently to read into spare capacity.
	pub(crate) fn fill_from_reader(
		&mut self,
		reader: &mut impl Read,
		count: usize,
		allow_vectored: bool,
	) -> BufferResult<usize> {
		if count == 0 {
			return Ok(0)
		}

		let mut read = 0;
		if self.capacity() > 0 {
			read = self.fill_spare_from_reader(reader, count, allow_vectored)?;
			if read >= count || read == 0 {
				return Ok(read)
			}
		}

		// Read in block-sized chunks until count is reached, an error occurs, or
		// the reader stops reading any more bytes.
		let mut cur_read;
		while read < count {
			cur_read = 0;
			let remaining = count - read;
			if self.reserve(remaining.min(N)).is_err() {
				break
			}

			let mut seg = self.data.back_mut().unwrap();
			let (mut slice, _) = seg.spare_capacity_mut();
			let len = remaining.min(slice.len());
			slice = &mut slice[..len];
			let result = read_into_buf(reader, slice.into(), &mut cur_read);
			read += cur_read;
			unsafe {
				seg.inc_len(cur_read);
			}
			result?;

			if cur_read == 0 {
				break
			}
		}
		Ok(read)
	}

	pub(crate) fn fill_spare_from_reader(
		&mut self,
		reader: &mut impl Read,
		count: usize,
		allow_vectored: bool,
	) -> BufferResult<usize> {
		let spare = self.spare_capacity(..count);
		let mut read = 0;
		let result = if allow_vectored && reader.is_read_vectored() {
			let mut spare = spare.collect_io_slices();
			// Todo: benchmark to determine whether the overhead of allocating a
			//  vector outweighs the speedup of vectored reads.
			reader.read_vectored(&mut spare)
				  .map(|cur_read| read += cur_read)
		} else {
			try {
				for buf in spare.map_into_bufs() {
					read_into_buf(reader, buf, &mut read)?
				}
			}
		};

		unsafe {
			self.data.grow(read);
		}
		result.context(Fill)?;
		Ok(read)
	}
}

fn read_into_buf(reader: &mut impl Read, mut buf: BorrowedBuf, count: &mut usize) -> io::Result<()> {
	let mut written;
	let result = try {
		while buf.len() < buf.capacity() {
			let mut cursor = buf.unfilled();
			written = cursor.written();
			match reader.read_buf(cursor.reborrow()) {
				Ok(_) => {
					*count += cursor.written();
					if cursor.written() == written {
						// No more bytes read.
						break
					}
				}
				Err(e) if e.kind() == ErrorKind::Interrupted => { }
				error => {
					*count += cursor.written();
					error?
				}
			};
		}
	};
	result
}

impl<'a, const N: usize> RBuf<Seg<'a, N>> {
	fn write_back(&mut self, data: &mut &[u8], expect: &str) -> usize {
		let mut seg = self.back_mut().expect(expect);
		let written = seg.write(data).expect("back segment should be writable");
		*data = &data[written..];
		written
	}
}
