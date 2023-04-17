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

use crate::pool::{DefaultPool, Pool};
use crate::segment::Segments;
use crate::streams::{BufSink, BufSource, BufStream, Error, Result, Sink, Source, Stream};

pub struct Buffer<P: Pool = DefaultPool> {
	pool: P,
	segments: Segments,
	closed: bool,
}

impl<P: Pool + Default> Default for Buffer<P> {
	fn default() -> Self { Self::new(P::default()) }
}

impl<P: Pool> Buffer<P> {
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

impl<P: Pool> Drop for Buffer<P> {
	fn drop(&mut self) {
		let _ = self.close();
	}
}

impl<P: Pool> Stream for Buffer<P> {
	fn close(&mut self) -> Result {
		if !self.closed {
			self.closed = true;
			self.clear()
		} else {
			Ok(())
		}
	}
}

impl<P: Pool> Source for Buffer<P> {
	fn read(&mut self, sink: &mut Buffer<impl Pool>, count: usize) -> Result<usize> {
		todo!()
	}

	fn read_all(&mut self, sink: &mut Buffer<impl Pool>) -> Result<usize> {
		todo!()
	}
}

impl<P: Pool> Sink for Buffer<P> {
	fn write(&mut self, source: &mut Buffer<impl Pool>, count: usize) -> Result<usize> {
		todo!()
	}

	fn write_all(&mut self, source: &mut Buffer<impl Pool>) -> Result<usize> {
		todo!()
	}
}

impl<P: Pool> BufStream for Buffer<P> {
	type Pool = P;
	fn buf(&mut self) -> &mut Self { self }
}

impl<P: Pool> BufSource for Buffer<P> {
	fn read_all(&mut self, sink: &mut impl Sink) -> Result<usize> {
		sink.write_all(self)
			.map_err(Error::with_op_buf_read)
	}
}

impl<P: Pool> BufSink for Buffer<P> {
	fn write_all(&mut self, source: &mut impl Source) -> Result<usize> {
		source.read_all(self)
			  .map_err(Error::with_op_buf_write)
	}
}


