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

use std::io;
use std::io::{Read, Seek, SeekFrom, Write};
use crate::Buffer;
use crate::pool::SharedPool;
use crate::streams::{BufSink, BufSource, BufStream, Error, Result, Seekable, SeekOffset, Sink, Source};
use crate::streams::OperationKind::{BufFlush, Seek as SeekOp};

trait AsInner {
	type Inner;
	fn as_inner(&mut self) -> &mut Self::Inner;
}

/// A [`Source`] reading from a wrapped [`Read`]er.
pub struct ReaderSource<R: Read>(R);

/// A [`Sink`] writing to a wrapped [`Write`]r.
pub struct WriterSink<W: Write>(W);

impl<R: Read> From<R> for ReaderSource<R> {
	fn from(value: R) -> Self { Self(value) }
}

impl<W: Write> From<W> for WriterSink<W> {
	fn from(value: W) -> Self { Self(value) }
}

impl<R: Read> AsInner for ReaderSource<R> {
	type Inner = R;
	fn as_inner(&mut self) -> &mut R {
		let Self(reader) = self;
		reader
	}
}

impl<W: Write> AsInner for WriterSink<W> {
	type Inner = W;
	fn as_inner(&mut self) -> &mut W {
		let Self(writer) = self;
		writer
	}
}

impl<R: Read> Source for ReaderSource<R> {
	fn read(&mut self, sink: &mut Buffer<impl SharedPool>, count: usize) -> Result<usize> {
		let Self(reader) = self;
		sink.write_std(reader, count)
			.map_err(Error::with_op_buf_read)
	}
}

impl<W: Write> Sink for WriterSink<W> {
	fn write(&mut self, source: &mut Buffer<impl SharedPool>, count: usize) -> Result<usize> {
		let Self(writer) = self;
		source.read_std(writer, count)
			  .map_err(Error::with_op_buf_write)
	}

	fn flush(&mut self) -> Result {
		let Self(writer) = self;
		writer.flush()
			  .map_err(|err| Error::io(BufFlush, err))
	}
}

impl<T: AsInner<Inner: Seek>> Seekable for T {
	fn seek(&mut self, offset: SeekOffset) -> Result<usize> {
		Ok(
			self.as_inner()
				.seek(offset.into_seek_from())
				.map_err(|err| Error::io(SeekOp, err))? as usize
		)
	}
}

/// A wrapper implementing the [`Read`] trait for [`Source`].
pub struct SourceReader<S: Source>(S);

/// A wrapper implementing the [`Write`] trait for [`Sink`].
pub struct SinkWriter<S: Sink>(S);

pub trait IntoRead: Source + Sized {
	type Reader: Read + From<Self>;
	fn into_read(self) -> Self::Reader { self.into() }
}

pub trait IntoWrite: Sink + Sized {
	type Writer: Write + From<Self>;
	fn into_write(self) -> Self::Writer { self.into() }
}

default impl<S: Source> IntoRead for S {
	type Reader = SourceReader<S>;
}

default impl<S: Sink> IntoWrite for S {
	type Writer = SinkWriter<S>;
}

impl<S: Source> From<S> for SourceReader<S> {
	fn from(value: S) -> Self { Self(value) }
}

impl<S: Source> Read for SourceReader<S> {
	default fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		let Self(source) = self;
		let ref mut buffer = Buffer::default();
		let count = source.read(buffer, buf.len())
						  .map_err(Error::into_io)?;
		buffer.read_into_slice_exact(buf)
			  .map_err(Error::into_io)?;
		Ok(count)
	}
}

impl<S: BufSource> Read for SourceReader<S> {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		let Self(source) = self;
		source.read_into_slice(buf)
			  .map_err(Error::into_io)
	}
}

impl<S: Source> AsInner for SourceReader<S> {
	type Inner = S;

	fn as_inner(&mut self) -> &mut S {
		let Self(source) = self;
		source
	}
}

impl<S: Source + Seekable> Seek for SourceReader<S> {
	fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> { bridge_seek_impl(self, pos) }
}

impl<S: Sink> From<S> for SinkWriter<S> {
	fn from(value: S) -> Self { Self(value) }
}

impl<S: Sink> Write for SinkWriter<S> {
	default fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		let Self(sink) = self;
		let ref mut buffer = Buffer::from_slice(buf).map_err(Error::into_io)?;
		sink.write_all(buffer)
			.map_err(Error::into_io)
	}

	default fn flush(&mut self) -> io::Result<()> {
		self.0
			.flush()
			.map_err(Error::into_io)
	}
}

impl<S: BufSink> Write for SinkWriter<S> {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		let Self(sink) = self;
		sink.write_from_slice(buf)
			.map_err(Error::into_io)?;
		Ok(buf.len())
	}

	fn flush(&mut self) -> io::Result<()> {
		self.0
			.flush()
			.map_err(Error::into_io)
	}
}

impl<S: Sink> AsInner for SinkWriter<S> {
	type Inner = S;

	fn as_inner(&mut self) -> &mut S {
		let Self(sink) = self;
		sink
	}
}

impl<S: Sink + Seekable> Seek for SinkWriter<S> {
	fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> { bridge_seek_impl(self, pos) }
}

fn bridge_seek_impl(stream: &mut impl AsInner<Inner: Seekable>, pos: SeekFrom) -> io::Result<u64> {
	Ok(
		stream.as_inner()
			  .seek(pos.into())
			  .map_err(Error::into_io)? as u64
	)
}