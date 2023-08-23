// SPDX-License-Identifier: Apache-2.0

use std::error::Error as StdError;
use std::{fmt, mem};
use std::cmp::min;
use std::fmt::{Display, Formatter};
use simdutf8::compat::Utf8Error;
use crate::{Buffer, BufferOptions, ByteStr, ByteString, Context::BufRead, Error, Result, ResultExt, SEGMENT_SIZE};
use crate::buffered_wrappers::{buffer_sink, buffer_source, BufferedSink, BufferedSource};
use crate::pool::SharedPool;
use crate::streams::codec::{Decode, Encode};

pub mod codec;
mod seeking;
mod void;

pub use seeking::*;
pub use void::*;

/// A data source.
pub trait Source {
	/// Reads `count` bytes from the source into the buffer.
	fn read(&mut self, sink: &mut Buffer<impl SharedPool>, count: usize) -> Result<usize>;

	/// Reads all bytes from the source into the buffer.
	#[inline]
	fn read_all(&mut self, sink: &mut Buffer<impl SharedPool>) -> Result<usize> {
		self.read(sink, usize::MAX)
	}

	/// Closes the source. All default streams close automatically when dropped.
	/// Closing is idempotent, [`close`] may be called more than once with no
	/// effect.
	fn close_source(&mut self) -> Result { Ok(()) }
}

pub trait SourceBuffer: Source + Sized {
	/// Wrap the source in a buffered source.
	fn buffer(self) -> BufferedSource<Self> {
		self.buffer_with(BufferOptions::default())
	}

	fn buffer_with(self, options: BufferOptions) -> BufferedSource<Self> {
		buffer_source(self, options)
	}
}

impl<S: Source> SourceBuffer for S { }

/// A data sink.
pub trait Sink {
	/// Writes `count` bytes from the buffer into the sink.
	fn write(
		&mut self,
		source: &mut Buffer<impl SharedPool>,
		count: usize
	) -> Result<usize>;

	/// Writes all bytes from the buffer into the sink.
	#[inline]
	fn write_all(
		&mut self,
		source: &mut Buffer<impl SharedPool>
	) -> Result<usize> {
		self.write(source, source.count())
	}

	/// Writes all buffered data to its final target.
	fn flush(&mut self) -> Result { Ok(()) }

	/// Flushes and closes the sink. All default streams close automatically when
	/// dropped. Closing is idempotent, [`close`] may be called more than once with
	/// no effect.
	fn close_sink(&mut self) -> Result { self.flush() }
}

pub trait SinkBuffer: Sink + Sized {
	/// Wrap the sink in a buffered sink.
	fn buffer(self) -> BufferedSink<Self> { buffer_sink(self) }
}

impl<S: Sink> SinkBuffer for S { }

pub trait BufStream {
	fn buf(&self) -> &Buffer<impl SharedPool>;
	fn buf_mut(&mut self) -> &mut Buffer<impl SharedPool>;
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
			Ok(())
		} else {
			Err(Error::eos(BufRead))
		}
	}

	fn read_all(&mut self, sink: &mut impl Sink) -> Result<usize>;

	fn read_into(&mut self, value: &mut impl Decode, byte_count: usize) -> Result<usize> {
		value.decode::<SEGMENT_SIZE>(self.buf_mut(), byte_count, false)
	}

	fn read_into_le(&mut self, value: &mut impl Decode, byte_count: usize) -> Result<usize> {
		value.decode::<SEGMENT_SIZE>(self.buf_mut(), byte_count, true)
	}

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

	/// Reads up to `byte_count` bytes into a [`ByteString`].
	fn read_byte_str(&mut self, byte_count: usize) -> Result<ByteString> {
		self.request(byte_count)?;
		self.buf_mut().read_byte_str(byte_count)
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

	/// Reads UTF-8 text into a string slice, returning the number of bytes read.
	fn read_utf8_into_slice(&mut self, mut str: &mut str) -> Result<usize> {
		let mut n = 0;
		while str.len() > 0 && self.request(calc_read_count(str.len(), self.buf()))? {
			let read = self.buf_mut().read_utf8_into_slice(str)?;
			n += read;
			str = &mut str[read..];
		}
		Ok(n)
	}
}

fn calc_read_count(byte_count: usize, buf: &Buffer<impl SharedPool>) -> usize {
	min(byte_count, SEGMENT_SIZE.saturating_sub(buf.count()))
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

	fn write_from(&mut self, value: impl Encode) -> Result<usize> {
		value.encode::<SEGMENT_SIZE>(self.buf_mut(), false)
	}

	fn write_from_le(&mut self, value: impl Encode) -> Result<usize> {
		value.encode::<SEGMENT_SIZE>(self.buf_mut(), true)
	}

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

	fn write_byte_str(&mut self, value: &ByteStr) -> Result {
		for slice in value.iter() {
			self.write_from_slice(slice)?;
		}
		Ok(())
	}

	fn write_byte_string(&mut self, value: &ByteString) -> Result {
		self.write_from_slice(value.as_slice())
	}

	fn write_from_slice(&mut self, value: &[u8]) -> Result {
		self.buf_mut().write_from_slice(value)
	}

	fn write_utf8(&mut self, value: &str) -> Result {
		self.buf_mut().write_utf8(value)
	}
}

// Impls

impl Source for &[u8] {
	fn read(&mut self, sink: &mut Buffer<impl SharedPool>, mut count: usize) -> Result<usize> {
		count = min(count, self.len());
		(&self[..count]).read_all(sink)?;
		*self = &self[count..];
		Ok(count)
	}

	fn read_all(&mut self, sink: &mut Buffer<impl SharedPool>) -> Result<usize> {
		sink.write_from_slice(self).context(BufRead)?;
		let len = self.len();
		*self = &self[len..];
		Ok(len)
	}
}

// Into

/// Converts some type into a [`Source`].
pub trait IntoSource<S: Source> {
	fn into_source(self) -> S;
}

/// Converts some type into a [`Sink`].
pub trait IntoSink<S: Sink> {
	fn into_sink(self) -> S;
}

impl<S: Source, T: Into<S>> IntoSource<S> for T {
	fn into_source(self) -> S { self.into() }
}

impl<S: Sink, T: Into<S>> IntoSink<S> for T {
	fn into_sink(self) -> S { self.into() }
}

#[derive(Copy, Clone, Debug)]
pub struct OffsetUtf8Error {
	inner: Utf8Error,
	offset: usize
}

impl OffsetUtf8Error {
	pub(crate) fn new(inner: Utf8Error, offset: usize) -> Self {
		Self { inner, offset }
	}

	pub fn into_inner(self) -> Utf8Error { self.inner }

	pub fn valid_up_to(&self) -> usize {
		self.offset + self.inner.valid_up_to()
	}

	pub fn error_len(&self) -> Option<usize> {
		self.inner.error_len()
	}
}

impl From<Utf8Error> for OffsetUtf8Error {
	fn from(value: Utf8Error) -> Self { Self::new(value, 0) }
}

impl Display for OffsetUtf8Error {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		if let Some(error_len) = self.error_len() {
			write!(
				f,
				"invalid utf-8 sequence of {error_len} bytes from index {}",
				self.valid_up_to()
			)
		} else {
			write!(
				f,
				"incomplete utf-8 byte sequence from index {}",
				self.valid_up_to()
			)
		}
	}
}

impl StdError for OffsetUtf8Error {
	fn source(&self) -> Option<&(dyn StdError + 'static)> {
		Some(&self.inner)
	}
}
