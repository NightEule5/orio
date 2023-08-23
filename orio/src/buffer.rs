// SPDX-License-Identifier: Apache-2.0

mod read;
mod write;

pub use read::*;
pub use write::*;

use std::cmp::min;
use std::{fmt, mem, slice};
use std::fmt::{Debug, Formatter};
use std::ops::{DerefMut, RangeBounds};
use simdutf8::compat::from_utf8;
use crate::pool::{SharedPool, DefaultPool, Pool};
use crate::segment::{Segment, SegRing};
use crate::{ByteStr, Context, expect, Result, ResultExt, SEGMENT_SIZE};
use crate::streams::{BufSink, BufStream, Seekable, SeekOffset};
use crate::streams::codec::Encode;
use Context::{BufClear, BufCompact, BufCopy, BufRead, StreamSeek};

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
	/// The fragmentation-compact threshold, the total size of fragmentation (gaps
	/// where segments have been partially read or written to) that triggers compacting.
	/// Defaults to `4096B`, half the segment size. With a value of `0`, the buffer
	/// always compacts.
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
	/// Presets the options to create a "lean" buffer, a buffer that always shares,
	/// returns all empty segments, always compacts, and always forks.
	pub fn lean() -> Self {
		Self {
			share_threshold: 0,
			retention_ratio: 0,
			compact_threshold: 0,
			is_fork_reluctant: true,
		}
	}
	
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

/// A dynamically-resizing byte buffer which borrows and returns pool memory as
/// needed.
pub struct Buffer<P: SharedPool = DefaultPool> {
	pool: P,
	segments: SegRing,
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
	/// Creates a new "lean" buffer. See [`BufferOptions::lean`] for details.
	pub fn lean() -> Self {
		Self::new_options(DefaultPool::get(), BufferOptions::lean())
	}
	
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
	pub fn from_utf8<T: AsRef<str>>(value: T) -> Result<Self> {
		let mut buf = Buffer::default();
		buf.write_utf8(value.as_ref())?;
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
	pub fn from_slice<T: AsRef<[u8]>>(value: T) -> Result<Self> {
		let mut buf = Buffer::default();
		buf.write_from_slice(value.as_ref())?;
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
			segments: SegRing::default(),
			options,
		}
	}

	pub fn count(&self) -> usize {
		self.segments.count()
	}

	pub fn is_empty(&self) -> bool { self.segments.is_empty() }

	pub fn is_not_empty(&self) -> bool { self.segments.is_not_empty() }

	fn lock_pool(pool: &P) -> Result<impl DerefMut<Target = impl Pool> + '_> {
		Ok(pool.lock()?)
	}

	pub fn clear(&mut self) -> Result {
		let Self { pool, segments, .. } = self;
		segments.clear(
			pool.lock()
				.context(BufClear)?
				.deref_mut()
		);
		Ok(())
	}

	/// Copies `byte_count` bytes into `sink`. Memory is either actually copied or
	/// shared for performance; the tradeoff between wasted space by sharing small
	/// segments and large, expensive mem-copies is managed by the implementation.
	pub fn copy_to(&self, sink: &mut Buffer<impl SharedPool>, mut byte_count: usize) -> Result {
		if byte_count == 0 { return Ok(()) }

		let Buffer { pool, segments: dst, .. } = sink;
		let share_threshold = sink.options.share_threshold;

		let result: Result = try {
			for seg in self.segments.iter() {
				if byte_count == 0 { break }

				let size = min(seg.len(), byte_count);
				if size > share_threshold {
					dst.push_back(seg.share(size));
					byte_count -= size;
				} else {
					dst.reserve(size, pool.lock()?.deref_mut());

					let mut off = 0;
					while let Some(mut target) = if off < size {
						dst.pop_back()
					} else {
						None
					} {
						off += seg.copy_all_into(&mut target, off);
						dst.push_back(target);
					}
				}
			}

			sink.tidy()?;
		};
		result.context(BufCopy)
	}

	/// Copies all byte into `sink`. Memory is either actually copied or shared for
	/// performance; the tradeoff between wasted space by sharing small segments and
	/// large, expensive mem-copies is managed by the implementation.
	#[inline]
	pub fn copy_all_to(&self, sink: &mut Buffer<impl SharedPool>) -> Result {
		self.copy_to(sink, usize::MAX)
	}

	fn tidy(&mut self) -> Result {
		if self.segments.frag() >= self.options.compact_threshold {
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
		let mut pool = pool.lock()?;

		let count = segments.len() * retention_ratio;
		let limit = segments.limit();

		if count < limit {
			let surplus = (limit - count) / SEGMENT_SIZE;
			segments.trim(surplus, pool.deref_mut());
		} else {
			let deficit = count - limit;
			pool.deref_mut().claim_size(segments, deficit);
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

		let Self { pool, segments, options, .. } = self;
		let BufferOptions { is_fork_reluctant, .. } = *options;

		let result: Result = try {
			let mut pool = pool.lock()?;

			segments.trim(usize::MAX, pool.deref_mut());

			*segments = (|| {
				let mut target = SegRing::default();
				let Some(mut prev) = segments.pop_front() else { return target };
				while let Some(mut curr) = segments.pop_front() {
					merge(&mut prev, &mut curr, pool.deref_mut(), is_fork_reluctant);

					if prev.is_full() {
						target.push_back(prev);
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

				target.push_back(prev);
				target
			})();
		};
		result.context(BufCompact)
	}

	/// Skips up to `byte_count` bytes.
	pub fn skip(&mut self, mut byte_count: usize) -> Result<usize> {
		let count = self.segments.read(|data| {
			let mut count = 0;
			for seg in data {
				if byte_count == 0 { break }

				let len = seg.len();
				seg.consume(byte_count);
				let skip_count = len - seg.len();
				byte_count -= skip_count;
				count += skip_count;
			}
			count
		});

		self.tidy().context(BufRead)?;

		Ok(count)
	}
	
	/// Skips all remaining bytes. Unlike [`clear`][], free space will be retained
	/// according to the [`retention_ratio`][].
	/// 
	/// [`clear`]: Self::clear
	/// [`retention_ratio`]: BufferOptions::retention_ratio
	pub fn skip_all(&mut self) -> Result<usize> {
		self.skip(self.count())
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
				let mut data = seg.data(start..end);
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
				return Some(seg.data(..)[pos])
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

	/// Inserts the contents of `other` at the start of this buffer. Used for seeking.
	pub(crate) fn prefix_with(&mut self, other: &mut Buffer<impl SharedPool>) -> Result {
		// Todo: shouldn't these segments go at the front of self?
		while let Some(seg) = other.segments.pop_front() {
			self.segments.push_back(seg)
		}
		
		let tidy_self = self.tidy();
		let tidy_other = other.tidy();
		tidy_self?;
		tidy_other
	}
}

impl<P: SharedPool> Drop for Buffer<P> {
	fn drop(&mut self) {
		let _ = self.close();
	}
}

impl<P: SharedPool> Seekable for Buffer<P> {
	/// Seeks to an `offset` in the buffer by skipping, returning a new *effective
	/// position*.
	///
	/// # Behavior
	///
	/// Since reading the buffer consumes bytes irreversibly, its seek position is
	/// always zero. Seeking back is impossible, and will just return `0`. Seeking
	/// returns the new position on other streams, but on buffers `0` would always
	/// be returned. For consistency with other streams, an *effective position*,
	/// one that would be returned for a stream at position `0` before seeking, is
	/// returned. Seeking forward of by offset from start or end returns the number
	/// of bytes skipped.
	fn seek(&mut self, offset: SeekOffset) -> Result<usize> {
		self.skip(
			offset.to_pos(0, self.count())
		).context(StreamSeek)
	}

	/// Returns the [`count`][].
	/// 
	/// [`count`]: Buffer::count
	fn seek_len(&mut self) -> Result<usize> { Ok(self.count()) }

	/// Returns `0`.
	fn seek_pos(&mut self) -> Result<usize> { Ok(0) }
}

impl<P: SharedPool> BufStream for Buffer<P> {
	fn buf(&self) -> &Self { self }
	fn buf_mut(&mut self) -> &mut Self { self }
}
