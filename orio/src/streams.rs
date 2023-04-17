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

use std::error::Error as StdError;
use std::{io, result};
use amplify_derive::Display;
use OperationKind::{BufRead, BufWrite};
use crate::{Buffer, DEFAULT_SEGMENT_SIZE, error};
use crate::buffered_wrappers::{buffer_sink, buffer_source};
use crate::pool::{DefaultPool, Pool, Error as PoolError};
use crate::streams::ErrorKind::{Closed, Eos, Io, Other};
use crate::streams::OperationKind::{BufClear, BufFlush};

pub type Error = error::Error<OperationKind, ErrorKind>;
pub type Result<T = ()> = result::Result<T, Error>;

#[derive(Copy, Clone, Debug, Default, Display)]
pub enum OperationKind {
	#[default]
	#[display("unknown operation")]
	Unknown,
	#[display("read from buffer")]
	BufRead,
	#[display("write to buffer")]
	BufWrite,
	#[display("clear buffer")]
	BufClear,
	#[display("flush buffer")]
	BufFlush,
	#[display("{0}")]
	Other(&'static str)
}

impl error::OperationKind for OperationKind {
	fn unknown() -> Self { Self::Unknown }
}

#[derive(Copy, Clone, Debug, Display)]
pub enum ErrorKind {
	#[display("premature end-of-stream")]
	Eos,
	#[display("IO error")]
	Io,
	#[display("stream closed")]
	Closed,
	#[display("segment pool error")]
	Pool,
	#[display("{0}")]
	Other(&'static str),
}

impl error::ErrorKind for ErrorKind {
	fn other(message: &'static str) -> Self { Other(message) }
}

impl From<io::Error> for Error {
	fn from(value: io::Error) -> Self {
		if let io::ErrorKind::UnexpectedEof = value.kind() {
			Self::eos(OperationKind::Unknown)
		} else {
			Self::io(OperationKind::Unknown, value)
		}
	}
}

impl Error {
	/// Creates a new "end-of-stream" error.
	pub fn eos(op: OperationKind) -> Self { Self::new(op, Eos, None) }

	/// Creates a new IO error.
	pub fn io(op: OperationKind, error: io::Error) -> Self {
		Self::new(op, Io, Some(error.into()))
	}

	/// Creates a new "closed" error.
	pub fn closed(op: OperationKind) -> Self {
		Self::new(op, Closed, None)
	}

	/// Creates a new segment pool error.
	pub fn pool(error: PoolError) -> Self {
		Self::new(error.into(), ErrorKind::Pool, Some(error.into()))
	}

	/// Returns the source downcast into an IO Error, if possible.
	pub fn io_source(&self) -> Option<&io::Error> {
		self.source()?.downcast_ref()
	}

	/// Convenience shorthand for `with_operation(OperationKind::BufRead)`.
	pub fn with_op_buf_read(mut self) -> Self { self.with_operation(BufRead) }

	/// Convenience shorthand for `with_operation(OperationKind::BufWrite)`.
	pub fn with_op_buf_write(mut self) -> Self { self.with_operation(BufWrite) }

	/// Convenience shorthand for `with_operation(OperationKind::BufClear)`.
	pub fn with_op_buf_clear(mut self) -> Self { self.with_operation(BufClear) }

	/// Convenience shorthand for `with_operation(OperationKind::BufFlush)`.
	pub fn with_op_buf_flush(mut self) -> Self { self.with_operation(BufFlush) }
}

/// A data stream, either [`Source`] or [`Sink`]
pub trait Stream {
	/// Closes the stream. All default streams close automatically when dropped.
	/// Closing is idempotent, [`close`] may be called more than once with no
	/// effect.
	fn close(&mut self) -> Result { Ok(()) }
}

/// A data source.
pub trait Source: Stream {
	/// Reads `count` bytes from the source into the buffer.
	fn read(&mut self, sink: &mut Buffer<impl Pool>, count: usize) -> Result<usize>;

	/// Reads all bytes from the source into the buffer.
	#[inline]
	fn read_all(&mut self, sink: &mut Buffer<impl Pool>) -> Result<usize> {
		self.read(sink, usize::MAX)
	}
}

pub trait SourceBuffer: Source + Sized {
	/// Wrap the source in a buffered source.
	fn buffer<P: Pool + Default>(self) -> impl BufSource { buffer_source::<_, P>(self) }
}

impl<S: Source> SourceBuffer for S { }

/// A data sink.
pub trait Sink: Stream {
	/// Writes `count` bytes from the buffer into the sink.
	fn write(
		&mut self,
		source: &mut Buffer<impl Pool>,
		count: usize
	) -> Result<usize>;

	/// Writes all bytes from the buffer into the sink.
	#[inline]
	fn write_all(&mut self, source: &mut Buffer<impl Pool>) -> Result<usize> {
		self.write(source, source.count())
	}

	/// Writes all buffered data to its final target.
	fn flush(&mut self) -> Result { Ok(()) }
}

impl<S: Sink> Stream for S {
	/// Flushes and closes the stream. All default streams close automatically when
	/// dropped. Closing is idempotent, [`close`] may be called more than once with
	/// no effect.
	default fn close(&mut self) -> Result { self.flush() }
}

pub trait SinkBuffer: Sink + Sized {
	/// Wrap the sink in a buffered sink.
	fn buffer<P: Pool + Default>(self) -> impl BufSink { buffer_sink::<_, P>(self) }
}

impl<S: Sink> SinkBuffer for S { }

pub trait BufStream {
	type Pool: Pool = DefaultPool;
	fn buf(&mut self) -> &mut Buffer<Self::Pool>;
}

pub trait BufSource: BufStream + Source {
	fn read_all(&mut self, sink: &mut impl Sink) -> Result<usize>;
}

pub trait BufSink: BufStream + Sink {
	fn write_all(&mut self, source: &mut impl Source) -> Result<usize>;
}
