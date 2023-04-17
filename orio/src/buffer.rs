// Copyright 2023 Strixpyrr
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::DEFAULT_SEGMENT_SIZE;
use crate::pool::{DefaultPool, Pool};
use crate::segment::Segments;
use crate::streams::{BufSink, BufSource, BufStream, Error, Result, Sink, Source, Stream};

pub struct Buffer<const N: usize = DEFAULT_SEGMENT_SIZE, P: Pool<N> = DefaultPool<N>> {
	pool: P,
	segments: Segments<N>,
	closed: bool,
}

impl<const N: usize, P: Pool<N> + Default> Default for Buffer<N, P> {
	fn default() -> Self { Self::new(P::default()) }
}

impl<const N: usize, P: Pool<N>> Buffer<N, P> {
	pub fn new(pool: P) -> Self {
		Self {
			pool,
			segments: Segments::default(),
			closed: false,
		}
	}

	pub fn count(&self) -> usize {
		self.segments.count()
	}

	pub fn clear(&mut self) -> Result {
		if !self.closed {
			self.segments
				.clear(&mut self.pool)
				.map_err(Error::with_op_buf_clear)
		} else {
			Ok(())
		}
	}
}

impl<const N: usize, P: Pool<N>> Drop for Buffer<N, P> {
	fn drop(&mut self) {
		let _ = self.close();
	}
}

impl<const N: usize, P: Pool<N>> Stream for Buffer<N, P> {
	fn close(&mut self) -> Result {
		if !self.closed {
			self.closed = true;
			self.clear()
		} else {
			Ok(())
		}
	}
}

impl<const N: usize, P: Pool<N>> Source for Buffer<N, P> {
	fn read<const X: usize>(&mut self, sink: &mut Buffer<X, impl Pool<X>>, count: usize) -> Result<usize> {
		todo!()
	}

	fn read_all<const X: usize>(&mut self, sink: &mut Buffer<X, impl Pool<X>>) -> Result<usize> {
		todo!()
	}
}

impl<const N: usize, P: Pool<N>> Sink for Buffer<N, P> {
	fn write<const X: usize>(&mut self, source: &mut Buffer<X, impl Pool<X>>, count: usize) -> Result<usize> {
		todo!()
	}

	fn write_all<const X: usize>(&mut self, source: &mut Buffer<X, impl Pool<X>>) -> Result<usize> {
		todo!()
	}
}

impl<const N: usize, P: Pool<N>> BufStream<N> for Buffer<N, P> {
	type Pool = P;
	fn buf(&mut self) -> &mut Self { self }
}

impl<const N: usize, P: Pool<N>> BufSource<N> for Buffer<N, P> {
	fn read_all(&mut self, sink: &mut impl Sink) -> Result<usize> {
		sink.write_all(self)
			.map_err(Error::with_op_buf_read)
	}
}

impl<const N: usize, P: Pool<N>> BufSink<N> for Buffer<N, P> {
	fn write_all(&mut self, source: &mut impl Source) -> Result<usize> {
		source.read_all(self)
			  .map_err(Error::with_op_buf_write)
	}
}


