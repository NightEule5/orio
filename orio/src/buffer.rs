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

use std::cmp::min;
use std::{fmt, mem, slice};
use std::fmt::{Debug, Formatter};
use std::io::{Read, Write};
use std::ops::RangeBounds;
use simdutf8::compat::from_utf8;
use crate::pool::{DefaultPool, Pool, SharedPool};
use crate::segment::{Segment, SegmentRing};
use crate::{ByteStr, ByteString, expect, SEGMENT_SIZE};
use crate::streams::{BufSink, BufSource, BufStream, Error, OffsetUtf8Error, Result, Sink, Source};
use crate::streams::codec::Encode;
use crate::streams::OperationKind::BufRead;

#[derive(Copy, Clone, Debug)]
pub struct BufferOptions {
	/// The segment share threshold, the minimum size for a segment to be shared
	/// rather than its data moved to another segment. Defaults to `1024B`, one
	/// eighth the segment size. With a value is more than the segment size,
	/// segments are never shared.
	share_threshold: usize,
	/// The retention ratio, the number of bytes of free space to be retained for
	/// every filled segment. The buffer will collect or claim segments to maintain
	/// at least this ratio. Defaults to one segment, or `8192B`.
	retention_ratio: usize,
	/// The void-compact threshold, the total size of voids (gaps where segments
	/// have been partially read or written to) that triggers compacting. Defaults
	/// to `4096B`, half the segment size. With a value of `0`, the buffer always
	/// compacts.
	compact_threshold: usize,
	/// Whether the buffer is "reluctant" to cause a fork (copy) shared memory. If
	/// `false`, the buffer will always write to shared memory rather than claiming
	/// a new segment to write to. Defaults to `true`.
	is_fork_reluctant: bool,
}

impl Default for BufferOptions {
	fn default() -> Self {
		const SIZE: usize = SEGMENT_SIZE;

		Self {
			share_threshold: SIZE / 8,
			retention_ratio: SIZE,
			compact_threshold: SIZE / 2,
			is_fork_reluctant: true,
		}
	}
}

impl BufferOptions {
	/// Returns the segment share threshold.
	pub fn share_threshold(&self) -> usize { self.share_threshold }
	/// Returns the retention ratio.
	pub fn retention_ratio(&self) -> usize { self.retention_ratio }
	/// Returns the void-compact threshold.
	pub fn compact_threshold(&self) -> usize { self.compact_threshold }
	/// Returns whether the buffer will be "fork reluctant".
	pub fn is_fork_reluctant(&self) -> bool { self.is_fork_reluctant }

	/// Sets the segment share threshold.
	pub fn set_share_threshold(mut self, value: usize) -> Self {
		self.share_threshold = value;
		self
	}

	/// Sets the retention ratio.
	pub fn set_retention_ratio(mut self, value: usize) -> Self {
		self.retention_ratio = value;
		self
	}

	/// Sets the void-compact threshold.
	pub fn set_compact_threshold(mut self, value: usize) -> Self {
		self.compact_threshold = value;
		self
	}

	/// Sets whether the buffer should be "fork reluctant".
	pub fn set_fork_reluctant(mut self, value: bool) -> Self {
		self.is_fork_reluctant = value;
		self
	}
}

pub struct Buffer<P: SharedPool = DefaultPool> {
	pool: P,
	segments: SegmentRing,
	options: BufferOptions,
}

impl Default for Buffer {
	fn default() -> Self { Self::new(DefaultPool::get()) }
}

impl<P: SharedPool> Debug for Buffer<P> {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.debug_struct("Buffer")
			.field("segments", &self.segments)
			.finish_non_exhaustive()
	}
}

impl Buffer {
	/// Creates a new buffer with `value`. Shorthand for:
	///
	/// ```no_run
	/// use orio::Buffer;
	/// use orio::streams::BufSink;
	///
	/// let mut buf = Buffer::default();
	/// buf.write_from(value)?;
	/// ```
	pub fn from_encode(value: impl Encode) -> Result<Self> {
		let mut buf = Self::default();
		buf.write_from(value)?;
		Ok(buf)
	}
	
	/// Creates a new buffer with `value` in little-endian order. Shorthand for:
	///
	/// ```no_run
	/// use orio::Buffer;
	/// use orio::streams::BufSink;
	///
	/// let mut buf = Buffer::default();
	/// buf.write_from_le(value)?;
	/// ```
	pub fn from_encode_le(value: impl Encode) -> Result<Self> {
		let mut buf = Self::default();
		buf.write_from_le(value)?;
		Ok(buf)
	}

	/// Creates a new buffer containing `value` as UTF-8-encoded bytes. Shorthand
	/// for:
	///
	/// ```no_run
	/// use orio::Buffer;
	/// use orio::streams::BufSink;
	///
	/// let mut buf = Buffer::default();
	/// buf.write_utf8(value)?;
	/// ```
	pub fn from_utf8(value: &str) -> Result<Self> {
		let mut buf = Buffer::default();
		buf.write_utf8(value)?;
		Ok(buf)
	}

	/// Creates a new buffer with `value`. Shorthand for:
	///
	/// ```no_run
	/// use orio::Buffer;
	/// use orio::streams::BufSink;
	///
	/// let mut buf = Buffer::default();
	/// buf.write_from_slice(value)?;
	/// ```
	pub fn from_slice(value: &[u8]) -> Result<Self> {
		let mut buf = Buffer::default();
		buf.write_from_slice(value)?;
		Ok(buf)
	}
}

impl<P: SharedPool> Buffer<P> {
	pub fn new(pool: P) -> Self {
		Self::new_options(pool, BufferOptions::default())
	}

	pub fn new_options(pool: P, options: BufferOptions) -> Self {
		Self {
			pool,
			segments: SegmentRing::default(),
			options,
		}
	}

	pub fn count(&self) -> usize {
		self.segments.count()
	}

	pub fn clear(&mut self) -> Result {
		let mut pool = self.pool.lock().map_err(|err| Error::pool(err).with_op_buf_clear())?;
		self.segments
			.clear(&mut *pool);
		Ok(())
	}

	/// Copies `byte_count` bytes into `sink`. Memory is either actually copied or
	/// shared for performance; the tradeoff between wasted space by sharing small
	/// segments and large, expensive mem-copies is managed by the implementation.
	pub fn copy_to(&self, sink: &mut Buffer<impl SharedPool>, mut byte_count: usize) -> Result {
		if byte_count == 0 { return Ok(()) }

		let ref mut dst = sink.segments;
		let share_threshold = sink.options.share_threshold;

		for seg in self.segments.iter() {
			if byte_count == 0 { break }

			let size = min(seg.len(), byte_count);
			if size > share_threshold {
				dst.push_laden(seg.share(size));
				byte_count -= size;
			} else {
				let mut pool = sink.pool
								   .lock()
								   .map_err(|err| Error::pool(err).with_op_buf_copy())?;
				dst.reserve(size, &mut *pool);

				let mut off = 0;
				while let Some(mut target) = if off < size {
					dst.pop_back()
				} else {
					None
				} {
					off += seg.copy_all_into(&mut target, off);
					dst.push_laden(target);
				}
			}
		}

		sink.tidy().map_err(Error::with_op_buf_copy)
	}

	/// Copies all byte into `sink`. Memory is either actually copied or shared for
	/// performance; the tradeoff between wasted space by sharing small segments and
	/// large, expensive mem-copies is managed by the implementation.
	#[inline]
	pub fn copy_all_to(&self, sink: &mut Buffer<impl SharedPool>) -> Result {
		self.copy_to(sink, usize::MAX)
	}

	fn tidy(&mut self) -> Result {
		if self.segments.void(
			self.options.compact_threshold
		) {
			self.compact()?;
		}

		self.resize()
	}

	/// Resizes according to the retention ratio.
	fn resize(&mut self) -> Result {
		let Self {
			options: BufferOptions {
				retention_ratio,
				..
			},
			..
		} = *self;
		let Self { pool, segments, .. } = self;

		let count = segments.len() * retention_ratio;
		let limit = segments.limit();

		if count < limit {
			let surplus = (limit - count) / SEGMENT_SIZE;
			segments.trim(&mut *pool.lock()?, surplus);
		} else {
			let deficit = count - limit;
			pool.claim_size(segments, deficit)?;
		}

		Ok(())
	}

	/// Frees space by compacting data into partially filled segments. Called
	/// automatically on write via the [`compact_threshold`][]. If an error occurs
	/// in the pool, segments are reinserted before returning to ensure no data is
	/// lost.
	///
	/// [`compact_threshold`]: BufferOptions::compact_threshold
	pub fn compact(&mut self) -> Result {
		let Self { pool, segments, options, .. } = self;
		let BufferOptions { is_fork_reluctant, .. } = *options;

		let mut pool = pool.lock().map_err(|err|
			Error::pool(err).with_op_buf_compact()
		)?;

		fn merge(
			a: &mut Segment,
			b: &mut Segment,
			pool: &mut impl Pool,
			avoid_forks: bool,
		) {
			let is_a_shared = a.is_shared();
			let is_b_shared = b.is_shared();

			if a.off() == 0 && a.is_full() {
				if !avoid_forks || is_b_shared {
					b.shift();
				}
			} else {
				if !avoid_forks || is_a_shared {
					if (!avoid_forks || !is_b_shared) && SEGMENT_SIZE - b.len() >= a.len() {
						b.prefix_with(a);
						mem::swap(a, b);
					} else {
						let mut empty = pool.claim_one();
						a.shift();
						a.move_all_into(&mut empty, 0);
						mem::swap(a, &mut empty);
						pool.collect_one(empty);
						b.move_all_into(a, 0);
					}
				} else {
					a.shift();
					b.move_all_into(a, 0);
				}
			}
		}

		segments.trim(&mut *pool, usize::MAX);

		let mut compress = || {
			let mut target = SegmentRing::default();
			let Some(mut prev) = segments.pop_front() else { return target };
			while let Some(mut curr) = segments.pop_front() {
				merge(&mut prev, &mut curr, &mut *pool, is_fork_reluctant);

				if prev.is_full() {
					target.push_laden(prev);
					prev = curr;
				} else if curr.is_empty() {
					pool.collect_one(curr);
				} else {
					segments.push_front(curr);
				}
			}

			if prev.is_empty() {
				pool.collect_one(prev);
				return target
			}

			if !is_fork_reluctant || !prev.is_shared() {
				prev.shift();
			} else if prev.off() > 0 && !prev.is_full() {
				let mut base = pool.claim_one();
				prev.move_all_into(&mut base, 0);
				pool.collect_one(prev);
				prev = base;
			}

			target.push_laden(prev);
			target
		};

		*segments = compress();
		Ok(())
	}

	/// Skips up to `byte_count` bytes.
	pub fn skip(&mut self, byte_count: usize) -> Result<usize> {
		self.read_segments_exact(byte_count, |seg, mut count| {
			Ok(count)
		})
	}

	/// Returns the index of a `char` in the buffer, or `None` if not found.
	pub fn find_utf8_char(&self, char: char) -> Option<usize> {
		self.find_utf8_char_in(char, ..)
	}

	/// Returns the index of a `char` in the buffer within `range`, or `None` if
	/// not found. If invalid UTF-8 is encountered before a match is found, returns
	/// `None`.
	///
	/// # Panics
	///
	/// Panics if the end point of `range` is greater than [`count`][].
	///
	/// [`count`]: Self::count
	pub fn find_utf8_char_in<R: RangeBounds<usize>>(
		&self,
		char: char,
		range: R
	) -> Option<usize> {
		let range = slice::range(range, ..self.count());
		let mut start = range.start;
		let mut count = range.len();
		let mut offset = 0;

		for seg in self.segments.iter() {
			if count == 0 { break }

			// Seek
			if start >= seg.len() {
				start -= seg.len();
				offset += seg.len();
				continue
			} else {
				offset += start;
				start = 0;
			}

			let end = min(count, seg.len());

			let mut invalid = false;
			let str = {
				let mut data = &seg.data()[start..end];
				match from_utf8(data) {
					Ok(str) => str,
					Err(err) => {
						invalid = true;
						let end = err.valid_up_to();
						data = &data[..end];
						expect!(
							from_utf8(data),
							"data should be valid UTF-8 up to {end}"
						)
					}
				}
			};

			if let Some(hit) = str.find(char) {
				return Some(offset + hit)
			}

			if invalid { break }

			offset += seg.len();
			count = count.saturating_sub(seg.len());
		}

		None
	}

	/// Returns the byte at position `pos`, or `None` if `pos` is out of bounds.
	pub fn get(&self, mut pos: usize) -> Option<u8> {
		if pos > self.count() { return None }

		for seg in self.segments.iter() {
			if seg.len() < pos {
				pos -= seg.len();
			} else {
				return Some(seg.data()[pos])
			}
		}

		None
	}

	/// Borrows the contents of the buffer as a [`ByteStr`].
	pub fn as_byte_str(&self) -> ByteStr {
		let ref segments = self.segments;
		segments.into()
	}

	/// Clears and closed the buffer.
	pub fn close(&mut self) -> Result { self.clear() }

	fn read_segments_exact(
		&mut self,
		mut count: usize,
		mut consume: impl FnMut(&[u8], usize) -> Result<usize>,
	) -> Result<usize> {
		self.require(count)?;
		self.read_segments(count, |data| {
			let n = consume(data, count)?;
			count -= n;
			Ok(n)
		})
	}

	fn read_segments(
		&mut self,
		max_count: usize,
		mut consume: impl FnMut(&[u8]) -> Result<usize>,
	) -> Result<usize> {
		let mut read = 0;
		let ref mut segments = self.segments;
		while let Some(mut seg) = if read < max_count {
			segments.pop_front()
		} else {
			None
		} {
			let mut n = min(max_count - read, seg.len());
			n = match consume(&seg.data()[..n]).map_err(Error::with_op_buf_read) {
				Ok(n) => n,
				error => {
					segments.push_front(seg);
					return error
				}
			};

			read += n;
			seg.consume(n);

			if seg.is_empty() {
				segments.push_empty(seg);
			} else {
				segments.push_front(seg);
			}
		}

		self.tidy().map_err(Error::with_op_buf_read)?;
		Ok(read)
	}

	fn write_segments(
		&mut self,
		count: usize,
		mut write: impl FnMut(&mut [u8]) -> Result<usize>
	) -> Result<usize> {
		let Self { pool, segments, .. } = self;

		let mut written = 0;
		while written < count {
			let mut seg = if let Some(seg) = segments.pop_back() {
				seg
			} else {
				pool.claim_one().map_err(|err|
					Error::pool(err).with_op_buf_write()
				)?
			};

			let mut n = min(count - written, seg.limit());
			let slice = &mut seg.data_mut()[..n];

			if slice.is_empty() { continue }

			n = write(slice).map_err(Error::with_op_buf_write)?;
			written += n;
			seg.grow(n);

			if seg.is_empty() {
				segments.push_empty(seg);
			} else {
				segments.push_laden(seg);
			}
		}

		self.tidy().map_err(Error::with_op_buf_write)?;
		Ok(written)
	}

	pub(crate) fn write_std<R: Read>(&mut self, reader: &mut R, count: usize) -> Result<usize> {
		self.write_segments(count, |seg| Ok(reader.read(seg)?))
	}

	pub(crate) fn read_std<W: Write>(&mut self, writer: &mut W, count: usize) -> Result<usize> {
		self.read_segments(count, |seg| Ok(writer.write(seg)?))
	}
}

impl<P: SharedPool> Drop for Buffer<P> {
	fn drop(&mut self) {
		let _ = self.close();
	}
}

impl<P: SharedPool> Source for Buffer<P> {
	fn read(
		&mut self,
		sink: &mut Buffer<impl SharedPool>,
		mut count: usize
	) -> Result<usize> {
		let mut read = 0;
		count = count.clamp(0, self.count());

		let Self { segments, .. } = self;
		while count > 0 {
			let Some(seg) = segments.pop_front() else { break };
			let len = seg.len();

			if seg.len() <= count {
				// Move full segments to the sink.
				sink.segments.push_laden(seg);
			} else {
				// Share the last partial segment.
				sink.segments.push_laden(seg.share(count));
				segments.push_front(seg);
			}

			count -= len;
			read += len;
		}

		Ok(read)
	}

	#[inline]
	fn read_all(&mut self, sink: &mut Buffer<impl SharedPool>) -> Result<usize> {
		self.read(sink, self.count())
	}

	fn close_source(&mut self) -> Result { self.close() }
}

impl<P: SharedPool> Sink for Buffer<P> {
	fn write(&mut self, source: &mut Buffer<impl SharedPool>, count: usize) -> Result<usize> {
		source.read(self, count).map_err(Error::with_op_buf_write)
	}

	fn write_all(&mut self, source: &mut Buffer<impl SharedPool>) -> Result<usize> {
		BufSource::read_all(source, self).map_err(Error::with_op_buf_write)
	}

	fn close_sink(&mut self) -> Result { self.close() }
}

impl<P: SharedPool> BufStream for Buffer<P> {
	fn buf(&self) -> &Self { self }
	fn buf_mut(&mut self) -> &mut Self { self }
}

macro_rules! gen_int_reads {
    ($($s_name:ident$s_le_name:ident$s_ty:ident$u_name:ident$u_le_name:ident$u_ty:ident),+) => {
		$(
		fn $s_name(&mut self) -> Result<$s_ty> {
			self.$u_name().map(|n| n as $s_ty)
		}

		fn $s_le_name(&mut self) -> Result<$s_ty> {
			self.$u_le_name().map(|n| n as $s_ty)
		}

		fn $u_name(&mut self) -> Result<$u_ty> {
			Ok($u_ty::from_be_bytes(self.read_array()?))
		}

		fn $u_le_name(&mut self) -> Result<$u_ty> {
			Ok($u_ty::from_le_bytes(self.read_array()?))
		}
		)+
	};
}

impl<P: SharedPool> BufSource for Buffer<P> {
	fn request(&mut self, byte_count: usize) -> Result<bool> {
		Ok(self.count() >= byte_count)
	}

	fn read_all(&mut self, sink: &mut impl Sink) -> Result<usize> {
		sink.write_all(self)
			.map_err(Error::with_op_buf_read)
	}

	fn read_i8(&mut self) -> Result<i8> {
		self.read_u8().map(|n| n as i8)
	}

	fn read_u8(&mut self) -> Result<u8> {
		self.require(1)?;
		let mut seg = self.segments.pop_front().unwrap();
		let byte = seg.pop().unwrap();

		let Self { segments, .. } = self;

		if segments.is_empty() {
			segments.push_empty(seg);
		} else {
			segments.push_front(seg);
		}

		self.tidy().map_err(Error::with_op_buf_read)?;
		Ok(byte)
	}

	gen_int_reads! {
		read_i16   read_i16_le   i16   read_u16   read_u16_le u16,
		read_i32   read_i32_le   i32   read_u32   read_u32_le u32,
		read_i64   read_i64_le   i64   read_u64   read_u64_le u64,
		read_isize read_isize_le isize read_usize read_usize_le usize
	}

	fn read_byte_str(&mut self, byte_count: usize) -> Result<ByteString> {
		let len = min(byte_count, self.count());
		let mut dst = ByteString::with_capacity(len);

		self.read_segments(byte_count, |seg| {
			dst.extend_from_slice(seg);
			Ok(seg.len())
		})?;
		Ok(dst)
	}

	fn read_into_slice(&mut self, dst: &mut [u8]) -> Result<usize> {
		let n = min(dst.len(), self.count());
		self.read_into_slice_exact(&mut dst[..n])?;
		Ok(n)
	}

	fn read_into_slice_exact(&mut self, dst: &mut [u8]) -> Result {
		self.read_segments_exact(dst.len(), |seg, count| {
			let off = dst.len() - count;
			let end = min(dst.len(), seg.len());
			dst[off..end].copy_from_slice(seg);
			Ok(end - off)
		})?;
		Ok(())
	}

	fn read_utf8(&mut self, str: &mut String, byte_count: usize) -> Result<usize> {
		let mut off = 0;
		self.read_segments(byte_count, |seg| {
			let utf8 = from_utf8(seg).map_err(|err|
				Error::invalid_utf8(BufRead, OffsetUtf8Error::new(err, off))
			)?;

			off += seg.len();

			str.push_str(utf8);

			Ok(utf8.len())
		})
	}

	fn read_utf8_line(&mut self, str: &mut String) -> Result<bool> {
		if let Some(mut line_term) = self.find_utf8_char('\n') {
			let mut len = 1;

			// CRLF
			if line_term > 0 {
				if let Some(b'\r') = self.get(line_term - 1) {
					line_term -= 1;
					len += 1;
				}
			}

			self.read_utf8(str, line_term)?;
			self.skip(len)?;
			Ok(true)
		} else {
			// No line terminator found, read to end instead.
			self.read_all_utf8(str)?;
			Ok(false)
		}
	}

	fn read_utf8_into_slice(&mut self, str: &mut str) -> Result<usize> {
		let mut off = 0;
		self.read_segments(str.len(), |seg| {
			let utf8 = from_utf8(seg).map_err(|err|
				Error::invalid_utf8(BufRead, OffsetUtf8Error::new(err, off))
			)?;

			off += seg.len();

			let off = str.len() - seg.len();
			unsafe {
				str[off..].as_bytes_mut().copy_from_slice(utf8.as_bytes());
			}

			Ok(utf8.len())
		})
	}
}

macro_rules! gen_int_writes {
    ($($name:ident$le_name:ident$ty:ident),+) => {
		$(
		fn $name(&mut self, value: $ty) -> Result {
			self.write_from_slice(&value.to_be_bytes())
		}

		fn $le_name(&mut self, value: $ty) -> Result {
			self.write_from_slice(&value.to_le_bytes())
		}
		)+
	};
}

impl<P: SharedPool> BufSink for Buffer<P> {
	fn write_all(&mut self, source: &mut impl Source) -> Result<usize> {
		source.read_all(self)
			  .map_err(Error::with_op_buf_write)
	}

	fn write_i8(&mut self, value: i8) -> Result {
		self.write_u8(value as u8)
	}

	fn write_u8(&mut self, value: u8) -> Result {
		self.write_segments(1, |seg| {
			seg[0] = value;
			Ok(1)
		})?;
		Ok(())
	}

	gen_int_writes! {
		write_i16   write_i16_le   i16,
		write_u16   write_u16_le   u16,
		write_i32   write_i32_le   i32,
		write_u32   write_u32_le   u32,
		write_i64   write_i64_le   i64,
		write_u64   write_u64_le   u64,
		write_isize write_isize_le isize,
		write_usize write_usize_le usize
	}

	fn write_from_slice(&mut self, mut value: &[u8]) -> Result {
		while !value.is_empty() {
			self.write_segments(value.len(), |seg| {
				let n = min(seg.len(), value.len());
				seg.copy_from_slice(value);
				value = &value[n..];
				Ok(n)
			})?;
		}
		Ok(())
	}

	fn write_utf8(&mut self, value: &str) -> Result {
		self.write_from_slice(value.as_bytes())
	}
}
