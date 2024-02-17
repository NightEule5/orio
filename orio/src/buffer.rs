// SPDX-License-Identifier: Apache-2.0

mod read;
mod write;
mod options;

pub use options::*;

use std::cmp::min;
use std::{fmt, mem, slice};
use std::fmt::{Debug, Formatter};
use std::ops::{Index, IndexMut, Range, RangeBounds};
use all_asserts::{assert_ge, assert_le};
use itertools::Itertools;
use crate::pool::{DefaultPoolContainer, Pool, pool, PoolExt};
use crate::{BufferResult as Result, ByteStr, ResultContext, ResultSetContext, Seg, StreamResult};
use crate::BufferContext::{Copy, Reserve, Resize};
use crate::pattern::Pattern;
use crate::segment::RBuf;
use crate::streams::{BufSink, BufStream, Seekable, SeekOffset, Stream};
use crate::util::partial_utf8::*;

// Todo: track how much space is reserved to keep empty segments after resize-on-read.

pub type DefaultBuffer<'d> = Buffer<'d>;

/// A dynamically-resizing byte buffer which borrows and returns pool memory as
/// needed.
#[derive(Clone, Eq)]
pub struct Buffer<
	'd,
	const N: usize = 8192,
	P: Pool<N> = DefaultPoolContainer
> {
	data: RBuf<Seg<'d, N>>,
	pool: P,
	share_threshold: usize,
	borrow_threshold: usize,
	allocation: Allocate,
}

impl<const N: usize, P: Pool<N>> Default for Buffer<'_, N, P> {
	fn default() -> Self { BufferOptions::default().into() }
}

impl<const N: usize, P: Pool<N>> From<BufferOptions> for Buffer<'_, N, P> {
	fn from(options: BufferOptions) -> Self {
		Self::new(P::get(), options)
	}
}

impl<const N: usize, P: Pool<N>> From<P> for Buffer<'_, N, P> {
	fn from(value: P) -> Self {
		Self::new(value, BufferOptions::default())
	}
}

impl<const N: usize, P: Pool<N>> Debug for Buffer<'_, N, P> {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.debug_struct("Buffer")
			.field("data", &self.data)
			.field("share_threshold", &self.share_threshold)
			.field("borrow_threshold", &self.borrow_threshold)
			.field("allocation", &self.allocation)
			.finish_non_exhaustive()
	}
}

impl<'d> Buffer<'d> {
	/// Creates a new "lean" buffer. See [`BufferOptions::lean`] for details.
	pub fn lean() -> Self { BufferOptions::lean().into() }
	
	/// Creates a new buffer with `value` in big-endian order. Shorthand for:
	///
	/// ```ignore
	/// use orio::Buffer;
	/// use orio::streams::BufSink;
	///
	/// let mut buf = Buffer::default();
	/// buf.write_int(value)?;
	/// ```
	pub fn from_int<T: num_traits::PrimInt + bytemuck::Pod>(value: T) -> Result<Self> {
		let mut buf = Self::default();
		buf.write_int(value)?;
		Ok(buf)
	}
	
	/// Creates a new buffer with `value` in little-endian order. Shorthand for:
	///
	/// ```ignore
	/// use orio::Buffer;
	/// use orio::streams::BufSink;
	///
	/// let mut buf = Buffer::default();
	/// buf.write_pod(value.to_le())?;
	/// ```
	pub fn from_int_le<T: num_traits::PrimInt + bytemuck::Pod>(value: T) -> Result<Self> {
		let mut buf = Self::default();
		buf.write_int_le(value)?;
		Ok(buf)
	}

	/// Creates a new buffer from a UTF-8 string without copying its contents.
	/// Shorthand for:
	///
	/// ```ignore
	/// use orio::Buffer;
	/// use orio::streams::BufSink;
	///
	/// let mut buf = Buffer::default();
	/// buf.push_utf8(value)?;
	/// ```
	pub fn from_utf8<T: AsRef<str> + ?Sized>(value: &'d T) -> Self {
		let mut buf = Self::default();
		buf.push_utf8(value.as_ref());
		buf
	}

	/// Creates a new buffer from a slice without copying its contents. Shorthand
	/// for:
	///
	/// ```ignore
	/// use orio::Buffer;
	/// use orio::streams::BufSink;
	///
	/// let mut buf = Buffer::default();
	/// buf.push_slice(value)?;
	/// ```
	pub fn from_slice<T: AsRef<[u8]> + ?Sized>(value: &'d T) -> Self {
		let mut buf = Self::default();
		buf.push_slice(value.as_ref());
		buf
	}

	/// Creates a new buffer from a [byte string](ByteStr) without copying its
	/// contents.
	pub fn from_byte_str(value: ByteStr<'d>) -> Self {
		let mut buf = Self::default();
		buf.data = value.slices()
						.map(Seg::from_slice)
						.collect::<Vec<_>>()
						.into();
		buf
	}
}

impl<'d> FromIterator<u8> for Buffer<'d> {
	fn from_iter<T: IntoIterator<Item = u8>>(iter: T) -> Self {
		let iter = iter.into_iter();
		let capacity = match iter.size_hint() {
			(_, Some(upper)) => upper,
			(lower, None) => lower
		};
		let mut data = Vec::<Seg>::with_capacity(capacity);
		let pool = pool();

		fn is_full(data: &Vec<Seg>) -> bool {
			match data.last() {
				Some(seg) => seg.is_full(),
				None => true
			}
		}

		for byte in iter {
			if is_full(&data) {
				data.push(pool.claim_one().unwrap_or_default());
			}

			let seg = data.last_mut().expect("a segment should have been claimed");
			seg.push(byte).expect("claimed or created segment should be writable");
		}

		Self::new_buf(pool, data, BufferOptions::default())
	}
}

impl<'d, const N: usize, P: Pool<N>> Buffer<'d, N, P> {
	/// Creates a new buffer.
	pub const fn new(
		pool: P,
		BufferOptions {
			share_threshold,
			borrow_threshold,
			allocation,
		}: BufferOptions
	) -> Self {
		Self {
			data: RBuf::new(),
			pool,
			share_threshold,
			borrow_threshold,
			allocation,
		}
	}

	/// Creates a new buffer with capacity reserved for at least `capacity` bytes.
	pub fn with_capacity(capacity: usize) -> Self {
		let mut new = Self::default();
		new.claim_or_alloc(capacity);
		new
	}

	/// Creates a new buffer with `data` as its internal ring buffer.
	fn new_buf(
		pool: P,
		data: impl Into<RBuf<Seg<'d, N>>>,
		BufferOptions {
			share_threshold,
			borrow_threshold,
			allocation,
		}: BufferOptions
	) -> Self {
		Self {
			pool,
			data: data.into(),
			share_threshold,
			borrow_threshold,
			allocation,
		}
	}

	/// Returns the options used to create the buffer.
	pub fn options(&self) -> BufferOptions {
		BufferOptions {
			share_threshold: self.share_threshold,
			borrow_threshold: self.borrow_threshold,
			allocation: self.allocation,
		}
	}

	/// Returns the number of bytes that can be written to the buffer.
	pub fn limit(&self) -> usize { self.data.limit() }
	/// Returns the number of bytes in the buffer.
	pub fn count(&self) -> usize { self.data.count() }
	/// Returns the total number of bytes that can be written to the buffer.
	pub fn capacity(&self) -> usize { self.data.byte_capacity() }
	/// Returns `true` if the buffer is empty.
	pub fn is_empty(&self) -> bool { self.data.is_empty() }
	/// Returns `true` if the buffer is not empty.
	pub fn is_not_empty(&self) -> bool { !self.data.is_empty() }

	/// Consumes the buffer, creating a new one with identical contents, but with
	/// borrowed data written to owned segments. The new buffer is "detached" from
	/// the original buffer's lifetime, allowing it to outlive previously borrowed
	/// data. This is useful when creating a buffer from a slice (i.e. [`from_utf8`]
	/// or [`from_slice`]), where the borrowed data falls out of scope.
	///
	/// For example, this doesn't compile:
	/// ```compile_fail
	/// fn buf<'a, 'b>(data: &'b str) -> orio::Buffer<'a> {
	/// 	orio::Buffer::from_utf8(data) // lifetime may not live long enough
	/// }
	/// ```
	///
	/// To make the above compile, the buffer lifetime must be detached from the slice
	/// lifetime, by writing its data into owned segments:
	/// ```no_run
	/// use orio::streams::{BufSink, Result};
	/// fn buf<'a, 'b>(data: &'b str) -> Result<orio::Buffer<'a>> {
	/// 	let mut buf = orio::Buffer::default();
	/// 	buf.write_utf8(data)?;
	/// 	Ok(buf)
	/// }
	/// ```
	///
	/// Or, using `detached`:
	/// ```no_run
	/// fn buf<'a, 'b>(data: &'b str) -> orio::Buffer<'a> {
	/// 	orio::Buffer::from_utf8(data).detached()
	/// }
	/// ```
	///
	/// [`from_utf8`]: Buffer::from_utf8
	/// [`from_slice`]: Buffer::from_slice
	pub fn detached<'de>(mut self) -> Buffer<'de, N, P> {
		let data = {
			let Self { data, pool, .. } = &mut self;
			data.split_slice_segments();
			data.drain(data.len())
				.map(|seg| seg.detach(pool))
				.collect_vec()
		};
		Buffer::new_buf(self.pool.clone(), data, self.options())
	}

	/// Clears data from the buffer.
	pub fn clear(&mut self) {
		let Err(_) = self.pool.try_use(|mut pool| {
			use crate::pool::MutPool;

			// Take the internal ring buffer instead of draining. This should be
			// significantly faster.
			let segments = mem::take(&mut self.data).buf;

			(&mut pool).collect(segments);
		}) else { return };

		// Returning segments to the pool failed, clear and retain them instead.

		for seg in self.data.iter_mut() {
			seg.clear();
		}

		unsafe {
			// Safety: all segments have been cleared.
			self.data.set_len(0);
			self.data.set_count(0);
		}

		// Drop shared segments
		self.data.buf.retain(Seg::is_exclusive);
	}

	/// Reserves at least `count` bytes of additional memory in the buffer.
	pub fn reserve(&mut self, mut count: usize) -> Result {
		let Self { data, pool, allocation, .. } = self;

		let limit = data.limit();
		if count <= limit {
			return Ok(())
		}

		count -= limit;
		let seg_count = count.div_ceil(N);
		match allocation {
			Allocate::Always => {
				data.allocate(seg_count);
				Ok(())
			}
			Allocate::OnError => {
				self.claim_or_alloc(count);
				Ok(())
			}
			Allocate::Never => pool.claim_count(data, seg_count).context(Reserve)
		}
	}

	/// Grows the buffer by `count` bytes without writing data to it. The elements
	/// exposed at the end of the buffer are unsafe to read from until initialized.
	/// This is useful for lower-level modification of buffer contents, such as
	/// setting by index.
	///
	/// # Panics
	///
	/// Panics if `count` is greater than the buffer limit.
	pub unsafe fn grow(&mut self, count: usize) {
		assert_le!(count, self.limit());
		self.data.grow(count);
	}

	fn claim_or_alloc(&mut self, count: usize) {
		let Self { data, pool, .. } = self;
		let seg_count = count.div_ceil(N);
		if let Err(_) = pool.claim_count(data, seg_count) {
			data.allocate(seg_count);
		}
	}

	/// Returns empty segments to the pool after reading.
	fn resize(&mut self) -> Result {
		let Self { pool, data, .. } = self;
		pool.collect(data.drain_all_empty())
			.context(Resize)
	}

	/// Copies `count` bytes into `sink`. Memory is either actually copied or
	/// shared for performance; the tradeoff between wasted space by sharing small
	/// segments and large, expensive mem-copies is managed by the implementation.
	pub fn copy_to(&self, sink: &mut Buffer<'d, N, impl Pool<N>>, mut count: usize) -> Result {
		if count == 0 { return Ok(()) }
		let share_threshold = sink.share_threshold;

		let result: Result = try {
			for seg in self.data.iter() {
				if count == 0 { break }

				let size = min(seg.len(), count);
				let mut shared = seg.share(..size);
				if size > share_threshold {
					sink.data.push_back(shared);
					count -= size;
				} else {
					sink.reserve(size)?;

					while let Some(mut dst) = shared.is_not_empty().then(||
						sink.data
							.pop_back()
							.expect("sufficient space should have been reserved")
					) {
						dst.write_from(&mut shared);
						sink.data.push_back(dst);
					}
				}
			}
		};
		result.set_context(Copy)
	}

	/// Copies all bytes into `sink`. Memory is either actually copied or shared for
	/// performance; the tradeoff between wasted space by sharing small segments and
	/// large, expensive mem-copies is managed by the implementation.
	#[inline]
	pub fn copy_all_to(&self, sink: &mut Buffer<'d, N, impl Pool<N>>) -> Result {
		self.copy_to(sink, self.count())
	}

	/// Skips up to `count` bytes.
	pub fn skip(&mut self, count: usize) -> usize {
		if count == self.count() {
			self.clear();
			return count
		}

		let mut seg_count = 0;
		let mut skipped = 0;
		for (i, seg) in self.data.iter_mut().enumerate() {
			let remaining = count - skipped;
			if remaining > 0 {
				skipped += seg.consume(remaining);
			} else {
				seg_count = i;
				break
			}
		}

		let Err(_) = self.pool.try_use(|mut pool| {
			use crate::pool::MutPool;

			(&mut pool).collect(self.data.drain(seg_count));
		}) else { return skipped };

		// Returning segments to the pool failed, retain them instead.

		unsafe {
			// Safety: bytes have been skipped in these segments.
			self.data.dec_count(skipped);
		}
		self.data.rotate_back(seg_count);
		// Drop empty, shared segments
		self.data.buf.retain(|seg| seg.is_not_empty() || seg.is_exclusive());
		skipped
	}

	/// Finds `pattern` within `range` in the buffer, returning the matching byte
	/// range if found.
	pub fn find(&self, pattern: impl Pattern) -> Option<Range<usize>> {
		pattern.find_in(self.data.iter_slices())
	}

	/// Finds `pattern` within `range` in the buffer, returning the matching byte
	/// range if found.
	pub fn find_in_range<R: RangeBounds<usize>>(&self, pattern: impl Pattern, range: R) -> Option<Range<usize>> {
		let range = slice::range(range, ..self.count());
		pattern.find_in(self.data.iter_slices_in_range(range))
	}

	/// Returns the byte at position `pos`, or `None` if `pos` is out of bounds.
	pub fn get(&self, mut pos: usize) -> Option<&u8> {
		if pos > self.count() { return None }

		for seg in self.data.iter() {
			if seg.len() < pos {
				pos -= seg.len();
			} else {
				return Some(&seg[pos])
			}
		}

		None
	}

	/// Returns a mutable reference to the byte at position `pos`, or `None` if
	/// `pos` is out of bounds.
	///
	/// # Panics
	///
	/// Panics if the segment containing the position is shared.
	pub fn get_mut(&mut self, mut pos: usize) -> Option<&mut u8> {
		if pos > self.count() { return None }

		for seg in self.data.iter_mut() {
			if seg.len() < pos {
				pos -= seg.len();
			} else {
				return Some(&mut seg[pos])
			}
		}

		None
	}

	/// Returns a new buffer containing data shared with this buffer in `range`.
	/// Runs in at most `O(n)` time, where `n` is the number of segments.
	pub fn range<R: RangeBounds<usize>>(&self, range: R) -> Self {
		let range = slice::range(range, ..self.count());
		if range == (0..self.count()) {
			return self.clone()
		}

		let mut deque = Vec::with_capacity(self.data.len());
		deque.extend(self.data.share_range(range));
		Self {
			data: deque.into(),
			pool: self.pool.clone(),
			..*self
		}
	}

	/// Borrows the contents of the buffer as a [byte string](ByteStr).
	pub fn as_byte_str(&self) -> ByteStr {
		(&self.data).into()
	}

	/// Updates `hasher` with buffer data.
	#[cfg(feature = "hash")]
	pub fn hash(&self, hasher: &mut impl digest::Digest) {
		for slice in self.data.iter_slices() {
			hasher.update(slice);
		}
	}

	/// Updates `hasher` with buffer data within `range`.
	#[cfg(feature = "hash")]
	pub fn hash_in_range<R: RangeBounds<usize>>(
		&self,
		range: R,
		hasher: &mut impl digest::Digest
	) {
		for slice in self.data.iter_slices_in_range(range) {
			hasher.update(slice);
		}
	}
}

impl<'d, const N: usize, P: Pool<N>> Buffer<'d, N, P> {
	pub(crate) fn full_segment_count(&self) -> usize {
		let len = self.data.back_index().filter(|&i| {
			self.data[i].is_full()
		}).unwrap_or(self.data.len());

		self.data.buf.range(..len).map(|seg|
			seg.len()
		).sum()
	}

	/// Swaps internal buffers.
	pub(crate) fn swap(&mut self, Buffer { data, .. }: &mut Buffer<'d, N, impl Pool<N>>) {
		mem::swap(&mut self.data, data);
	}

	/// Takes the internal buffer, leaving a new one in its place.
	pub(crate) fn take_buf(&mut self) -> RBuf<Seg<'d, N>> {
		mem::take(&mut self.data)
	}

	/// Returns a new buffer containing this buffer's internal data, leaving an
	/// empty buffer in its place.
	pub(crate) fn take(&mut self) -> Self {
		Self {
			data: self.take_buf(),
			pool: self.pool.clone(),
			..*self
		}
	}
}

impl<const N: usize, P: Pool<N>> Drop for Buffer<'_, N, P> {
	fn drop(&mut self) {
		let _ = self.close();
	}
}

impl<const N: usize, P: Pool<N>> Stream<N> for Buffer<'_, N, P> {
	/// Returns whether the buffer is closed; always returns `false`.
	#[inline(always)]
	fn is_closed(&self) -> bool { false }
	/// Clears the buffer.
	#[inline]
	fn close(&mut self) -> StreamResult {
		self.clear();
		Ok(())
	}
}

impl<'d, const N: usize, P: Pool<N>> BufStream<'d, N> for Buffer<'d, N, P> {
	type Pool = P;

	fn buf<'b>(&'b self) -> &'b Buffer<'d, N, P> { self }
	fn buf_mut<'b>(&'b mut self) -> &'b mut Buffer<'d, N, P> { self }
}

impl<const N: usize, P: Pool<N>> Seekable for Buffer<'_, N, P> {
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
	fn seek(&mut self, offset: SeekOffset) -> StreamResult<usize> {
		Ok(
			self.skip(offset.to_pos(0, self.count()))
		)
	}

	/// Returns the [`count`][].
	/// 
	/// [`count`]: Buffer::count
	fn seek_len(&mut self) -> StreamResult<usize> { Ok(self.count()) }

	/// Returns `0`.
	fn seek_pos(&mut self) -> StreamResult<usize> { Ok(0) }
}

impl<const N: usize, Pa: Pool<N>, const O: usize, Pb: Pool<O>> PartialEq<Buffer<'_, O, Pb>> for Buffer<'_, N, Pa> {
	fn eq(&self, other: &Buffer<'_, O, Pb>) -> bool {
		self.data.iter().eq(other.data.iter())
	}
}

impl<const N: usize, P: Pool<N>> PartialEq<[u8]> for Buffer<'_, N, P> {
	fn eq(&self, mut other: &[u8]) -> bool {
		if self.count() != other.len() {
			return false
		}

		self.data.iter().all(move |seg| {
			assert_ge!(other.len(), seg.len());
			let cur = &other[..seg.len()];
			other = &other[seg.len()..];
			seg == cur
		})
	}
}

impl<const N: usize, P: Pool<N>, T: AsRef<[u8]>> PartialEq<T> for Buffer<'_, N, P> {
	fn eq(&self, other: &T) -> bool {
		self == other.as_ref()
	}
}

impl<const N: usize, P: Pool<N>> Index<usize> for Buffer<'_, N, P> {
	type Output = u8;

	#[inline]
	fn index(&self, index: usize) -> &u8 {
		let Some(byte) = self.get(index) else {
			index_out_of_bounds(index, self.count())
		};
		byte
	}
}

impl<const N: usize, P: Pool<N>> IndexMut<usize> for Buffer<'_, N, P> {
	#[inline]
	fn index_mut(&mut self, index: usize) -> &mut u8 {
		let count = self.count();
		let Some(byte) = self.get_mut(index) else {
			index_out_of_bounds(index, count)
		};
		byte
	}
}

#[cold]
#[inline(never)]
#[track_caller]
fn index_out_of_bounds(index: usize, count: usize) -> ! {
	panic!("byte index {index} is out of bounds on buffer of byte count {count}")
}
