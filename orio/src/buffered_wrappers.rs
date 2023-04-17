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

use std::io::Write;
use crate::Buffer;
use crate::pool::Pool;
use crate::streams::{Sink, Source, Stream, Result, BufStream, BufSource, Error, BufSink};
use crate::streams::OperationKind::BufFlush;

pub fn buffer_source<S: Source, P: Pool + Default>(source: S) -> BufferedSource<S, P> {
	BufferedSource {
		buffer: Buffer::default(),
		source,
		closed: false,
	}
}

pub fn buffer_sink<S: Sink, P: Pool + Default>(sink: S) -> BufferedSink<S, P> {
	BufferedSink {
		buffer: Buffer::default(),
		sink,
		closed: false,
	}
}

pub struct BufferedSource<S: Source, P: Pool> {
	buffer: Buffer<P>,
	source: S,
	closed: bool,
}

impl<S: Source, P: Pool> Stream for BufferedSource<S, P> {
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

impl<S: Source, P: Pool> Source for BufferedSource<S, P> {
	fn read(&mut self, buffer: &mut Buffer<impl Pool>, count: usize) -> Result<usize> {
		todo!()
	}
}

impl<S: Source, P: Pool> BufStream for BufferedSource<S, P> {
	type Pool = P;
	fn buf(&mut self) -> &mut Buffer<P> { &mut self.buffer }
}

impl<S: Source, P: Pool> BufSource for BufferedSource<S, P> {
	fn read_all(&mut self, mut sink: &mut impl Sink) -> Result<usize> {
		sink.write_all(self.buf())
			.map_err(Error::with_op_buf_read)
	}
}

impl<S: Source, P: Pool> Drop for BufferedSource<S, P> {
	fn drop(&mut self) {
		let _ = self.close();
	}
}

pub struct BufferedSink<S: Sink, P: Pool> {
	buffer: Buffer<P>,
	sink: S,
	closed: bool,
}

impl<S: Sink, P: Pool> Stream for BufferedSink<S, P> {
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

impl<S: Sink, P: Pool> Sink for BufferedSink<S, P> {
	fn write(&mut self, buffer: &mut Buffer<impl Pool>, count: usize) -> Result<usize> {
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

impl<S: Sink, P: Pool> BufStream for BufferedSink<S, P> {
	type Pool = P;
	fn buf(&mut self) -> &mut Buffer<Self::Pool> { &mut self.buffer }
}

impl<S: Sink, P: Pool> BufSink for BufferedSink<S, P> {
	fn write_all(&mut self, source: &mut impl Source) -> Result<usize> {
		source.read_all(self.buf())
			  .map_err(Error::with_op_buf_write)
	}
}

impl<S: Sink, P: Pool> Drop for BufferedSink<S, P> {
	fn drop(&mut self) {
		let _ = self.close();
	}
}
