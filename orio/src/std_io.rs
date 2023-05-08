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
use std::io::{Read, Write};
use crate::Buffer;
use crate::pool::SharedPool;
use crate::streams::{BufSink, BufSource, Error, ErrorKind, Result, Sink, Source};
use crate::streams::OperationKind::BufFlush;

impl<R: Read> Source for R {
	fn read(&mut self, sink: &mut Buffer<impl SharedPool>, count: usize) -> Result<usize> {
		sink.write_std(self, count)
			.map_err(Error::with_op_buf_read)
	}
}

impl<W: Write> Sink for W {
	fn write(&mut self, source: &mut Buffer<impl SharedPool>, count: usize) -> Result<usize> {
		source.read_std(self, count)
			  .map_err(Error::with_op_buf_write)
	}

	fn flush(&mut self) -> Result {
		self.flush()
			.map_err(|err| Error::io(BufFlush, err))
	}
}

pub trait IntoRead: Source + Sized {
	fn into_read(self) -> impl Read { SourceReader(self) }
}

pub trait IntoWrite: Sink + Sized {
	fn into_write(self) -> impl Write { SinkWriter(self) }
}

default impl<S: Source> IntoRead  for S { }
default impl<S: Sink  > IntoWrite for S { }

struct SourceReader<S>(S);

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

struct SinkWriter<S>(S);

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
