// SPDX-License-Identifier: Apache-2.0

use std::io;
use std::io::{ErrorKind, IoSlice, Write};
use crate::{Buffer, StreamResult as Result, BufferResult, StreamResult, ResultSetContext, ResultContext};
use crate::BufferContext::{Drain, Fill};
use crate::pattern::{LineTerminator, Pattern};
use crate::pool::Pool;
use crate::segment::SliceRangeIter;
use crate::streams::{BufSource, Source, Utf8Match};
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

		let mut remaining = count;
		let partial_pos = self.data.iter().position(|seg|
			if seg.len() > remaining && remaining > sink.share_threshold {
				true
			} else {
				remaining -= seg.len();
				false
			}
		).unwrap();

		sink.reserve(remaining).set_context(Fill)?;

		sink.data.extend(
			self.data
				.drain(partial_pos)
		);

		if let Some(mut seg) = self.data.pop_front() {
			let mut shared = seg.share(..remaining);
			// Share partial segment with sink
			let partial = if remaining >= sink.share_threshold {
				shared
			} else {
				let mut fresh = sink.data.pop_back().expect(
					"buffer should have writable segments after reserve"
				);
				fresh.write_from(&mut shared)
					 .expect("back segment should be writable");
				fresh
			};

			seg.consume(remaining);
			self.data.push_front(seg);
			sink.data.push_back(partial);
		}

		self.resize().set_context(Fill)?;
		Ok(count)
	}

	fn fill_all(&mut self, sink: &mut Buffer<'d, N, impl Pool<N>>) -> BufferResult<usize> {
		self.resize().set_context(Fill)?;
		let read = self.count();
		// Take the internal ring buffer instead of draining, which should be
		// significantly faster; similar to Buffer::clear.
		sink.data.extend(self.take_buf());
		Ok(read)
	}
}

impl<'d, const N: usize, P: Pool<N>> BufSource<'d, N> for Buffer<'d, N, P> {
	fn request(&mut self, count: usize) -> StreamResult<bool> {
		Ok(self.count() >= count)
	}

	fn read_slice(&mut self, buf: &mut [u8]) -> Result<usize> {
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
		Ok(count)
	}

	fn read_slice_exact(&mut self, buf: &mut [u8]) -> Result<usize> {
		self.require(buf.len())?;
		let count = self.read_slice(buf)?;
		assert_eq!(count, buf.len(), "require should ensure all bytes are available");
		Ok(count)
	}

	fn read_utf8(&mut self, buf: &mut String, mut count: usize) -> Result<usize> {
		count = count.min(self.count());
		buf.reserve(count);
		let read = read_partial_utf8_into(
			self.data.iter_slices_in_range(..count),
			buf
		).context(Read)?;
		Ok(self.skip(read))
	}

	#[inline]
	fn read_utf8_to_end(&mut self, buf: &mut String) -> Result<usize> {
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
			let count = self.read_utf8(buf, range.start)?;
			self.skip(range.len());
			Ok((count, true).into())
		} else {
			self.read_utf8_to_end(buf)
				.map(|count| (count, false).into())
		}
	}

	/// Reads UTF-8 bytes into `buf` until and including the `terminator` pattern,
	/// returning the number of bytes read and whether the pattern was found. If a
	/// decode error occurs, no data is consumed and `buf` will contain the last
	/// valid data.
	fn read_utf8_until_inclusive(&mut self, buf: &mut String, terminator: impl Pattern) -> Result<Utf8Match> {
		if let Some(range) = self.find(terminator) {
			self.read_utf8(buf, range.end)
				.map(|count| (count, true).into())
		} else {
			self.read_utf8_to_end(buf)
				.map(|count| (count, false).into())
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
