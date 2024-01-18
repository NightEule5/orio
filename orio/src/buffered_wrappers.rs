// SPDX-License-Identifier: Apache-2.0

use crate::{Buffer, BufferResult, ResultContext, SIZE, StreamResult};
use crate::BufferContext::{Drain, Fill};
use crate::pool::Pool;
use crate::streams::{Sink, Source, BufStream, BufSource, BufSink, Seekable, SeekOffset, Stream, SeekableExt};
use crate::StreamContext::{Flush, Read, Seek, Write};

pub struct BufferedSource<'d, S: Source<'d, SIZE>, P: Pool<SIZE>> {
	buffer: Buffer<'d, SIZE, P>,
	source: Option<S>,
	closed: bool,
	eos: bool,
}

#[inline]
fn max_read_size(buffer_limit: usize, segment_size: usize) -> usize {
	buffer_limit.min(segment_size)
}

#[inline]
fn read_size(requested: usize, buffer_limit: usize, segment_size: usize) -> usize {
	requested.clamp(segment_size, max_read_size(buffer_limit, segment_size))
}

impl<'d, S: Source<'d, SIZE>, P: Pool<SIZE>> BufferedSource<'d, S, P> {
	#[inline]
	pub(crate) fn new(source: S, buffer: Buffer<'d, SIZE, P>) -> Self {
		let closed = source.is_closed();
		Self { buffer, source: Some(source), closed, eos: false }
	}

	/// Consumes the buffered sink without closing, returning the inner sink.
	pub fn into_inner(mut self) -> S {
		unsafe {
			// Safety: option will only be None if this method was already called,
			// which is impossible because we consume self.
			self.source.take().unwrap_unchecked()
		}
	}

	fn source_mut(&mut self) -> &mut S {
		unsafe {
			// Safety: option will only be None if into_inner is called, but this
			// consumes and drops self, making it impossible to ever have a
			// reference (except on drop, which is guarded).
			self.source.as_mut().unwrap_unchecked()
		}
	}

	fn source(&self) -> &S {
		unsafe {
			// Safety: see `source_mut`
			self.source.as_ref().unwrap_unchecked()
		}
	}

	fn internals(&mut self) -> (&mut Buffer<'d, SIZE, P>, &mut S, &mut bool) {
		let source = unsafe {
			// Safety: see `source_mut`
			self.source.as_mut().unwrap_unchecked()
		};

		(&mut self.buffer, source, &mut self.eos)
	}

	#[inline]
	fn max_request_size(&self) -> usize {
		self.buffer.limit().max(SIZE)
	}

	/// Determines the request size for a read of `count` bytes. Requests are at
	/// least one segment in length, and at most the buffer limit if the limit is
	/// more than the segment size. This ensures reads have a minimum size for
	/// better efficiency, while limiting allocation during very large reads.
	#[inline]
	fn request_size(&self, count: usize) -> usize {
		read_size(count, self.buffer.limit(), SIZE)
	}
}

impl<'d, S: Source<'d, SIZE>, P: Pool<SIZE>> Stream<SIZE> for BufferedSource<'d, S, P> {
	#[inline]
	fn is_closed(&self) -> bool { self.closed }

	fn close(&mut self) -> StreamResult {
		if !self.closed {
			self.closed = true;
			let buf_result = self.buffer.close();
			let src_result = self.source_mut().close();
			buf_result?;
			src_result?;
		}
		Ok(())
	}
}

impl<'d, S: Source<'d, SIZE>, P: Pool<SIZE>> Source<'d, SIZE> for BufferedSource<'d, S, P> {
	fn is_eos(&self) -> bool {
		self.eos || self.source().is_eos()
	}

	fn fill(&mut self, sink: &mut Buffer<'d, SIZE, impl Pool>, mut count: usize) -> BufferResult<usize> {
		self.check_open(Fill)?;
		let mut read = self.buffer.fill(sink, count)?;
		count -= read;
		read += self.source_mut().fill(sink, count)?;
		Ok(read)
	}

	fn fill_all(&mut self, sink: &mut Buffer<'d, SIZE, impl Pool>) -> BufferResult<usize> {
		self.check_open(Fill)?;
		let mut count = self.buffer.fill_all(sink)?;
		count +=  self.source_mut().fill_all(sink)?;
		Ok(count)
	}
}

impl<'d, S: Source<'d, SIZE>, P: Pool<SIZE>> BufStream<'d, SIZE> for BufferedSource<'d, S, P> {
	type Pool = P;

	fn buf<'b>(&'b self) -> &'b Buffer<'d, SIZE, P> { &self.buffer }
	fn buf_mut<'b>(&'b mut self) -> &'b mut Buffer<'d, SIZE, P> { &mut self.buffer }
}

impl<'d, S: Source<'d, SIZE>, P: Pool<SIZE>> BufSource<'d, SIZE> for BufferedSource<'d, S, P> {
	fn request(&mut self, count: usize) -> StreamResult<bool> {
		self.check_open(Read)?;
		if self.is_eos() { return Ok(false) }

		let (buffer, source, eos) = self.internals();

		// No fill necessary
		if buffer.request(count)? {
			return Ok(true)
		}

		// Fill buffer to its limit
		source.fill_free(buffer)?;
		if buffer.count() >= count {
			return Ok(true)
		}

		while buffer.count() < count && !source.is_eos() {
			let fill_size = count.next_multiple_of(SIZE) - buffer.count();
			match source.fill(buffer, fill_size) {
				Ok(0) => break,
				Ok(_) => { }
				Err(err) if err.is_eos() => {
					*eos = true;
					break
				}
				Err(err) => return Err(err.into())
			}
		}
		Ok(buffer.count() >= count)
	}

	fn read(&mut self, sink: &mut impl Sink<'d, SIZE>, count: usize) -> StreamResult<usize> {
		self.check_open(Read)?;

		let mut read = 0;
		while !self.is_eos() || self.buf_mut().is_not_empty() || read < count {
			let remaining = count - read;
			read += sink.drain(self.buf_mut(), remaining).context(Read)?;
			if self.is_eos() || self.request(self.request_size(remaining))? {
				break
			}
		}
		Ok(read)
	}

	fn read_all(&mut self, sink: &mut impl Sink<'d, SIZE>) -> StreamResult<usize> {
		self.check_open(Read)?;

		let mut read = 0;
		while !self.is_eos() || self.buf().is_not_empty() {
			read += sink.drain_all(self.buf_mut()).context(Read)?;
			self.request(self.max_request_size())?;
		}

		Ok(read)
	}
}

impl<'d, S: Source<'d, SIZE> + Seekable, P: Pool<SIZE>> BufferedSource<'d, S, P> {
	fn seek_back_buf(&mut self, off: usize) -> StreamResult<usize> {
		let cur_pos = self.seek_pos()?;
		let new_pos = self.source_mut().seek_back(off)?;
		let count = cur_pos - new_pos;

		if count == 0 {
			return Ok(new_pos)
		}

		let mut seek_buf = Buffer::<SIZE, P>::default();
		self.source_mut()
			.fill(&mut seek_buf, count)
			.context(Seek)?;
		seek_buf.drain_all(&mut self.buffer)
				.context(Seek)?;
		self.buffer.swap(&mut seek_buf);
		Ok(new_pos)
	}

	fn seek_forward(&mut self, mut off: usize) -> StreamResult<usize> {
		off -= self.buffer.skip(off);
		self.source_mut().seek_forward(off)
	}
}

impl<'d, S: Source<'d, SIZE> + Seekable, P: Pool<SIZE>> Seekable for BufferedSource<'d, S, P> {
	fn seek(&mut self, offset: SeekOffset) -> StreamResult<usize> {
		return match offset {
			SeekOffset::Forward(0) |
			SeekOffset::Back   (0) => self.seek_pos(),
			SeekOffset::Forward(off) => self.seek_forward (off),
			SeekOffset::Back   (off) => self.seek_back_buf(off),
			SeekOffset::FromEnd(_off @ ..=-1) => {
				let len = self.buffer.count();
				let pos = self.buffer.seek(offset)?;

				if pos < len {
					// We didn't seek through the entire buffer, just return the
					// current position.
					self.seek_pos()
				} else {
					// The buffer was exhausted, seek on the source.
					self.source_mut().seek(offset)
				}
			}
			_ => {
				// No clever way to do the rest, just invalidate the buffered data
				// and seek on the source.
				self.buffer.clear();
				self.source_mut().seek(offset)
			}
		}
	}

	fn seek_len(&mut self) -> StreamResult<usize> { self.source_mut().seek_len() }

	fn seek_pos(&mut self) -> StreamResult<usize> {
		// Offset the source position back by the buffer length to account for
		// buffering.
		Ok(self.source_mut().seek_pos()? - self.buffer.count())
	}
}

impl<'d, S: Source<'d, SIZE>, P: Pool<SIZE>> Drop for BufferedSource<'d, S, P> {
	fn drop(&mut self) {
		if self.source.is_some() {
			let _ = self.close();
		}
	}
}

pub struct BufferedSink<'d, S: Sink<'d, SIZE>, P: Pool<SIZE>> {
	buffer: Buffer<'d, SIZE, P>,
	sink: Option<S>,
	closed: bool
}

impl<'d, S: Sink<'d, SIZE>, P: Pool<SIZE>> BufferedSink<'d, S, P> {
	#[inline]
	pub(crate) fn new(sink: S, buffer: Buffer<'d, SIZE, P>) -> Self {
		let closed = sink.is_closed();
		Self { buffer, sink: Some(sink), closed }
	}

	/// Consumes the buffered sink without closing, returning the inner sink.
	pub fn into_inner(mut self) -> S {
		let _ = self.flush();
		unsafe {
			// Safety: option will only be None if this method was already called,
			// which is impossible because we consume self.
			self.sink.take().unwrap_unchecked()
		}
	}

	fn sink_mut(&mut self) -> &mut S {
		unsafe {
			// Safety: option will only be None if into_inner is called, but this
			// consumes and drops self, making it impossible to ever have a
			// reference (except on drop, which is guarded).
			self.sink.as_mut().unwrap_unchecked()
		}
	}

	fn internals(&mut self) -> (&mut Buffer<'d, SIZE, P>, &mut S) {
		let sink = unsafe {
			// Safety: see `sink_mut`
			self.sink.as_mut().unwrap_unchecked()
		};

		(&mut self.buffer, sink)
	}
}

impl<'d, S: Sink<'d, SIZE>, P: Pool<SIZE>> Stream<SIZE> for BufferedSink<'d, S, P> {
	#[inline]
	fn is_closed(&self) -> bool { self.closed }

	fn close(&mut self) -> StreamResult {
		if !self.closed {
			self.closed = true;
			let flush = self.flush();
			let close = self.sink_mut().close();
			let clear = self.buffer.close();
			flush?;
			close?;
			clear
		} else {
			Ok(())
		}
	}
}

impl<'d, S: Sink<'d, SIZE>, P: Pool<SIZE>> Sink<'d, SIZE> for BufferedSink<'d, S, P> {
	fn drain(&mut self, source: &mut Buffer<'d, SIZE, impl Pool>, count: usize) -> BufferResult<usize> {
		self.check_open(Drain)?;
		self.sink_mut().drain(source, count)
	}

	fn drain_all(&mut self, source: &mut Buffer<'d, SIZE, impl Pool>) -> BufferResult<usize> {
		self.check_open(Drain)?;
		self.sink_mut().drain_all(source)
	}

	fn flush(&mut self) -> StreamResult {
		self.check_open(Flush)?;

		// Both of these need a chance to run before returning an error.
		let read = self.drain_all_buffered().context(Flush);
		let flush = self.sink_mut().flush();
		read?;
		flush
	}
}

impl<'d, S: Sink<'d, SIZE>, P: Pool<SIZE>> BufStream<'d, SIZE> for BufferedSink<'d, S, P> {
	type Pool = P;

	fn buf<'b>(&'b self) -> &'b Buffer<'d, SIZE, P> { &self.buffer }
	fn buf_mut<'b>(&'b mut self) -> &'b mut Buffer<'d, SIZE, P> { &mut self.buffer }
}

impl<'d, S: Sink<'d, SIZE>, P: Pool<SIZE>> BufSink<'d, SIZE> for BufferedSink<'d, S, P> {
	fn write(&mut self, source: &mut impl Source<'d, SIZE>, count: usize) -> StreamResult<usize> {
		self.check_open(Write)?;

		let mut written = 0;
		while written < count {
			let remaining = count - written;
			let cur_written = source.fill(self.buf_mut(), remaining).context(Write)?;

			if cur_written == 0 {
				break
			}

			written += cur_written;

			self.drain_buffered().context(Write)?;
		}
		Ok(written)
	}

	fn write_all(&mut self, source: &mut impl Source<'d, SIZE>) -> StreamResult<usize> {
		self.check_open(Write)?;

		let mut written = 0;
		while let cur_written @ 1.. = source.fill_all(self.buf_mut()).context(Write)? {
			written += cur_written;
			if self.buffer.count() == 0 {
				break
			}

			self.drain_buffered().context(Write)?;
		}
		self.drain_all_buffered().context(Write)?;
		Ok(written)
	}

	fn drain_all_buffered(&mut self) -> BufferResult {
		self.check_open(Drain)?;
		let (buf, sink) = self.internals();
		sink.drain_all(buf)?;
		Ok(())
	}

	fn drain_buffered(&mut self) -> BufferResult {
		self.check_open(Drain)?;
		let (buf, sink) = self.internals();
		sink.drain(buf, buf.full_segment_count())?;
		Ok(())
	}
}

impl<'d, S: Sink<'d, SIZE> + Seekable, P: Pool> Seekable for BufferedSink<'d, S, P> {
	fn seek(&mut self, offset: SeekOffset) -> StreamResult<usize> {
		self.check_open(Seek)?;
		// Todo: Is there some less naive approach than flushing then seeking?
		self.drain_all_buffered().context(Seek)?;
		self.sink_mut().seek(offset)
	}

	fn seek_len(&mut self) -> StreamResult<usize> {
		Ok(self.buffer.count() + self.sink_mut().seek_len()?)
	}
}

impl<'d, S: Sink<'d, SIZE>, P: Pool<SIZE>> Drop for BufferedSink<'d, S, P> {
	fn drop(&mut self) {
		if self.sink.is_some() {
			let _ = self.close();
		}
	}
}
