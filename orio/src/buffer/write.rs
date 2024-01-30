// SPDX-License-Identifier: Apache-2.0

use std::collections::vec_deque;
use std::io;
use std::io::{BorrowedBuf, ErrorKind, IoSliceMut, Read};
use std::iter::FilterMap;
use std::mem::MaybeUninit;
use std::ops::RangeTo;
use crate::{Buffer, BufferResult, ResultContext, Seg, StreamResult as Result};
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
	/// a version of [`write_from_slice`] optimized for large slices, with the
	/// caveat that `value` **must** outlive the buffer.
	///
	/// [`write_from_slice`]: Buffer::write_from_slice
	pub fn push_slice(&mut self, value: &'d [u8]) {
		// If the slice length is below the borrow threshold, try writing the slice
		// before using borrowing as a fallback.
		if value.len() >= self.borrow_threshold ||
			self.write_from_slice(value).is_err() {
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

	fn write_from_slice(&mut self, mut value: &[u8]) -> Result<usize> {
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
