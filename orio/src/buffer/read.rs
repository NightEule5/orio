// SPDX-License-Identifier: Apache-2.0

use std::str::pattern::Pattern;
use crate::{Buffer, StreamResult as Result, ResultContext, BufferResult, StreamResult, ResultSetContext};
use super::partial_utf8::*;
use crate::BufferContext::Fill;
use crate::pool::Pool;
use crate::streams::{BufSource, Source, Utf8Match};
use crate::StreamContext::Read;

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

	fn read_slice(&mut self, mut buf: &mut [u8]) -> Result<usize> {
		let mut count = 0;
		while self.is_not_empty() && !buf.is_empty() {
			let Some(mut seg) = self.data.pop_front() else { break };
			let read = seg.read(buf);
			buf = &mut buf[read..];
			count += read;
			self.data.push_front(seg);
		}
		Ok(count)
	}

	fn read_slice_exact(&mut self, buf: &mut [u8]) -> Result<usize> {
		self.require(buf.len())?;
		let count = self.read_slice(buf)?;
		assert_eq!(count, buf.len(), "require should ensure all bytes are available");
		Ok(count)
	}

	fn read_utf8_count(&mut self, buf: &mut String, count: usize) -> Result<usize> {
		let mut read = 0;
		let ref mut partial_char = PartialChar::default();
		while read < count {
			let remaining = count - read;
			let Some(seg) = self.data.pop_front() else { break };
			let (a, b) = seg.as_slices_in_range(..remaining.min(seg.len()));
			read_partial_utf8_into(a, buf, partial_char, &mut read).context(Read)?;
			read_partial_utf8_into(b, buf, partial_char, &mut read).context(Read)?;
		}

		self.skip(read)?;
		Ok(read)
	}

	fn read_utf8_to_end(&mut self, buf: &mut String) -> Result<usize> {
		self.read_utf8_count(buf, self.count())
	}

	fn read_utf8_line(&mut self, buf: &mut String) -> Result<Utf8Match> {
		self.read_utf8_line_inclusive(buf).map(|mut um| {
			if um.found {
				let nl_len = if buf.ends_with("\r\n") {
					2
				} else {
					1
				};
				buf.truncate(buf.len() - nl_len);
				um.read_count -= nl_len;
			}
			um
		})
	}

	fn read_utf8_line_inclusive(&mut self, buf: &mut String) -> Result<Utf8Match> {
		if let Some(pos) = self.find_utf8_char('\n') {
			self.read_utf8_count(buf, pos + 1)
				.map(|count| (count, true).into())
		} else {
			self.read_utf8_to_end(buf)
				.map(|count| (count, false).into())
		}
	}

	fn read_utf8_until<'p>(&mut self, _buf: &mut String, _terminator: impl Pattern<'p>) -> Result<Utf8Match> {
		todo!()
	}

	fn read_utf8_until_inclusive<'p>(&mut self, _buf: &mut String, _terminator: impl Pattern<'p>) -> Result<Utf8Match> {
		todo!()
	}
}
