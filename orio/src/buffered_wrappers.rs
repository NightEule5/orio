// SPDX-License-Identifier: Apache-2.0

use crate::{Buffer, BufferError, BufferOptions, BufferResult, ResultContext, StreamError, StreamResult};
use crate::BufferContext::{Drain, Fill};
use crate::streams::{Sink, Source, BufStream, BufSource, BufSink, Seekable, SeekOffset, SeekableExt, Stream};
use crate::StreamContext::{Flush, Read, Seek, Write};

pub fn buffer_source<'d, S: Source>(source: S, options: BufferOptions) -> BufferedSource<'d, S> {
	BufferedSource::new(source, options)
}

pub fn buffer_sink<'d, S: Sink>(sink: S, options: BufferOptions) -> BufferedSink<'d, S> {
	BufferedSink::new(sink, options)
}

pub struct BufferedSource<'d, S: Source> {
	buffer: Buffer<'d>,
	source: S,
	closed: bool,
}

impl<S: Source> BufferedSource<'_, S> {
	fn new(source: S, options: BufferOptions) -> Self {
		Self {
			buffer: options.into(),
			source,
			closed: false,
		}
	}
}

impl<S: Source> Stream for BufferedSource<'_, S> {
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

impl<S: Source> Source for BufferedSource<'_, S> {
	fn is_eos(&self) -> bool {
		self.source.is_eos()
	}

	fn fill(&mut self, sink: &mut Buffer<'_>, mut count: usize) -> BufferResult<usize> {
		if self.closed { return Err(BufferError::closed(Fill)) }
		let mut read = self.buffer.fill(sink, count)?;
		count -= read;
		read += self.source.fill(sink, count)?;
		Ok(read)
	}

	fn fill_all(&mut self, sink: &mut Buffer<'_>) -> BufferResult<usize> {
		if self.closed { return Err(BufferError::closed(Fill)) }
		let mut count = self.buffer.fill_all(sink)?;
		count +=  self.source.fill_all(sink)?;
		Ok(count)
	}
}

impl<'d, S: Source> BufStream for BufferedSource<'d, S> {
	type Pool = <Buffer<'d> as BufStream>::Pool;

	fn buf(&self) -> &Buffer { &self.buffer }
	fn buf_mut(&mut self) -> &mut Buffer { &mut self.buffer }
}

impl<S: Source> BufSource for BufferedSource<'_, S> {
	fn request(&mut self, count: usize) -> usize {
		if self.closed || self.is_eos() { return 0 }

		let available = self.buffer.request(count);
		if available >= count {
			return available
		}

		self.source
			.fill(self.buf_mut(), count.min(self.buffer.limit()))
			.unwrap_or_default()
	}

	fn read(&mut self, sink: &mut impl Sink, mut count: usize) -> StreamResult<usize> {
		if self.closed { return Err(StreamError::closed(Read)) }

		let mut read = sink.drain(self.buf_mut(), count).context(Read)?;
		count -= read;
		read += self.source.read(sink)?;
		Ok(read)
	}

	fn read_all(&mut self, sink: &mut impl Sink) -> StreamResult<usize> {
		if self.closed { return Err(StreamError::closed(Read)) }
		let mut read = sink.drain_all(self.buf_mut()).context(Read)?;
		read += self.source.read_all(sink)?;
		Ok(read)
	}
}

impl<S: Source + Seekable> BufferedSource<'_, S> {
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
			.read(&mut seek_buf, count)
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

impl<S: Source + Seekable> Seekable for BufferedSource<'_, S> {
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
				self.buffer.skip_all()?;
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

impl<S: Source> Drop for BufferedSource<'_, S> {
	fn drop(&mut self) {
		let _ = self.close();
	}
}

pub struct BufferedSink<'d, S: Sink> {
	buffer: Buffer<'d>,
	sink: S,
	closed: bool,
}

impl<S: Sink> BufferedSink<'_, S> {
	fn new(sink: S, options: BufferOptions) -> Self {
		Self {
			buffer: options.into(),
			sink,
			closed: false,
		}
	}
}

impl<S: Sink> Stream for BufferedSink<'_, S> {
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

impl<S: Sink> Sink for BufferedSink<'_, S> {
	fn drain(&mut self, source: &mut Buffer<'_>, count: usize) -> BufferResult<usize> {
		if self.closed { return Err(BufferError::closed(Drain)) }
		self.flush().context(Drain)?;
		self.sink.drain(source, count)
	}

	fn drain_all(&mut self, source: &mut Buffer<'_>) -> BufferResult<usize> {
		if self.closed { return Err(BufferError::closed(Drain)) }
		self.flush().context(Drain)?;
		self.sink.drain_all(source)
	}

	fn flush(&mut self) -> StreamResult {
		if self.closed { return Err(StreamError::closed(Flush)) }
		// Both of these need a chance to run before returning an error.
		let read = self.sink.drain_all(self.buf_mut());
		let flush = self.sink.flush();
		read?;
		flush
	}
}

impl<'d, S: Sink> BufStream for BufferedSink<'d, S> {
	type Pool = <Buffer<'d> as BufStream>::Pool;

	fn buf(&self) -> &Buffer { &self.buffer }
	fn buf_mut(&mut self) -> &mut Buffer { &mut self.buffer }
}

impl<S: Sink> BufSink<'_> for BufferedSink<'_, S> {
	fn write(&mut self, source: &mut impl Source, mut count: usize) -> StreamResult<usize> {
		if self.closed { return Err(StreamError::closed(Write)) }

		let mut written = source.fill(self.buf_mut(), count).context(Write)?;
		count -= written;
		written += self.sink.write(source)?;
		Ok(written)
	}

	fn write_all(&mut self, source: &mut impl Source) -> StreamResult<usize> {
		if self.closed { return Err(StreamError::closed(Write)) }
		let mut written = source.fill_all(self.buf_mut()).context(Write)?;
		written += self.sink.write_all(source)?;
		Ok(written)
	}
}

impl<S: Sink + Seekable> Seekable for BufferedSink<'_, S> {
	fn seek(&mut self, offset: SeekOffset) -> StreamResult<usize> {
		// Todo: Is there some less naive approach than flushing then seeking?
		self.flush().context(Seek)?;
		self.sink.seek(offset)
	}

	fn seek_len(&mut self) -> StreamResult<usize> {
		Ok(self.buffer.count() + self.sink.seek_len()?)
	}
}

impl<S: Sink> Drop for BufferedSink<'_, S> {
	fn drop(&mut self) {
		let _ = self.close();
	}
}
