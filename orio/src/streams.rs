// SPDX-License-Identifier: Apache-2.0

use std::result;
use num_traits::PrimInt;
use crate::pool::Pool;

mod seeking;
mod void;

pub use seeking::*;
pub use void::*;
use crate::{Buffer, BufferResult, Error, ErrorSource, ResultContext, SIZE, StreamContext, StreamError};
use crate::buffered_wrappers::{BufferedSink, BufferedSource};
use crate::error::Context;
use crate::pattern::Pattern;
use crate::StreamContext::{Read, Write};

/// An "stream closed" error.
#[derive(Copy, Clone, Debug, Default, thiserror::Error)]
#[error("stream closed")]
pub struct StreamClosed;
/// An end-of-stream error.
#[derive(Copy, Clone, Debug, Default, thiserror::Error)]
#[error("premature end-of-stream{}", self.format_req())]
pub struct EndOfStream {
	/// The number of bytes required for reading.
	pub required_count: Option<usize>
}

impl EndOfStream {
	fn format_req(&self) -> String {
		self.required_count.map_or_else(
			Default::default,
			|n| format!("(required {n} bytes)")
		)
	}
}

impl From<usize> for EndOfStream {
	fn from(value: usize) -> Self {
		Self { required_count: Some(value) }
	}
}

impl StreamError {
	fn end_of_stream(required_count: usize, context: StreamContext) -> Self {
		Self {
			source: ErrorSource::Eos(required_count.into()),
			context
		}
	}
}

pub type Result<T = (), E = StreamError> = result::Result<T, E>;

pub trait Stream<const N: usize> {
	/// Returns whether the stream is closed.
	fn is_closed(&self) -> bool;
	/// Closes the stream if not already closed. For [`Sink`]s, this should flush
	/// buffered bytes to the sink. Closing must be idempotent; streams that have
	/// already been closed must return `Ok` on subsequent calls. [`Buffer`] is
	/// the sole exception to the close-idempotence rule.
	///
	/// All default streams close automatically when dropped.
	fn close(&mut self) -> Result {
		// Hack to workaround "conflicting implementation" when specializing to
		// call flush on sinks.
		trait CloseSpec<const N: usize, S: Stream<N> + ?Sized> {
			fn close_spec(&mut self) -> Result;
		}

		impl<const N: usize, S: Stream<N> + ?Sized> CloseSpec<N, S> for S {
			default fn close_spec(&mut self) -> Result {
				Ok(())
			}
		}

		impl<'d, const N: usize, S: Sink<'d, N> + ?Sized> CloseSpec<N, S> for S {
			fn close_spec(&mut self) -> Result {
				self.flush()
			}
		}

		self.close_spec()
	}
	/// Checks whether the stream is open, returning an error if closed.
	#[inline]
	#[allow(private_bounds)]
	fn check_open<C: Context>(&self, context: C) -> Result<(), Error<C>> {
		if self.is_closed() {
			Err(Error::closed(context))
		} else {
			Ok(())
		}
	}
}

pub trait Source<'d, const N: usize = SIZE>: Stream<N> {
	/// Returns `true` if end-of-stream was reached. The end-of-stream state must
	/// be *terminal*; once this method returns `true` it must always return `true`.
	/// [`Buffer`] is the sole exception to the terminality rule.
	fn is_eos(&self) -> bool;
	/// Fills a buffer with up to `count` bytes read from the source, returning the
	/// number of bytes read.
	/// 
	/// # Errors
	/// 
	/// If any error occurs, any bytes that have already been written to the buffer
	/// will remain buffered.
	fn fill(
		&mut self,
		sink: &mut Buffer<'d, N, impl Pool<N>>,
		count: usize
	) -> BufferResult<usize>;
	/// Fills free space in the buffer with bytes read from the source, returning
	/// the number of bytes read. Shorthand for `fill(sink, sink.limit())`.
	///
	/// # Errors
	///
	/// If any error occurs, any bytes that have already been written to the buffer
	/// will remain buffered.
	fn fill_free(&mut self, sink: &mut Buffer<'d, N, impl Pool<N>>) -> BufferResult<usize> {
		self.fill(sink, sink.limit())
	}
	/// Fills a buffer with all available data read from the source, returning the
	/// number of bytes read. This may not read the source to its end.
	///
	/// The default implementation fills free space in `sink`, then fills with `N`
	/// bytes repeatedly until end-of-stream is reached or no bytes are read.
	///
	/// # Errors
	///
	/// If any error occurs, any bytes that have already been written to the buffer
	/// will remain buffered.
	fn fill_all(
		&mut self,
		sink: &mut Buffer<'d, N, impl Pool<N>>
	) -> BufferResult<usize> {
		let mut total = 0;
		let mut last = self.fill_free(sink)?;
		while !self.is_eos() && last > 0 {
			total += last;
			last = self.fill(sink, N)?;
		}
		Ok(total)
	}
}

pub trait SourceExt<'d, const N: usize, P: Pool<N>>: Source<'d, N> + Sized {
	type Buffered: BufSource<'d, N, Pool = P>;

	fn buffered(self) -> Self::Buffered;
}

impl<'d, S: Source<'d, SIZE>, P: Pool<SIZE>> SourceExt<'d, SIZE, P> for S {
	type Buffered = BufferedSource<'d, Self, P>;

	fn buffered(self) -> Self::Buffered {
		BufferedSource::new(self, Buffer::default())
	}
}

pub trait Sink<'d, const N: usize = SIZE>: Stream<N> {
	/// Drains a buffer by writing up to `count` bytes into the sink, returning the
	/// number of bytes written.
	fn drain(
		&mut self,
		source: &mut Buffer<'d, N, impl Pool<N>>,
		count: usize
	) -> BufferResult<usize>;
	/// Drains a buffer by writing full segments into the sink, returning the number
	/// of bytes written.
	fn drain_full(
		&mut self,
		source: &mut Buffer<'d, N, impl Pool<N>>
	) -> BufferResult<usize> {
		self.drain(source, source.full_segment_count())
	}
	/// Drains a buffer by writing all its data into the sink, returning the number
	/// of bytes written.
	fn drain_all(
		&mut self,
		source: &mut Buffer<'d, N, impl Pool<N>>
	) -> BufferResult<usize> {
		self.drain(source, source.count())
	}
	/// Writes all buffered data to its final target.
	fn flush(&mut self) -> Result { Ok(()) }
}

pub trait SinkExt<'d, const N: usize, P: Pool<N>>: Sink<'d, N> + Sized {
	type Buffered: BufSink<'d, N, Pool = P>;

	fn buffered(self) -> Self::Buffered;
}

impl<'d, S: Sink<'d, SIZE>, P: Pool<SIZE>> SinkExt<'d, SIZE, P> for S {
	type Buffered = BufferedSink<'d, Self, P>;

	fn buffered(self) -> Self::Buffered {
		BufferedSink::new(self, Buffer::default())
	}
}

pub trait BufStream<'d, const N: usize = SIZE>: Stream<N> {
	type Pool: Pool<N>;

	/// Borrows the stream buffer.
	fn buf<'b>(&'b self) -> &'b Buffer<'d, N, Self::Pool>;
	/// Borrows the stream buffer mutably.
	fn buf_mut<'b>(&'b mut self) -> &'b mut Buffer<'d, N, Self::Pool>;
}

/// The result of a buffered *find* operation such as [`read_utf8_line`].
///
/// [`read_utf8_line`]: BufSource::read_utf8_line
#[derive(Copy, Clone)]
pub struct Utf8Match {
	/// The amount of bytes read.
	pub read_count: usize,
	/// Whether the pattern was found.
	pub found: bool
}

impl From<(usize, bool)> for Utf8Match {
	fn from((read_count, found): (usize, bool)) -> Self {
		Self { read_count, found }
	}
}

impl From<Utf8Match> for (usize, bool) {
	fn from(Utf8Match { read_count, found }: Utf8Match) -> Self {
		(read_count, found)
	}
}

pub trait BufSource<'d, const N: usize = SIZE>: BufStream<'d, N> + Source<'d, N> {
	/// Returns the number of bytes available for reading.
	fn available(&self) -> usize {
		self.buf().count()
	}
	/// Reads at most `count` bytes into the buffer, returning the whether enough
	/// bytes are available.
	/// To return an end-of-stream error, use [`require`] instead.
	///
	/// Note that a request returning `false` doesn't necessarily mean the stream
	/// has ended. To check if end-of-stream was reached, use [`is_eos`].
	///
	/// [`require`]: Self::require
	/// [`is_eos`]: Self::is_eos
	fn request(&mut self, count: usize) -> Result<bool>;
	/// Reads at least `count` bytes into the buffer, returning the available count
	/// if successful, or an end-of-stream error if not. For a softer version that
	/// returns an available count, use [`request`].
	///
	/// [`request`]: Self::request
	fn require(&mut self, count: usize) -> Result<()> {
		self.check_open(Read)?;
		if count > 0 && (self.is_eos() || !self.request(count)?) {
			return Err(StreamError::end_of_stream(count, Read))
		}
		Ok(())
	}

	/// Reads up to `count` bytes into `sink`, returning the number of bytes read.
	fn read(&mut self, sink: &mut impl Sink<'d, N>, mut count: usize) -> Result<usize> {
		self.request(count)?;
		count = count.min(self.available());
		sink.drain(self.buf_mut(), count)
			.context(Read)
	}

	/// Reads all available bytes into `sink`, returning the number of bytes read.
	fn read_all(&mut self, sink: &mut impl Sink<'d, N>) -> Result<usize> {
		sink.drain_all(self.buf_mut())
			.context(Read)
	}

	/// Removes up to `count` bytes, returning the number of bytes skipped.
	fn skip(&mut self, count: usize) -> Result<usize> {
		self.read_count_spec(count, Buffer::skip)
	}

	/// Reads bytes into a slice, returning the number of bytes read.
	fn read_slice(&mut self, buf: &mut [u8]) -> Result<usize> {
		let mut read = 0;
		self.read_count_spec(buf.len(), move |src, _| {
			read += src.read_slice(&mut buf[read..])?;
			Ok::<_, StreamError>(read)
		})
	}

	/// Reads the exact length of bytes into a slice, returning the number of bytes
	/// read if successful, or an end-of-stream error if the slice could not be filled.
	/// Bytes are not consumed if an end-of-stream error is returned.
	fn read_slice_exact(&mut self, buf: &mut [u8]) -> Result<usize> {
		self.require(buf.len())?;
		let read_count = self.buf_mut().read_slice_exact(buf)?;
		assert_eq!(read_count, buf.len());
		Ok(read_count)
	}

	/// Reads an array with a size of `T` bytes.
	fn read_array<const T: usize>(&mut self) -> Result<[u8; T]> {
		let mut array = [0; T];
		self.read_slice_exact(&mut array)?;
		Ok(array)
	}

	/// Reads a [`u8`].
	#[inline]
	fn read_u8(&mut self) -> Result<u8> { self.read_pod() }

	/// Reads an [`i8`].
	#[inline]
	fn read_i8(&mut self) -> Result<i8> {
		self.read_u8().map(|v| v as i8)
	}

	/// Reads a big-endian [`u16`].
	#[inline]
	fn read_u16(&mut self) -> Result<u16> { self.read_int() }

	/// Reads a little-endian [`u16`].
	#[inline]
	fn read_u16_le(&mut self) -> Result<u16> { self.read_int_le() }

	/// Reads a big-endian [`i16`].
	#[inline]
	fn read_i16(&mut self) -> Result<i16> { self.read_int() }

	/// Reads a little-endian [`i16`].
	#[inline]
	fn read_i16_le(&mut self) -> Result<i16> { self.read_int_le() }

	/// Reads a big-endian [`u32`].
	#[inline]
	fn read_u32(&mut self) -> Result<u32> { self.read_int() }

	/// Reads a little-endian [`u32`].
	#[inline]
	fn read_u32_le(&mut self) -> Result<u32> { self.read_int_le() }

	/// Reads a big-endian [`i32`].
	#[inline]
	fn read_i32(&mut self) -> Result<i32> { self.read_int() }

	/// Reads a little-endian [`i32`].
	#[inline]
	fn read_i32_le(&mut self) -> Result<i32> { self.read_int_le() }

	/// Reads a big-endian [`u64`].
	#[inline]
	fn read_u64(&mut self) -> Result<u64> { self.read_int() }

	/// Reads a little-endian [`u64`].
	#[inline]
	fn read_u64_le(&mut self) -> Result<u64> { self.read_int_le() }

	/// Reads a big-endian [`i64`].
	#[inline]
	fn read_i64(&mut self) -> Result<i64> { self.read_int() }

	/// Reads a little-endian [`i64`].
	#[inline]
	fn read_i64_le(&mut self) -> Result<i64> { self.read_int_le() }

	/// Reads a big-endian [`usize`]. To make streams consistent across platforms,
	/// [`usize`] is fixed to the size of [`u64`] regardless of the target platform.
	#[inline]
	fn read_usize(&mut self) -> Result<usize> {
		self.read_u64().map(|i| i as usize)
	}

	/// Reads a little-endian [`usize`]. To make streams consistent across platforms,
	/// [`usize`] is fixed to the size of [`u64`] regardless of the target platform.
	#[inline]
	fn read_usize_le(&mut self) -> Result<usize> {
		self.read_u64_le().map(|i| i as usize)
	}

	/// Reads a big-endian [`isize`]. To make streams consistent across platforms,
	/// [`isize`] is fixed to the size of [`i64`] regardless of the target platform.
	#[inline]
	fn read_isize(&mut self) -> Result<isize> {
		self.read_i64().map(|i| i as isize)
	}

	/// Reads a little-endian [`isize`]. To make streams consistent across platforms,
	/// [`isize`] is fixed to the size of [`i64`] regardless of the target platform.
	#[inline]
	fn read_isize_le(&mut self) -> Result<isize> {
		self.read_i64_le().map(|i| i as isize)
	}

	/// Reads a big-endian [`u128`].
	#[inline]
	fn read_u128(&mut self) -> Result<u128> { self.read_int() }

	/// Reads a little-endian [`u128`].
	#[inline]
	fn read_u128_le(&mut self) -> Result<u128> { self.read_int_le() }

	/// Reads a big-endian [`i128`].
	#[inline]
	fn read_i128(&mut self) -> Result<i128> { self.read_int() }

	/// Reads a little-endian [`i128`].
	#[inline]
	fn read_i128_le(&mut self) -> Result<i128> { self.read_int_le() }

	/// Reads a big-endian integer.
	#[inline]
	fn read_int<T: PrimInt + bytemuck::Pod>(&mut self) -> Result<T> {
		self.read_pod().map(T::to_be)
	}

	/// Reads a little-endian integer.
	#[inline]
	fn read_int_le<T: PrimInt + bytemuck::Pod>(&mut self) -> Result<T> {
		self.read_pod().map(T::to_le)
	}

	/// Reads an arbitrary [`Pod`] data type.
	///
	/// [`Pod`]: bytemuck::Pod
	#[inline]
	fn read_pod<T: bytemuck::Pod>(&mut self) -> Result<T> {
		let mut buf = T::zeroed();
		self.read_slice_exact(
			bytemuck::bytes_of_mut(&mut buf)
		)?;
		Ok(buf)
	}

	/// Reads up to `count` UTF-8 bytes into `buf`, returning the number of bytes
	/// read. If a decode error occurs, no data is consumed and `buf` will contain
	/// the last valid data.
	fn read_utf8(&mut self, buf: &mut String, count: usize) -> Result<usize> {
		self.read_count_spec(count, |src, count| src.read_utf8(buf, count))
	}

	/// Reads UTF-8 bytes into `buf` until end-of-stream, returning the number of
	/// bytes read. If a decode error occurs, no data is consumed and `buf` will
	/// contain the last valid data.
	fn read_utf8_to_end(&mut self, buf: &mut String) -> Result<usize> {
		self.read_spec(|src| src.read_utf8_to_end(buf).map(|n| (n, false)))
			.map(|(n, _)| n)
	}

	/// Reads UTF-8 bytes into `buf` until a line terminator, returning the number
	/// of bytes read and whether the line terminator was found. If a decode error
	/// occurs, no data is consumed and `buf` will contain the last valid data.
	fn read_utf8_line(&mut self, buf: &mut String) -> Result<Utf8Match> {
		self.read_spec(|src| src.read_utf8_line(buf))
			.map(Into::into)
	}

	/// Reads UTF-8 bytes into `buf` until and including the line terminator,
	/// returning the number of bytes read and whether the line terminator was
	/// found. If a decode error occurs, no data is consumed and `buf` will contain
	/// the last valid data.
	fn read_utf8_line_inclusive(&mut self, buf: &mut String) -> Result<Utf8Match> {
		self.read_spec(|src| src.read_utf8_line_inclusive(buf))
			.map(Into::into)
	}

	/// Reads buffered UTF-8 bytes into `buf` until the `terminator` pattern,
	/// returning the number of bytes read and whether the pattern was found. If a
	/// decode error occurs, no data is consumed and `buf` will contain the last
	/// valid data.
	///
	/// Note that, unlike methods like [`read_utf8_line`], this method only acts on
	/// buffered bytes. This is because the terminator pattern must be moved, so it
	/// cannot be used in a loop. This constraint may be lifted in the future.
	fn read_utf8_until(&mut self, buf: &mut String, terminator: impl Pattern) -> Result<Utf8Match> {
		self.buf_mut().read_utf8_until(buf, terminator)
	}

	/// Reads buffered UTF-8 bytes into `buf` until and including the `terminator`
	/// pattern, returning the number of bytes read and whether the pattern was
	/// found. If a decode error occurs, no data is consumed and `buf` will contain
	/// the last valid data.
	///
	/// Note that, unlike methods like [`read_utf8_line`], this method only acts on
	/// buffered bytes. This is because the terminator pattern must be moved, so it
	/// cannot be used in a loop. This constraint may be lifted in the future.
	fn read_utf8_until_inclusive(&mut self, buf: &mut String, terminator: impl Pattern) -> Result<Utf8Match> {
		self.buf_mut().read_utf8_until_inclusive(buf, terminator)
	}
}

trait BufSourceSpec<'d, const N: usize>: BufSource<'d, N> {
	fn read_spec<R: Into<(usize, bool)>>(
		&mut self,
		mut read: impl FnMut(&mut Buffer<'d, N, <Self as BufStream<'d, N>>::Pool>) -> Result<R>
	) -> Result<(usize, bool)> {
		let mut count = 0;
		loop {
			let (read, term) = read(self.buf_mut())?.into();
			count += read;
			if term { break Ok((count, term)) }
			self.request(self.buf().limit())?;
			if self.is_eos() || self.available() == 0 {
				break Ok((count, false))
			}
		}
	}

	fn read_count_spec<E>(
		&mut self,
		mut count: usize,
		mut read: impl FnMut(&mut Buffer<'d, N, <Self as BufStream<'d, N>>::Pool>, usize) -> Result<usize, E>
	) -> Result<usize> where StreamError: From<E> {
		let initial = count;
		while count > 0 {
			count -= read(self.buf_mut(), count)?;
			self.request(count)?;
			if self.is_eos() || self.available() == 0 {
				break
			}
		}
		Ok(initial - count)
	}
}

impl<'d, const N: usize, T: BufSource<'d, N> + ?Sized> BufSourceSpec<'d, N> for T { }

pub trait BufSink<'d, const N: usize = SIZE>: BufStream<'d, N> + Sink<'d, N> {
	/// Writes up to `count` bytes from `source`, returning the number of bytes written.
	fn write(&mut self, source: &mut impl Source<'d, N>, count: usize) -> Result<usize> {
		let count = source.fill(self.buf_mut(), count)
			  			  .context(Write)?;
		self.drain_buffered().context(Write)?;
		Ok(count)
	}

	/// Writes all available bytes from `source`, returning the number of bytes written.
	fn write_all(&mut self, source: &mut impl Source<'d, N>) -> Result<usize> {
		let count = source.fill_all(self.buf_mut())
						  .context(Write)?;
		self.drain_buffered().context(Write)?;
		Ok(count)
	}

	/// Writes all buffered data to the underlying sink, returning memory back to
	/// the pool. Similar to [`Sink::flush`], but draining doesn't propagate to
	/// the underlying sink.
	///
	/// This is called automatically when needed, and does not usually need to be
	/// called by the user. It may sometimes be useful if writing to the underlying
	/// buffer directly, when this method would otherwise be skipped.
	fn drain_all_buffered(&mut self) -> BufferResult;

	/// Writes full segments of buffered data to the underlying sink, returning
	/// memory back to the pool. Similar to [`Sink::flush`], but draining doesn't
	/// propagate to the underlying sink.
	///
	/// This is called automatically when needed, and does not usually need to be
	/// called by the user. It may sometimes be useful if writing to the underlying
	/// buffer directly, when this method would otherwise be skipped.
	fn drain_buffered(&mut self) -> BufferResult;

	/// Writes bytes from a slice, returning the number of bytes written.
	fn write_from_slice(&mut self, mut buf: &[u8]) -> Result<usize> {
		let mut count = 0;
		while !buf.is_empty() {
			let written = self.buf_mut().write_from_slice(buf).context(Write)?;
			buf = &buf[written..];
			count += written;
			self.drain_buffered().context(Write)?;
		}
		Ok(count)
	}

	/// Writes a [`u8`].
	#[inline]
	fn write_u8(&mut self, value: u8) -> Result { self.write_pod(value) }

	/// Writes an [`i8`].
	#[inline]
	fn write_i8(&mut self, value: i8) -> Result {
		self.write_u8(value as u8)
	}

	/// Writes a big-endian [`u16`].
	#[inline]
	fn write_u16(&mut self, value: u16) -> Result { self.write_int(value) }

	/// Writes a little-endian [`u16`].
	#[inline]
	fn write_u16_le(&mut self, value: u16) -> Result { self.write_int_le(value) }

	/// Writes a big-endian [`i16`].
	#[inline]
	fn write_i16(&mut self, value: i16) -> Result { self.write_int(value) }

	/// Writes a little-endian [`i16`].
	#[inline]
	fn write_i16_le(&mut self, value: i16) -> Result { self.write_int_le(value) }

	/// Writes a big-endian [`u32`].
	#[inline]
	fn write_u32(&mut self, value: u32) -> Result { self.write_int(value) }

	/// Writes a little-endian [`u32`].
	#[inline]
	fn write_u32_le(&mut self, value: u32) -> Result { self.write_int_le(value) }

	/// Writes a big-endian [`i32`].
	#[inline]
	fn write_i32(&mut self, value: i32) -> Result { self.write_int(value) }

	/// Writes a little-endian [`i32`].
	#[inline]
	fn write_i32_le(&mut self, value: i32) -> Result { self.write_int_le(value) }

	/// Writes a big-endian [`u64`].
	#[inline]
	fn write_u64(&mut self, value: u64) -> Result { self.write_int(value) }

	/// Writes a little-endian [`u64`].
	#[inline]
	fn write_u64_le(&mut self, value: u64) -> Result { self.write_int_le(value) }

	/// Writes a big-endian [`i64`].
	#[inline]
	fn write_i64(&mut self, value: i64) -> Result { self.write_int(value) }

	/// Writes a little-endian [`i64`].
	#[inline]
	fn write_i64_le(&mut self, value: i64) -> Result { self.write_int_le(value) }

	/// Writes a big-endian [`usize`]. To make streams consistent across platforms,
	/// [`usize`] is fixed to the size of [`u64`] regardless of the target platform.
	#[inline]
	fn write_usize(&mut self, value: usize) -> Result {
		self.write_u64(value as u64)
	}

	/// Writes a little-endian [`usize`]. To make streams consistent across platforms,
	/// [`usize`] is fixed to the size of [`u64`] regardless of the target platform.
	#[inline]
	fn write_usize_le(&mut self, value: usize) -> Result {
		self.write_u64_le(value as u64)
	}

	/// Writes a big-endian [`isize`]. To make streams consistent across platforms,
	/// [`isize`] is fixed to the size of [`i64`] regardless of the target platform.
	#[inline]
	fn write_isize(&mut self, value: isize) -> Result {
		self.write_i64(value as i64)
	}

	/// Writes a little-endian [`isize`]. To make streams consistent across platforms,
	/// [`isize`] is fixed to the size of [`i64`] regardless of the target platform.
	#[inline]
	fn write_isize_le(&mut self, value: isize) -> Result {
		self.write_i64_le(value as i64)
	}

	/// Writes a big-endian [`u128`].
	#[inline]
	fn write_u128(&mut self, value: u128) -> Result { self.write_int(value) }

	/// Writes a little-endian [`u128`].
	#[inline]
	fn write_u128_le(&mut self, value: u128) -> Result { self.write_int_le(value) }

	/// Writes a big-endian [`i128`].
	#[inline]
	fn write_i128(&mut self, value: i128) -> Result { self.write_int(value) }

	/// Writes a little-endian [`i128`].
	#[inline]
	fn write_i128_le(&mut self, value: i128) -> Result { self.write_int_le(value) }

	/// Writes a big-endian integer.
	#[inline]
	fn write_int<T: PrimInt + bytemuck::Pod>(&mut self, value: T) -> Result {
		self.write_pod(value.to_be())
	}

	/// Writes a little-endian integer.
	#[inline]
	fn write_int_le<T: PrimInt + bytemuck::Pod>(&mut self, value: T) -> Result {
		self.write_pod(value.to_le())
	}

	/// Writes an arbitrary [`Pod`] data type.
	///
	/// [`Pod`]: bytemuck::Pod
	#[inline]
	fn write_pod<T: bytemuck::Pod>(&mut self, value: T) -> Result {
		self.write_from_slice(bytemuck::bytes_of(&value))?;
		Ok(())
	}

	/// Writes a UTF-8 string.
	#[inline]
	fn write_utf8(&mut self, value: &str) -> Result<usize> {
		self.write_from_slice(value.as_bytes())
	}
}

trait BufSinkSpec<'d, const N: usize>: BufSink<'d, N> {
	fn write_spec(
		&mut self,
		mut write: impl FnMut(&mut Buffer<'d, N, <Self as BufStream<'d, N>>::Pool>, &mut bool) -> Result<usize>
	) -> Result<usize> {
		let mut count = 0;
		let mut term = false;
		while !term {
			count += write(self.buf_mut(), &mut term)?;
		}
		Ok(count)
	}

	fn write_count_spec(
		&mut self,
		mut count: usize,
		mut write: impl FnMut(&mut Buffer<'d, N, <Self as BufStream<'d, N>>::Pool>, usize) -> Result<usize>
	) -> Result<usize> {
		let initial = count;
		while count > 0 {
			count -= write(self.buf_mut(), count)?;
		}
		Ok(initial - count)
	}
}

impl<'d, const N: usize, T: BufSink<'d, N> + ?Sized> BufSinkSpec<'d, N> for T { }
