// SPDX-License-Identifier: Apache-2.0

use crate::{Buffer, BufferResult, ResultContext, Seg, StreamResult as Result};
use crate::BufferContext::Drain;
use crate::streams::{BufSink, BufSource, Sink};
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
		source.read(self, count).context(Drain)
	}

	fn drain_all(&mut self, source: &mut Buffer<'d, N, impl Pool<N>>) -> BufferResult<usize> {
		source.read_all(self).context(Drain)
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
}

impl<'a, const N: usize> RBuf<Seg<'a, N>> {
	fn write_back(&mut self, data: &mut &[u8], expect: &str) -> usize {
		let mut seg = self.pop_back().expect(expect);
		let written = seg.write(data).expect("back segment should be writable");
		self.push_back(seg);
		*data = &data[written..];
		written
	}
}
