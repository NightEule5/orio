// SPDX-License-Identifier: Apache-2.0

use std::io;
use std::io::{Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use crate::{Buffer, BufferResult, DefaultBuffer, Error, ResultContext, SIZE, StreamContext, StreamResult};
use crate::BufferContext::{Drain, Fill};
use crate::pool::Pool;
use crate::StreamContext::Flush;
use crate::streams::{BufSink, BufSource, Seekable, SeekOffset, Sink, Source, Stream};

/// A [`Source`] reading from a wrapped [`Read`]er.
pub struct ReaderSource<R: Read> {
	reader: Option<R>,
	is_eos: bool,
	/// Allows the use of [`Read::read_vectored`] to possibly speed up reading.
	/// Defaults to `true`.
	pub allow_vectored: bool,
}

/// A [`Sink`] writing to a wrapped [`Write`]r.
pub struct WriterSink<W: Write> {
	writer: Option<W>,
	/// Allows the use of [`Write::write_vectored`] to possibly speed up writing.
	/// Defaults to `true`.
	pub allow_vectored: bool,
}

impl<R: Read> From<R> for ReaderSource<R> {
	fn from(reader: R) -> Self {
		Self {
			reader: Some(reader),
			is_eos: false,
			allow_vectored: true,
		}
	}
}

impl<W: Write> From<W> for WriterSink<W> {
	fn from(writer: W) -> Self {
		Self {
			writer: Some(writer),
			allow_vectored: true,
		}
	}
}

impl<const N: usize, R: Read> Stream<N> for ReaderSource<R> {
	fn is_closed(&self) -> bool {
		self.reader.is_none()
	}

	/// Closes the underlying reader by letting it fall out of scope. Subsequent
	/// reads will fail.
	fn close(&mut self) -> StreamResult {
		self.reader.take();
		Ok(())
	}
}

impl<'d, const N: usize, R: Read> Source<'d, N> for ReaderSource<R> {
	fn is_eos(&self) -> bool {
		self.is_eos
	}

	fn fill(&mut self, sink: &mut Buffer<'d, N, impl Pool<N>>, count: usize) -> BufferResult<usize> {
		if self.is_eos { return Ok(0) }
		let reader = self.reader
						 .as_mut()
						 .ok_or_else(|| Error::closed(Fill))?;
		sink.fill_from_reader(reader, count, self.allow_vectored)
	}
}

impl<const N: usize, W: Write> Stream<N> for WriterSink<W> {
	fn is_closed(&self) -> bool {
		self.writer.is_none()
	}

	/// Closes the underlying writer by letting it fall out of scope. Subsequent
	/// writes will fail.
	fn close(&mut self) -> StreamResult {
		self.writer.take();
		Ok(())
	}
}

impl<'d, const N: usize, W: Write> Sink<'d, N> for WriterSink<W> {
	fn drain(&mut self, source: &mut Buffer<'d, N, impl Pool<N>>, count: usize) -> BufferResult<usize> {
		let writer = self.writer.as_mut().ok_or_else(|| Error::closed(Drain))?;
		source.drain_into_writer(writer, count, self.allow_vectored)
	}

	fn flush(&mut self) -> StreamResult {
		self.writer.as_mut().ok_or_else(|| Error::closed(Flush))?.flush().context(Flush)
	}
}

impl<R: Read + Seek> Seekable for ReaderSource<R> {
	fn seek(&mut self, offset: SeekOffset) -> StreamResult<usize> {
		Ok(
			self.reader
				.as_mut()
				.ok_or_else(|| Error::closed(StreamContext::Seek))?
				.seek(offset.into_seek_from())
				.context(StreamContext::Seek)? as usize
		)
	}
}

impl<W: Write + Seek> Seekable for WriterSink<W> {
	fn seek(&mut self, offset: SeekOffset) -> StreamResult<usize> {
		Ok(
			self.writer
				.as_mut()
				.ok_or_else(|| Error::closed(StreamContext::Seek))?
				.seek(offset.into_seek_from())
				.context(StreamContext::Seek)? as usize
		)
	}
}

/// A wrapper implementing the [`Read`] trait for a [`Source`].
pub struct SourceReader<'d, S: Source<'d, SIZE>>(S, PhantomData<&'d ()>);

/// A wrapper implementing the [`Write`] trait for a [`Sink`].
pub struct SinkWriter<'d, S: Sink<'d, SIZE>>(S, PhantomData<&'d ()>);

pub trait IntoRead<'d, const N: usize>: Source<'d, N> + Sized {
	type Reader: Read;
	fn into_read(self) -> Self::Reader;
}

pub trait IntoWrite<'d, const N: usize>: Sink<'d, N> + Sized {
	type Writer: Write;
	fn into_write(self) -> Self::Writer;
}

impl<'d, S: Source<'d, SIZE>> IntoRead<'d, SIZE> for S
where SourceReader<'d, Self>: Read {
	type Reader = SourceReader<'d, Self>;
	fn into_read(self) -> Self::Reader {
		SourceReader(self, PhantomData)
	}
}

impl<'d, S: Sink<'d, SIZE>> IntoWrite<'d, SIZE> for S
where SinkWriter<'d, Self>: Write {
	type Writer = SinkWriter<'d, Self>;
	fn into_write(self) -> Self::Writer {
		SinkWriter(self, PhantomData)
	}
}

impl<'d, S: Source<'d, SIZE>> Read for SourceReader<'d, S> {
	default fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		let Self(source, ..) = self;
		let ref mut buffer = DefaultBuffer::default();
		let count = source.fill(buffer, buf.len())?;
		buffer.read_slice(buf)?;
		Ok(count)
	}
}

impl<'d, S: BufSource<'d, SIZE>> Read for SourceReader<'d, S> {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		let Self(source, ..) = self;
		Ok(source.read_slice(buf)?)
	}
}

impl<'d, S: Source<'d, SIZE> + Seekable> Seek for SourceReader<'d, S> {
	fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
		let Self(source, ..) = self;
		Ok(source.seek(pos.into())? as u64)
	}
}

impl<'d, S: Source<'d, SIZE>> Drop for SourceReader<'d, S> {
	fn drop(&mut self) {
		let _ = self.0.close();
	}
}

impl<'d, S: Sink<'d, SIZE>> Write for SinkWriter<'d, S> {
	default fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		let Self(sink, ..) = self;
		Ok(
			sink.drain_all(
				&mut Buffer::from_slice(buf).detached()
			)?
		)
	}

	default fn flush(&mut self) -> io::Result<()> {
		let Self(sink, ..) = self;
		Ok(sink.flush()?)
	}
}

impl<'d, S: BufSink<'d, SIZE>> Write for SinkWriter<'d, S> {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		let Self(sink, ..) = self;
		Ok(sink.write_from_slice(buf)?)
	}

	fn flush(&mut self) -> io::Result<()> {
		let Self(sink, ..) = self;
		Ok(sink.flush()?)
	}
}

impl<'d, S: Sink<'d, SIZE> + Seekable> Seek for SinkWriter<'d, S> {
	fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
		let Self(sink, ..) = self;
		Ok(sink.seek(pos.into())? as u64)
	}
}

impl<'d, S: Sink<'d, SIZE>> Drop for SinkWriter<'d, S> {
	fn drop(&mut self) {
		let _ = self.0.close();
	}
}
