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
use std::{io, mem, result};
use std::cmp::min;
use amplify_derive::Display;
use OperationKind::{BufRead, BufWrite};
use crate::{Buffer, DEFAULT_SEGMENT_SIZE, error};
use crate::buffered_wrappers::{buffer_sink, buffer_source};
use crate::pool::{Pool, Error as PoolError};
use crate::segment::OffsetUtf8Error;
use crate::streams::ErrorKind::{Closed, Eos, InvalidUTF8, Io, Other};
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
	#[display("invalid UTF-8")]
	InvalidUTF8,
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

impl From<PoolError> for Error {
	fn from(value: PoolError) -> Self { Self::pool(value) }
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
		Self::new(OperationKind::Unknown, ErrorKind::Pool, Some(error.into()))
	}

	/// Create a new UTF-8 error.
	pub fn invalid_utf8(op: OperationKind, error: OffsetUtf8Error) -> Self {
		Self::new(op, InvalidUTF8, Some(error.into()))
	}

	/// Returns the source downcast into an IO Error, if possible.
	pub fn io_source(&self) -> Option<&io::Error> {
		self.source()?.downcast_ref()
	}

	/// Convenience shorthand for `with_operation(OperationKind::BufRead)`.
	pub fn with_op_buf_read(self) -> Self { self.with_operation(BufRead) }

	/// Convenience shorthand for `with_operation(OperationKind::BufWrite)`.
	pub fn with_op_buf_write(self) -> Self { self.with_operation(BufWrite) }

	/// Convenience shorthand for `with_operation(OperationKind::BufClear)`.
	pub fn with_op_buf_clear(self) -> Self { self.with_operation(BufClear) }

	/// Convenience shorthand for `with_operation(OperationKind::BufFlush)`.
	pub fn with_op_buf_flush(self) -> Self { self.with_operation(BufFlush) }
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
	fn buf(&self) -> &Buffer<impl Pool>;
	fn buf_mut(&mut self) -> &mut Buffer<impl Pool>;
}

macro_rules! gen_int_reads {
    ($($be_name:ident$($le_name:ident)?->$ty:ident,)+) => {
		$(gen_int_reads! { $be_name$($le_name)?->$ty })+
	};
	($be_name:ident$le_name:ident->$ty:ident) => {
		gen_int_reads! { $be_name->$ty "big-endian " }
		gen_int_reads! { $le_name->$ty "little-endian " }
	};
	($name:ident->$ty:ident$($endian:literal)?) => {
		#[doc = concat!(" Reads one ",$($endian,)?"[`",stringify!($ty),"`] from the source.")]
		fn $name(&mut self) -> Result<$ty> {
			self.require(mem::size_of::<$ty>())?;
			self.buf_mut().$name()
		}
	}
}

pub trait BufSource: BufStream + Source {
	/// Reads up to `byte_count` bytes into the buffer, returning whether the
	/// requested count is available. To return an end-of-stream error, use
	/// [`Self::require`].
	fn request(&mut self, byte_count: usize) -> Result<bool>;
	/// Reads at least `byte_count` bytes into the buffer, returning an
	/// end-of-stream error if not successful. To return `true` if the requested
	/// count is available, use [`Self::request`].
	fn require(&mut self, byte_count: usize) -> Result {
		if self.request(byte_count)? {
			Err(Error::eos(BufRead))
		} else {
			Ok(())
		}
	}

	fn read_all(&mut self, sink: &mut impl Sink) -> Result<usize>;

	gen_int_reads! {
		read_i8 -> i8,
		read_u8 -> u8,
		read_i16 read_i16_le -> i16,
		read_u16 read_u16_le -> u16,
		read_i32 read_i32_le -> i32,
		read_u32 read_u32_le -> u32,
		read_i64 read_i64_le -> i64,
		read_u64 read_u64_le -> u64,
		read_isize read_isize_le -> isize,
		read_usize read_usize_le -> usize,
	}

	/// Removes `byte_count` bytes from the source.
	fn skip(&mut self, mut byte_count: usize) -> Result<usize> {
		let mut n = 0;
		while byte_count > 0 && self.request(calc_read_count(byte_count, self.buf()))? {
			let skipped = self.buf_mut().skip(byte_count)?;
			n += skipped;
			byte_count -= skipped;
		}
		Ok(n)
	}

	/// Reads bytes into a slice, returning the number of bytes read.
	fn read_into_slice(&mut self, mut dst: &mut [u8]) -> Result<usize> {
		let mut n = 0;
		while !dst.is_empty() && self.request(calc_read_count(dst.len(), self.buf()))? {
			let read = self.buf_mut().read_into_slice(dst)?;
			n += read;
			dst = &mut dst[read..];
		}
		Ok(n)
	}

	/// Reads the exact length of bytes into a slice, returning an end-of-stream if
	/// the slice could not be filled. Bytes are not consumed from the buffer if
	/// end-of-stream is returned.
	fn read_into_slice_exact(&mut self, dst: &mut [u8]) -> Result {
		let len = dst.len();
		while self.request(len.saturating_sub(self.buf().count()))? { }

		self.buf_mut().read_into_slice_exact(dst)
	}
	
	fn read_array<const N: usize>(&mut self) -> Result<[u8; N]> {
		let mut array = [0; N];
		self.read_into_slice_exact(&mut array)?;
		Ok(array)
	}

	/// Reads all bytes from the source, decoding them into `str` as UTF-8.
	fn read_all_utf8(&mut self, str: &mut String) -> Result {
		while self.read_utf8(str, usize::MAX)? > 0 { }
		Ok(())
	}

	/// Reads at most `byte_count` bytes from the source, decoding them into `str`
	/// as UTF-8. Returns the number of bytes read.
	fn read_utf8(&mut self, str: &mut String, mut byte_count: usize) -> Result<usize> {
		let mut n = 0;
		while byte_count > 0 && self.request(calc_read_count(byte_count, self.buf()))? {
			let read = self.buf_mut().read_utf8(str, byte_count)?;
			n += read;
			byte_count -= read;
		}
		Ok(n)
	}

	/// Reads UTF-8 text into `str` until a line terminator, returning whether the
	/// terminator was encountered. The line terminator is not written to the string.
	fn read_utf8_line(&mut self, str: &mut String) -> Result<bool> {
		while self.request(calc_read_count(usize::MAX, self.buf()))? {
			if self.buf_mut().read_utf8_line(str)? {
				return Ok(true)
			}
		}
		Ok(false)
	}
}

fn calc_read_count(byte_count: usize, buf: &Buffer<impl Pool>) -> usize {
	min(byte_count, DEFAULT_SEGMENT_SIZE.saturating_sub(buf.count()))
}

macro_rules! gen_int_writes {
    ($($be_name:ident$($le_name:ident)?->$ty:ident,)+) => {
		$(gen_int_writes! { $be_name$($le_name)?->$ty })+
	};
	($be_name:ident$le_name:ident->$ty:ident) => {
		gen_int_writes! { $be_name->$ty "big-endian " }
		gen_int_writes! { $le_name->$ty "little-endian " }
	};
	($name:ident->$ty:ident$($endian:literal)?) => {
		#[doc = concat!(" Writes one ",$($endian,)?"[`",stringify!($ty),"`] to the source.")]
		fn $name(&mut self, value: $ty) -> Result {
			self.buf_mut().$name(value)
		}
	}
}

pub trait BufSink: BufStream + Sink {
	fn write_all(&mut self, source: &mut impl Source) -> Result<usize>;

	gen_int_writes! {
		write_i8 -> i8,
		write_u8 -> u8,
		write_i16 write_i16_le -> i16,
		write_u16 write_u16_le -> u16,
		write_i32 write_i32_le -> i32,
		write_u32 write_u32_le -> u32,
		write_i64 write_i64_le -> i64,
		write_u64 write_u64_le -> u64,
		write_isize write_isize_le -> isize,
		write_usize write_usize_le -> usize,
	}

	fn write_from_slice(&mut self, value: &[u8]) -> Result {
		self.buf_mut().write_from_slice(value)
	}

	fn write_utf8(&mut self, value: &str) -> Result {
		self.buf_mut().write_utf8(value)
	}
}