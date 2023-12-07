// SPDX-License-Identifier: Apache-2.0

use crate::{Buffer, BufferResult, ResultContext, Seg, StreamContext, StreamResult as Result};
use crate::BufferContext::Drain;
use crate::streams::{BufSink, BufSource, Sink, Source};
use crate::pool::Pool;
use crate::segment::RBuf;

impl<const N: usize, P: Pool<N>> Sink<N> for Buffer<'_, N, P> {
	fn drain(&mut self, source: &mut Buffer<'_, N, impl Pool<N>>, count: usize) -> BufferResult<usize> {
		source.read(self, count).context(Drain)
	}

	fn drain_all(&mut self, source: &mut Buffer<'_, N, impl Pool<N>>) -> BufferResult<usize> {
		source.read_all(self).context(Drain)
	}
}

impl<'d, const N: usize, P: Pool<N>> BufSink<'d, N> for Buffer<'d, N, P> {
	fn write_all(&mut self, source: &mut impl Source) -> Result<usize> {
		source.read_all(self)
			  .context(StreamContext::Write)
	}

	fn write_from_slice(&mut self, mut value: &'d [u8]) -> Result<usize> {
		let mut count = 0;

		// Write as a slice segment if the length is above the share threshold, but
		// avoid pushing fragmentation beyond the compact threshold.

		let back_lim = self.data.back_limit();
		let frag_len = self.data.fragment_len() + back_lim;

		if frag_len >= self.compact_threshold && back_lim > 0 {
			count = self.data.write_back(
				&mut value,
				"buffer with back_limit > 0 should have writable segment"
			);
		}

		if value.len() >= self.share_threshold {
			self.data.push_back(value.into());
			count += value.len();
			return Ok(count)
		}

		// Write into segments.
		self.reserve(value.len()).context(StreamContext::Write)?;
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
