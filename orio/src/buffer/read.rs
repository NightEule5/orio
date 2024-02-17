// SPDX-License-Identifier: Apache-2.0

use std::io;
use std::io::{ErrorKind, IoSlice, Write};
use crate::{Buffer, StreamResult as Result, BufferResult, StreamResult, ResultSetContext, ResultContext};
use crate::BufferContext::{Drain, Fill};
use crate::pattern::{LineTerminator, Pattern};
use crate::pool::Pool;
use crate::segment::SliceRangeIter;
use crate::streams::{BufSink, BufSource, Source, Utf8Match};
use crate::StreamContext::Read;
use super::read_partial_utf8_into;

impl<'d, const N: usize, P: Pool<N>> Source<'d, N> for Buffer<'d, N, P> {
	fn is_eos(&self) -> bool {
		self.is_empty()
	}

	fn fill(&mut self, sink: &mut Buffer<'d, N, impl Pool<N>>, count: usize) -> BufferResult<usize> {
		if count == 0 { return Ok(0) }

		// Use faster fill_all.
		if count >= self.count() {
			return self.fill_all(sink)
		}

		let mut moved = 0;
		let full_count = self.data.iter().position(|seg| {
			let remaining = count - moved;
			if seg.len() > remaining {
				true
			} else {
				moved += seg.len();
				false
			}
		}).unwrap();

		sink.data.extend(
			self.data.drain(full_count)
		);

		if moved < count {
			let remaining = count - moved;
			let mut front = self.data
							   .front_mut()
							   .expect("should have one remaining segment");
			if remaining >= sink.share_threshold {
				let shared = front.share(..remaining);
				sink.data.push_back(shared);
			} else {
				let (a, b) = front.as_slices_in_range(..remaining);
				sink.write_from_slice(a).context(Fill)?;
				sink.write_from_slice(b).context(Fill)?;
			}

			front.consume(remaining);
		}

		self.resize().set_context(Fill)?;
		Ok(count)
	}

	fn fill_all(&mut self, sink: &mut Buffer<'d, N, impl Pool<N>>) -> BufferResult<usize> {
		self.resize().set_context(Fill)?;
		let count = self.count();
		if count == 0 { return Ok(0) }

		if self.data.len() == 1 {
			let seg = self.data.pop_front().unwrap();
			let len = seg.len();
			sink.data.push_back(seg);
			Ok(len)
		} else {
			// Take the internal ring buffer instead of draining, which should be
			// significantly faster; similar to Buffer::clear.
			sink.data.extend(self.take_buf());
			Ok(count)
		}
	}
}

impl<'d, const N: usize, P: Pool<N>> BufSource<'d, N> for Buffer<'d, N, P> {
	fn request(&mut self, count: usize) -> StreamResult<bool> {
		Ok(self.count() >= count)
	}

	fn read_slice<'s>(&mut self, buf: &'s mut [u8]) -> Result<&'s [u8]> {
		let mut count = 0;
		let mut empty_len = 0;
		for seg in self.data.iter_mut() {
			if buf.len() - count == 0 {
				break
			}

			count += seg.read(&mut buf[count..]);
			if seg.is_empty() {
				empty_len += 1;
			} else {
				break
			}
		}
		self.data.consume(count);
		self.data.rotate_back(empty_len);
		Ok(&mut buf[..count])
	}

	fn read_slice_exact<'s>(&mut self, buf: &'s mut [u8]) -> Result<&'s [u8]> {
		let len = buf.len();
		self.require(len)?;
		let slice = self.read_slice(buf)?;
		assert_eq!(slice.len(), len, "require should ensure all bytes are available");
		Ok(slice)
	}

	fn read_utf8<'s>(&mut self, buf: &'s mut String, mut count: usize) -> Result<&'s str> {
		let len = buf.len();
		count = count.min(self.count());
		buf.reserve(count);
		let read = read_partial_utf8_into(
			self.data.iter_slices_in_range(..count),
			buf
		).context(Read)?;
		self.skip(read);
		Ok(&buf[len..])
	}

	#[inline]
	fn read_utf8_to_end<'s>(&mut self, buf: &'s mut String) -> Result<&'s str> {
		self.read_utf8(buf, self.count())
	}

	#[inline]
	fn read_utf8_line(&mut self, buf: &mut String) -> Result<Utf8Match> {
		self.read_utf8_until(buf, LineTerminator)
	}

	#[inline]
	fn read_utf8_line_inclusive(&mut self, buf: &mut String) -> Result<Utf8Match> {
		self.read_utf8_until_inclusive(buf, LineTerminator)
	}

	/// Reads UTF-8 bytes into `buf` until the `terminator` pattern, returning the
	/// number of bytes read and whether the pattern was found. If a decode error
	/// occurs, no data is consumed and `buf` will contain the last valid data.
	fn read_utf8_until(&mut self, buf: &mut String, terminator: impl Pattern) -> Result<Utf8Match> {
		if let Some(range) = self.find(terminator) {
			let count = self.read_utf8(buf, range.start)?.len();
			self.skip(range.len());
			Ok((count, true).into())
		} else {
			self.read_utf8_to_end(buf)
				.map(|str| (str.len(), false).into())
		}
	}

	/// Reads UTF-8 bytes into `buf` until and including the `terminator` pattern,
	/// returning the number of bytes read and whether the pattern was found. If a
	/// decode error occurs, no data is consumed and `buf` will contain the last
	/// valid data.
	fn read_utf8_until_inclusive(&mut self, buf: &mut String, terminator: impl Pattern) -> Result<Utf8Match> {
		if let Some(range) = self.find(terminator) {
			self.read_utf8(buf, range.end)
				.map(|str| (str.len(), true).into())
		} else {
			self.read_utf8_to_end(buf)
				.map(|str| (str.len(), false).into())
		}
	}
}

impl<'a: 'b, 'b, const N: usize> SliceRangeIter<'a, 'b, N> {
	fn collect_io_slices(self) -> Vec<IoSlice<'b>> {
		let mut vec: Vec<_> = self.map(IoSlice::new).collect();
		vec.retain(|s| !s.is_empty());
		vec
	}
}

impl<'a, const N: usize, P: Pool<N>> Buffer<'a, N, P> {
	pub(crate) fn drain_into_writer(
		&mut self,
		writer: &mut impl Write,
		mut count: usize,
		allow_vectored: bool,
	) -> BufferResult<usize> {
		count = count.min(self.count());
		if count == 0 {
			return Ok(0)
		}

		let slices = self.data.iter_slices_in_range(..count);
		let result = if allow_vectored && writer.is_write_vectored() {
			count = writer.write_vectored(&slices.collect_io_slices()).context(Drain)?;
			Ok(())
		} else {
			let mut written = 0;
			let result: io::Result<()> = try {
				'data: for mut data in slices.filter(|s| !s.is_empty()) {
					while !data.is_empty() {
						written += match writer.write(data) {
							Ok(0) => break 'data,
							Ok(written) => {
								data = &data[written..];
								written
							}
							Err(err) if err.kind() == ErrorKind::Interrupted => continue,
							error => error?
						};
					}
				}
			};
			count = written;
			result.context(Drain)
		};
		count = self.skip(count);
		result?;
		Ok(count)
	}
}
