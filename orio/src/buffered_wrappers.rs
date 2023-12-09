// SPDX-License-Identifier: Apache-2.0

use crate::{Buffer, BufferError, BufferOptions, BufferResult, ResultContext, SIZE, StreamError, StreamResult};
use crate::BufferContext::{Drain, Fill};
use crate::pool::Pool;
use crate::streams::{Sink, Source, BufStream, BufSource, BufSink, Seekable, SeekOffset, SeekableExt, Stream};
use crate::StreamContext::{Flush, Read, Seek, Write};

pub fn buffer_source<'d, const N: usize, S: Source<'d, N>, P: Pool<N>>(source: S, options: BufferOptions, pool: P) -> BufferedSource<'d, N, S, P> {
	BufferedSource::new(source, options, pool)
}

pub fn buffer_sink<'d, const N: usize, S: Sink<'d, N>, P: Pool<N>>(sink: S, options: BufferOptions, pool: P) -> BufferedSink<'d, N, S, P> {
	BufferedSink::new(sink, options, pool)
}

pub struct BufferedSource<'d, const N: usize, S: Source<'d, N>, P: Pool<N>> {
	buffer: Buffer<'d, N, P>,
	source: S,
	closed: bool,
}

#[inline]
fn max_read_size(buffer_limit: usize, segment_size: usize) -> usize {
	buffer_limit.min(segment_size)
}

#[inline]
fn read_size(requested: usize, buffer_limit: usize, segment_size: usize) -> usize {
	requested.clamp(segment_size, max_read_size(buffer_limit, segment_size))
}

impl<'d, const N: usize, S: Source<'d, N>, P: Pool<N>> BufferedSource<'d, N, S, P> {
	fn new(source: S, options: BufferOptions, pool: P) -> Self {
		Self {
			buffer: Buffer::new(pool, options),
			source,
			closed: false,
		}
	}

	#[inline]
	fn max_request_size(&self) -> usize {
		max_read_size(self.buffer.limit(), N)
	}

	/// Determines the request size for a read of `count` bytes. Requests are at
	/// least one segment in length, and at most the buffer limit if the limit is
	/// more than the segment size. This ensures reads have a minimum size for
	/// better efficiency, while limiting allocation during very large reads.
	#[inline]
	fn request_size(&self, count: usize) -> usize {
		read_size(count, self.buffer.limit(), N)
	}
}

impl<'d, const N: usize, S: Source<'d, N>, P: Pool<N>> Stream<N> for BufferedSource<'d, N, S, P> {
	fn close(&mut self) -> StreamResult {
		if !self.closed {
			self.closed = true;
			let buf_result = self.buffer.close();
			let src_result = self.source.close();
			buf_result?;
			src_result?;
		}
		Ok(())
	}
}

impl<'d, const N: usize, S: Source<'d, N>, P: Pool<N>> Source<'d, N> for BufferedSource<'d, N, S, P> {
	fn is_eos(&self) -> bool {
		self.source.is_eos()
	}

	fn fill(&mut self, sink: &mut Buffer<'d, N, impl Pool<N>>, mut count: usize) -> BufferResult<usize> {
		if self.closed { return Err(BufferError::closed(Fill)) }
		let mut read = self.buffer.fill(sink, count)?;
		count -= read;
		read += self.source.fill(sink, count)?;
		Ok(read)
	}

	fn fill_all(&mut self, sink: &mut Buffer<'d, N, impl Pool<N>>) -> BufferResult<usize> {
		if self.closed { return Err(BufferError::closed(Fill)) }
		let mut count = self.buffer.fill_all(sink)?;
		count +=  self.source.fill_all(sink)?;
		Ok(count)
	}
}

impl<'d, const N: usize, S: Source<'d, N>, P: Pool<N>> BufStream<'d, N> for BufferedSource<'d, N, S, P> {
	type Pool = P;

	fn buf<'b>(&'b self) -> &'b Buffer<'d, N, P> where 'd: 'b { &self.buffer }
	fn buf_mut<'b>(&'b mut self) -> &'b mut Buffer<'d, N, P> where 'd: 'b { &mut self.buffer }
}

impl<'d, const N: usize, S: Source<'d, N>, P: Pool<N>> BufSource<'d, N> for BufferedSource<'d, N, S, P> {
	fn request(&mut self, count: usize) -> StreamResult<bool> {
		if self.closed { return Err(StreamError::closed(Read)) }
		if self.is_eos() { return Ok(false) }

		let Self { source, buffer, .. } = self;

		// No fill necessary
		buffer.request(count)?;
		if buffer.count() >= count {
			return Ok(buffer.count() >= count)
		}

		// Fill buffer to its limit
		source.fill(buffer, buffer.limit())?;
		if buffer.count() >= count {
			return Ok(buffer.count() >= count)
		}

		loop {
			match source.fill(buffer, N) {
				Ok(_) => {
					if buffer.count() >= count {
						break Ok(buffer.count() >= count)
					}
				}
				Err(err) if err.is_eos() => break Ok(buffer.count() >= count),
				err => { err?; }
			}
		}
	}

	fn read(&mut self, sink: &mut impl Sink<'d, N>, count: usize) -> StreamResult<usize> {
		if self.closed { return Err(StreamError::closed(Read)) }

		let mut read = 0;
		while read < count {
			let remaining = count - read;
			read += sink.drain(self.buf_mut(), remaining).context(Read)?;
			self.request(self.request_size(remaining))?;

			if self.available() == 0 {
				break
			}
		}
		Ok(read)
	}

	fn read_all(&mut self, sink: &mut impl Sink<'d, N>) -> StreamResult<usize> {
		if self.closed { return Err(StreamError::closed(Read)) }

		let mut read = 0;
		loop {
			read += sink.drain_all(self.buf_mut()).context(Read)?;
			self.request(self.max_request_size())?;

			if self.available() == 0 {
				break
			}
		}

		Ok(read)
	}
}

impl<'d, S: Source<'d, SIZE> + Seekable, P: Pool<SIZE>> BufferedSource<'d, SIZE, S, P> {
	fn seek_back(&mut self, off: usize) -> StreamResult<usize> {
		let cur_pos = self.seek_pos()?;
		let new_pos = self.source.seek_back(off)?;
		let count = cur_pos - new_pos;

		if count == 0 {
			return Ok(new_pos)
		}

		let mut seek_buf: Buffer = BufferOptions::default()
			.set_compact_threshold(usize::MAX)
			.into();
		self.source
			.fill(&mut seek_buf, count)
			.context(Seek)?;
		seek_buf.drain_all(&mut self.buffer)
				.context(Seek)?;
		self.buffer.swap(&mut seek_buf);
		Ok(new_pos)
	}

	fn seek_forward(&mut self, mut off: usize) -> StreamResult<usize> {
		off -= self.buffer.skip(off)?;
		self.source.seek_forward(off)
	}
}

impl<'d, const N: usize, S: Source<'d, N> + Seekable, P: Pool<N>> Seekable for BufferedSource<'d, N, S, P> {
	fn seek(&mut self, offset: SeekOffset) -> StreamResult<usize> {
		return match offset {
			SeekOffset::Forward(0) |
			SeekOffset::Back   (0) => self.seek_pos(),
			SeekOffset::Forward(off) => self.seek_forward(off),
			SeekOffset::Back   (off) => self.seek_back   (off),
			SeekOffset::FromEnd(_off @ ..=-1) => {
				let len = self.buffer.count();
				let pos = self.buffer.seek(offset)?;

				if pos < len {
					// We didn't seek through the entire buffer, just return the
					// current position.
					self.seek_pos()
				} else {
					// The buffer was exhausted, seek on the source.
					self.source.seek(offset)
				}
			}
			_ => {
				// No clever way to do the rest, just invalidate the buffered data
				// and seek on the source.
				self.buffer.clear()?;
				self.source.seek(offset)
			}
		}
	}

	fn seek_len(&mut self) -> StreamResult<usize> { self.source.seek_len() }

	fn seek_pos(&mut self) -> StreamResult<usize> {
		// Offset the source position back by the buffer length to account for
		// buffering.
		Ok(self.source.seek_pos()? - self.buffer.count())
	}
}

impl<'d, const N: usize, S: Source<'d, N>, P: Pool<N>> Drop for BufferedSource<'d, N, S, P> {
	fn drop(&mut self) {
		let _ = self.close();
	}
}

pub struct BufferedSink<'d, const N: usize, S: Sink<'d, N>, P: Pool<N>> {
	buffer: Buffer<'d, N, P>,
	sink: S,
	closed: bool,
}

impl<'d, const N: usize, S: Sink<'d, N>, P: Pool<N>> BufferedSink<'d, N, S, P> {
	fn new(sink: S, options: BufferOptions, pool: P) -> Self {
		Self {
			buffer: Buffer::new(pool, options),
			sink,
			closed: false,
		}
	}

	#[inline]
	fn max_request_size(&self) -> usize {
		max_read_size(self.buffer.limit(), N)
	}
}

impl<'d, const N: usize, S: Sink<'d, N>, P: Pool<N>> Stream<N> for BufferedSink<'d, N, S, P> {
	fn close(&mut self) -> StreamResult {
		if !self.closed {
			self.closed = true;
			let flush = self.flush();
			let close = self.sink.close();
			let clear = self.buffer.close();
			flush?;
			close?;
			clear
		} else {
			Ok(())
		}
	}
}

impl<'d, const N: usize, S: Sink<'d, N>, P: Pool<N>> Sink<'d, N> for BufferedSink<'d, N, S, P> {
	fn drain(&mut self, source: &mut Buffer<'d, N, impl Pool<N>>, count: usize) -> BufferResult<usize> {
		if self.closed { return Err(BufferError::closed(Drain)) }
		self.flush().context(Drain)?;
		self.sink.drain(source, count)
	}

	fn drain_all(&mut self, source: &mut Buffer<'d, N, impl Pool<N>>) -> BufferResult<usize> {
		if self.closed { return Err(BufferError::closed(Drain)) }
		self.flush().context(Drain)?;
		self.sink.drain_all(source)
	}

	fn flush(&mut self) -> StreamResult {
		if self.closed { return Err(StreamError::closed(Flush)) }

		let Self { sink, buffer, .. } = self;
		// Both of these need a chance to run before returning an error.
		let read = sink.drain_all(buffer);
		let flush = sink.flush();
		read?;
		flush
	}
}

impl<'d, const N: usize, S: Sink<'d, N>, P: Pool<N>> BufStream<'d, N> for BufferedSink<'d, N, S, P> {
	type Pool = P;

	fn buf<'b>(&'b self) -> &'b Buffer<'d, N, P> where 'd: 'b { &self.buffer }
	fn buf_mut<'b>(&'b mut self) -> &'b mut Buffer<'d, N, P> where 'd: 'b { &mut self.buffer }
}

impl<'d, const N: usize, S: Sink<'d, N>, P: Pool<N>> BufSink<'d, N> for BufferedSink<'d, N, S, P> {
	fn write(&mut self, source: &mut impl Source<'d, N>, count: usize) -> StreamResult<usize> {
		if self.closed { return Err(StreamError::closed(Write)) }

		let mut written = 0;
		while written < count {
			let remaining = count - written;
			written += source.fill(self.buf_mut(), remaining).context(Write)?;

			if self.buffer.count() == 0 {
				break
			}

			self.flush().context(Write)?;
		}
		Ok(written)
	}

	fn write_all(&mut self, source: &mut impl Source<'d, N>) -> StreamResult<usize> {
		if self.closed { return Err(StreamError::closed(Write)) }

		let mut written = 0;
		loop {
			written += source.fill_all(self.buf_mut()).context(Write)?;

			if self.buffer.count() == 0 {
				break
			}

			self.flush().context(Write)?;
		}
		Ok(written)
	}
}

impl<'d, const N: usize, S: Sink<'d, N> + Seekable, P: Pool<N>> Seekable for BufferedSink<'d, N, S, P> {
	fn seek(&mut self, offset: SeekOffset) -> StreamResult<usize> {
		// Todo: Is there some less naive approach than flushing then seeking?
		self.flush().context(Seek)?;
		self.sink.seek(offset)
	}

	fn seek_len(&mut self) -> StreamResult<usize> {
		Ok(self.buffer.count() + self.sink.seek_len()?)
	}
}

impl<'d, const N: usize, S: Sink<'d, N>, P: Pool<N>> Drop for BufferedSink<'d, N, S, P> {
	fn drop(&mut self) {
		let _ = self.close();
	}
}
