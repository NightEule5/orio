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

use crate::Buffer;
use crate::pool::Pool;
use crate::streams::{Sink, Source, Stream, Result, BufStream, BufSource, Error, OperationKind, BufSink};
use crate::streams::OperationKind::BufFlush;

pub fn buffer_source<const N: usize, S: Source, P: Pool<N> + Default>(source: S) -> BufferedSource<N, S, P> {
	BufferedSource {
		buffer: Buffer::default(),
		source,
		closed: false,
	}
}

pub fn buffer_sink<const N: usize, S: Sink, P: Pool<N> + Default>(sink: S) -> BufferedSink<N, S, P> {
	BufferedSink {
		buffer: Buffer::default(),
		sink,
		closed: false,
	}
}

pub struct BufferedSource<const N: usize, S: Source, P: Pool<N>> {
	buffer: Buffer<N, P>,
	source: S,
	closed: bool,
}

impl<const N: usize, S: Source, P: Pool<N>> Stream for BufferedSource<N, S, P> {
	fn close(&mut self) -> Result {
		if !self.closed {
			self.closed = true;
			let buf_close = self.buffer.close();
			let src_close = self.source.close();
			buf_close?;
			src_close
		} else {
			Ok(())
		}
	}
}

impl<const N: usize, S: Source, P: Pool<N>> Source for BufferedSource<N, S, P> {
	fn read<const X: usize>(&mut self, buffer: &mut Buffer<X, impl Pool<X>>, count: usize) -> Result<usize> {
		todo!()
	}
}

impl<const N: usize, S: Source, P: Pool<N>> BufStream<N> for BufferedSource<N, S, P> {
	type Pool = P;
	fn buf(&mut self) -> &mut Buffer<N, P> { &mut self.buffer }
}

impl<const N: usize, S: Source, P: Pool<N>> BufSource<N> for BufferedSource<N, S, P> {
	fn read_all(&mut self, mut sink: &mut impl Sink) -> Result<usize> {
		sink.write_all(self.buf())
			.map_err(Error::with_op_buf_read)
	}
}

impl<const N: usize, S: Source, P: Pool<N>> Drop for BufferedSource<N, S, P> {
	fn drop(&mut self) {
		let _ = self.close();
	}
}

pub struct BufferedSink<const N: usize, S: Sink, P: Pool<N>> {
	buffer: Buffer<N, P>,
	sink: S,
	closed: bool,
}

impl<const N: usize, S: Sink, P: Pool<N>> Stream for BufferedSink<N, S, P> {
	fn close(&mut self) -> Result {
		if !self.closed {
			self.closed = true;
			let flush = self.flush();
			let clear = self.buffer.close();
			flush?;
			clear
		} else {
			Ok(())
		}
	}
}

impl<const N: usize, S: Sink, P: Pool<N>> Sink for BufferedSink<N, S, P> {
	fn write<const B: usize>(&mut self, buffer: &mut Buffer<B, impl Pool<B>>, count: usize) -> Result<usize> {
		todo!()
	}

	fn flush(&mut self) -> Result {
		if !self.closed {
			// Both of these need a chance to run before returning an error.
			let read = self.sink
						   .write_all(&mut self.buffer)
						   .map_err(Error::with_op_buf_flush);
			let flush = self.sink
							.flush()
							.map_err(Error::with_op_buf_flush);
			read?;
			flush?;
			Ok(())
		} else {
			return Err(Error::closed(BufFlush))
		}
	}
}

impl<const N: usize, S: Sink, P: Pool<N>> BufStream<N> for BufferedSink<N, S, P> {
	type Pool = P;
	fn buf(&mut self) -> &mut Buffer<N, Self::Pool> { &mut self.buffer }
}

impl<const N: usize, S: Sink, P: Pool<N>> BufSink<N> for BufferedSink<N, S, P> {
	fn write_all(&mut self, source: &mut impl Source) -> Result<usize> {
		source.read_all(self.buf())
			  .map_err(Error::with_op_buf_write)
	}
}

impl<const N: usize, S: Sink, P: Pool<N>> Drop for BufferedSink<N, S, P> {
	fn drop(&mut self) {
		let _ = self.close();
	}
}
