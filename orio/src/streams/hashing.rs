// SPDX-License-Identifier: Apache-2.0

#![cfg(feature = "hash")]

use std::marker::PhantomData;
use std::mem;
use std::ops::Range;
use digest::{Digest, FixedOutputReset};
use crate::{Buffer, BufferResult, ByteString, ResultContext};
use crate::pattern::Pattern;
use crate::pool::Pool;
use crate::StreamContext::Read;
use super::{Sink, Source, Stream, Result, BufSource, BufStream, Utf8Match, BufSink};

mod sealed {
	use super::{Digest, Source, Sink};

	pub trait HashStream { }
	impl<'d, H, S, const N: usize> HashStream for super::HashSource<'d, H, S, N>
	where H: Digest,
		  S: Source<'d, N> { }
	impl<'d, H, S, const N: usize> HashStream for super::HashSink<'d, H, S, N>
	where H: Digest,
		  S: Sink<'d, N> { }
}

/// A [`Source`] that hashes data read from its inner source.
pub struct HashSource<'d, H, S: Source<'d, N>, const N: usize> {
	hasher: H,
	source: Option<S>,
	__data: PhantomData<&'d ()>
}

/// A [`Sink`] that hashes data written to its inner sink.
pub struct HashSink<'d, H, S: Sink<'d, N>, const N: usize> {
	hasher: H,
	sink: Option<S>,
	__data: PhantomData<&'d ()>
}

pub trait HashStream<H: Digest, S>: sealed::HashStream {
	fn new(hasher: H, source: S) -> Self;

	/// Returns a reference to the hasher.
	fn hasher(&self) -> &H;
	/// Returns a mutable reference to the hasher.
	fn hasher_mut(&mut self) -> &mut H;

	/// Returns a clone of the current hash.
	fn hash(&self) -> ByteString where H: Clone {
		self.hasher()
			.clone()
			.finalize()
			.to_vec()
			.into()
	}

	/// Takes and returns the current hash, resetting the hash function state.
	fn take_hash(&mut self) -> ByteString where H: FixedOutputReset {
		self.hasher_mut()
			.finalize_reset()
			.to_vec()
			.into()
	}

	fn into_inner(self) -> S;
}

impl<'d, H: Digest, S: Source<'d, N>, const N: usize> HashStream<H, S> for HashSource<'d, H, S, N> {
	/// Creates a new hash source, hashing data read from `source` with `hasher`.
	#[inline]
	fn new(hasher: H, source: S) -> Self {
		Self {
			hasher,
			source: Some(source),
			__data: PhantomData
		}
	}

	#[inline]
	fn hasher(&self) -> &H { &self.hasher }
	#[inline]
	fn hasher_mut(&mut self) -> &mut H { &mut self.hasher }

	/// Consumes the hash source, returning the inner source.
	fn into_inner(mut self) -> S {
		unsafe {
			// Safety: option will only be None if this method was already called,
			// which is impossible because we consume self.
			self.source.take().unwrap_unchecked()
		}
	}
}

impl<'d, H, S: Source<'d, N>, const N: usize> HashSource<'d, H, S, N> {
	/// Returns a reference to the inner source.
	pub fn source(&self) -> &S {
		unsafe {
			// Safety: option will only be None if into_inner is called, but this
			// consumes and drops self, making it impossible to ever have a
			// reference (except on drop, which is guarded).
			self.source.as_ref().unwrap_unchecked()
		}
	}

	/// Returns a mutable reference to the inner source, bypassing hashing.
	pub fn source_mut(&mut self) -> &mut S {
		unsafe {
			// Safety: option will only be None if into_inner is called, but this
			// consumes and drops self, making it impossible to ever have a
			// reference (except on drop, which is guarded).
			self.source.as_mut().unwrap_unchecked()
		}
	}
}

impl<'d, H: Digest, S: Source<'d, N>, const N: usize> HashSource<'d, H, S, N> {
	fn fill_with<P: Pool<N>>(
		&mut self,
		sink: &mut Buffer<'d, N, P>,
		fill: impl FnOnce(&mut S, &mut Buffer<'d, N, P>) -> BufferResult<usize>
	) -> BufferResult<usize> {
		let start = sink.count();
		let count = fill(self.source_mut(), sink)?;
		let range = start..start + count;
		sink.hash_in_range(range, &mut self.hasher);
		Ok(count)
	}
}

impl<'d, H: Digest, S: Sink<'d, N>, const N: usize> HashStream<H, S> for HashSink<'d, H, S, N> {
	/// Creates a new hash sink, hashing data written to `sink` with `hasher`.
	#[inline]
	fn new(hasher: H, sink: S) -> Self {
		Self {
			hasher,
			sink: Some(sink),
			__data: PhantomData
		}
	}

	#[inline]
	fn hasher(&self) -> &H { &self.hasher }
	#[inline]
	fn hasher_mut(&mut self) -> &mut H { &mut self.hasher }

	/// Consumes the hash source, returning the inner sink.
	fn into_inner(mut self) -> S {
		unsafe {
			// Safety: option will only be None if this method was already called,
			// which is impossible because we consume self.
			self.sink.take().unwrap_unchecked()
		}
	}
}

impl<'d, H, S: Sink<'d, N>, const N: usize> HashSink<'d, H, S, N> {
	/// Returns a reference to the inner sink.
	pub fn sink(&self) -> &S {
		unsafe {
			// Safety: option will only be None if into_inner is called, but this
			// consumes and drops self, making it impossible to ever have a
			// reference (except on drop, which is guarded).
			self.sink.as_ref().unwrap_unchecked()
		}
	}

	/// Returns a mutable reference to the inner sink, bypassing hashing.
	pub fn sink_mut(&mut self) -> &mut S {
		unsafe {
			// Safety: option will only be None if into_inner is called, but this
			// consumes and drops self, making it impossible to ever have a
			// reference (except on drop, which is guarded).
			self.sink.as_mut().unwrap_unchecked()
		}
	}
}

impl<'d, H, S: Source<'d, N>, const N: usize> Stream<N> for HashSource<'d, H, S, N> {
	/// Returns whether the inner source is closed.
	fn is_closed(&self) -> bool {
		self.source().is_closed()
	}

	/// Closes the inner source. The current hash is left intact, so [`hash`] and
	/// [`take_hash`] work after this operation.
	///
	/// [`hash`]: Self::hash
	/// [`take_hash`]: Self::take_hash
	fn close(&mut self) -> Result {
		self.source_mut().close()
	}
}

impl<'d, H: Digest, S: Source<'d, N>, const N: usize> Source<'d, N> for HashSource<'d, H, S, N> {
	fn is_eos(&self) -> bool {
		self.source().is_eos()
	}

	fn fill(&mut self, sink: &mut Buffer<'d, N, impl Pool<N>>, count: usize) -> BufferResult<usize> {
		self.fill_with(sink, |source, sink| source.fill(sink, count))
	}

	fn fill_free(&mut self, sink: &mut Buffer<'d, N, impl Pool<N>>) -> BufferResult<usize> {
		self.fill_with(sink, Source::fill_free)
	}

	fn fill_all(&mut self, sink: &mut Buffer<'d, N, impl Pool<N>>) -> BufferResult<usize> {
		self.fill_with(sink, Source::fill_all)
	}
}

impl<'d, H, S: BufSource<'d, N>, const N: usize> BufStream<'d, N> for HashSource<'d, H, S, N> {
	type Pool = S::Pool;

	fn buf<'b>(&'b self) -> &'b Buffer<'d, N, Self::Pool> {
		self.source().buf()
	}

	fn buf_mut<'b>(&'b mut self) -> &'b mut Buffer<'d, N, Self::Pool> {
		self.source_mut().buf_mut()
	}
}

impl<'d, H: Clone + Default + Digest, S: BufSource<'d, N>, const N: usize> BufSource<'d, N> for HashSource<'d, H, S, N> {
	fn available(&self) -> usize {
		self.source().available()
	}

	fn request(&mut self, count: usize) -> Result<bool> {
		self.source_mut().request(count)
	}

	fn read(&mut self, sink: &mut impl Sink<'d, N>, mut count: usize) -> Result<usize> {
		self.request(count)?;
		count = count.min(self.available());
		let mut sink = HashSink::new(self.hasher.clone(), sink);
		let result = sink.drain(self.buf_mut(), count);
		self.hasher = mem::take(&mut sink.hasher);
		let _ = sink.into_inner(); // Prevent the sink from closing on drop.
		result.context(Read)
	}

	fn read_all(&mut self, sink: &mut impl Sink<'d, N>) -> Result<usize> {
		let mut sink = HashSink::new(self.hasher.clone(), sink);
		let result = sink.drain_all(self.buf_mut());
		self.hasher = mem::take(&mut sink.hasher);
		let _ = sink.into_inner(); // Prevent the sink from closing on drop.
		result.context(Read)
	}

	fn read_slice<'s>(&mut self, buf: &'s mut [u8]) -> Result<&'s [u8]> {
		let slice = self.source_mut().read_slice(buf)?;
		self.hasher.update(&slice);
		Ok(slice)
	}

	fn read_slice_exact<'s>(&mut self, buf: &'s mut [u8]) -> Result<&'s [u8]> {
		let slice = self.source_mut().read_slice_exact(buf)?;
		self.hasher.update(&slice);
		Ok(slice)
	}

	fn read_utf8<'s>(&mut self, buf: &'s mut String, count: usize) -> Result<&'s str> {
		let str = self.source_mut().read_utf8(buf, count)?;
		self.hasher.update(str);
		Ok(str)
	}

	fn read_utf8_to_end<'s>(&mut self, buf: &'s mut String) -> Result<&'s str> {
		let str = self.source_mut().read_utf8_to_end(buf)?;
		self.hasher.update(str);
		Ok(str)
	}

	fn read_utf8_line(&mut self, buf: &mut String) -> Result<Utf8Match> {
		let start = buf.len();
		let r#match = self.source_mut().read_utf8_line(buf)?;
		let range = start..start + r#match.read_count.min(buf.len());
		self.hasher.update(&buf[range]);
		Ok(r#match)
	}

	fn read_utf8_line_inclusive(&mut self, buf: &mut String) -> Result<Utf8Match> {
		let start = buf.len();
		let r#match = self.source_mut().read_utf8_line_inclusive(buf)?;
		let range = start..start + r#match.read_count.min(buf.len());
		self.hasher.update(&buf[range]);
		Ok(r#match)
	}

	fn read_utf8_until(&mut self, buf: &mut String, terminator: impl Pattern) -> Result<Utf8Match> {
		let start = buf.len();
		let r#match = self.source_mut().read_utf8_until(buf, terminator)?;
		let range = start..start + r#match.read_count.min(buf.len());
		self.hasher.update(&buf[range]);
		Ok(r#match)
	}

	fn read_utf8_until_inclusive(&mut self, buf: &mut String, terminator: impl Pattern) -> Result<Utf8Match> {
		let start = buf.len();
		let r#match = self.source_mut().read_utf8_until_inclusive(buf, terminator)?;
		let range = start..start + r#match.read_count.min(buf.len());
		self.hasher.update(&buf[range]);
		Ok(r#match)
	}
}

impl<'d, H, S: Source<'d, N>, const N: usize> Drop for HashSource<'d, H, S, N> {
	fn drop(&mut self) {
		// If into_inner was called, closing would cause a seg fault.
		if self.source.is_some() {
			let _ = self.close();
		}
	}
}

impl<'d, H, S: Sink<'d, N>, const N: usize> Stream<N> for HashSink<'d, H, S, N> {
	/// Returns whether the inner sink is closed.
	fn is_closed(&self) -> bool {
		self.sink().is_closed()
	}

	/// Closes the inner sink. The current hash is left intact, so [`hash`] and
	/// [`take_hash`] work after this operation.
	///
	/// [`hash`]: Self::hash
	/// [`take_hash`]: Self::take_hash
	fn close(&mut self) -> Result {
		self.sink_mut().close()
	}
}

impl<'d, H: Digest, S: Sink<'d, N>, const N: usize> Sink<'d, N> for HashSink<'d, H, S, N> {
	fn drain(&mut self, source: &mut Buffer<'d, N, impl Pool<N>>, count: usize) -> BufferResult<usize> {
		let mut clone = source.range(..count.min(source.count()));
		// Give sink the cloned buffer instead of the full source buffer, in case
		// sink violates the "up to" contract and reads more than the specified
		// count. In this scenario, if the clone was hashed instead, this wouldn't
		// hash all the data written to the sink.
		let count = self.sink_mut().drain(&mut clone, count)?;
		source.hash_in_range(..count, &mut self.hasher);
		source.skip(count);
		Ok(count)
	}

	#[inline]
	fn drain_full(&mut self, source: &mut Buffer<'d, N, impl Pool<N>>) -> BufferResult<usize> {
		self.drain(source, source.full_segment_count())
	}

	#[inline]
	fn drain_all(&mut self, source: &mut Buffer<'d, N, impl Pool<N>>) -> BufferResult<usize> {
		self.drain(source, source.count())
	}

	fn flush(&mut self) -> Result {
		self.sink_mut().flush()
	}
}

impl<'d, H: Digest, S: BufSink<'d, N>, const N: usize> HashSink<'d, H, S, N> {
	fn hash_buf(&mut self, range: Range<usize>) {
		let Self { sink, hasher, .. } = self;
		// Can't use Self::buf because it would borrow self immutably, but we also
		// need to borrow self.hasher mutably. The only way around this is borrowing
		// via fields.
		let sink = unsafe {
			// Safety: see Self::sink.
			sink.as_ref().unwrap_unchecked()
		};
		sink.buf().hash_in_range(range, hasher);
	}
}

impl<'d, H, S: BufSink<'d, N>, const N: usize> BufStream<'d, N> for HashSink<'d, H, S, N> {
	type Pool = S::Pool;

	fn buf<'b>(&'b self) -> &'b Buffer<'d, N, Self::Pool> {
		self.sink().buf()
	}

	fn buf_mut<'b>(&'b mut self) -> &'b mut Buffer<'d, N, Self::Pool> {
		self.sink_mut().buf_mut()
	}
}

impl<'d, H: Digest, S: BufSink<'d, N>, const N: usize> BufSink<'d, N> for HashSink<'d, H, S, N> {
	fn write(&mut self, source: &mut impl Source<'d, N>, count: usize) -> Result<usize> {
		let start = self.buf().count();
		let count = source.fill(self.buf_mut(), count)?;
		self.hash_buf(start..start + count);
		Ok(count)
	}

	fn write_all(&mut self, source: &mut impl Source<'d, N>) -> Result<usize> {
		let start = self.buf().count();
		let count = source.fill_all(self.buf_mut())?;
		self.hash_buf(start..start + count);
		Ok(count)
	}

	fn drain_all_buffered(&mut self) -> BufferResult {
		// Todo: for now, we detach the buffer from the self reference by "taking"
		//  its internal buffer temporarily, to workaround needing two mutable refs
		//  to self (self.drain_all and self.buf_mut). Is there another way to do
		//  this? While this avoids heap allocation that would come with cloning,
		//  it's still quite awkward.
		let ref mut buf = self.buf_mut().take();
		self.drain_all(buf)?;
		self.buf_mut().swap(buf);
		Ok(())
	}

	fn drain_buffered(&mut self) -> BufferResult {
		let ref mut buf = self.buf_mut().take();
		self.drain_full(buf)?;
		self.buf_mut().swap(buf);
		Ok(())
	}

	fn write_from_slice(&mut self, buf: &[u8]) -> Result<usize> {
		let count = self.sink_mut().write_from_slice(buf)?;
		let slice = &buf[..count.min(buf.len())];
		self.hasher.update(slice);
		Ok(count)
	}
}

impl<'d, H, S: Sink<'d, N>, const N: usize> Drop for HashSink<'d, H, S, N> {
	fn drop(&mut self) {
		// If into_inner was called, closing would cause a seg fault.
		if self.sink.is_some() {
			let _ = self.close();
		}
	}
}

macro_rules! hash {
    ($sec:tt$feature:literal$module:ident
	$($size_name:literal$size_fn:ident$size_hasher:ident)+
	) => {
		$(
		hash! {
			$sec
			$module::$size_hasher
			$feature
			$size_name
			$size_fn
		}
		)+
	};
    (secure $module:ident::$ty:ident$feature:literal$name:literal$method:ident) => {
		#[cfg(feature = $feature)]
		impl<'d, S: Source<'d, N>, const N: usize> HashSource<'d, $module::$ty, S, N> {
			/// Creates a new hash source, hashing data read from `source` with
			#[doc = concat!($name, ".")]
			/// There are no known attacks on this hash function; it can be considered
			/// suitable for cryptography.
			#[inline]
			pub fn $method(source: S) -> Self {
				source.into()
			}
		}

		#[cfg(feature = $feature)]
		impl<'d, S: Sink<'d, N>, const N: usize> HashSink<'d, $module::$ty, S, N> {
			/// Creates a new hash sink, hashing data written to `sink` with
			#[doc = concat!($name, ".")]
			/// There are no known attacks on this hash function; it can be considered
			/// suitable for cryptography.
			#[inline]
			pub fn $method(sink: S) -> Self {
				sink.into()
			}
		}
	};
    (broken $module:ident::$ty:ident$feature:literal$name:literal$method:ident) => {
		#[cfg(feature = $feature)]
		impl<'d, S: Source<'d, N>, const N: usize> HashSource<'d, $module::$ty, S, N> {
			/// Creates a new hash source, hashing data read from `source` with
			#[doc = concat!($name, ".")]
			/// This hash function has been broken; its use in cryptography is
			/// ***not*** secure. Use for checksums only.
			#[inline]
			pub fn $method(source: S) -> Self {
				source.into()
			}
		}

		#[cfg(feature = $feature)]
		impl<'d, S: Sink<'d, N>, const N: usize> HashSink<'d, $module::$ty, S, N> {
			/// Creates a new hash sink, hashing data written to `sink` with
			#[doc = concat!($name, ".")]
			/// This hash function has been broken; its use in cryptography is
			/// ***not*** secure. Use for checksums only.
			#[inline]
			pub fn $method(sink: S) -> Self {
				sink.into()
			}
		}
	};
}

hash! {
	secure "groestl" groestl
	"Grøstl-224" groestl224 Groestl224
	"Grøstl-256" groestl256 Groestl256
	"Grøstl-384" groestl384 Groestl384
	"Grøstl-512" groestl512 Groestl512
}

hash! {
	broken "md5" md5
	"MD5" md5 Md5
}

hash! {
	broken "sha1" sha1
	"SHA1" sha1 Sha1
}

hash! {
	secure "sha2" sha2
	"SHA-224" sha224 Sha224
	"SHA-256" sha256 Sha256
	"SHA-384" sha384 Sha384
	"SHA-512" sha512 Sha512
}

hash! {
	secure "sha3" sha3
	"SHA3-224 (Keccak)" sha3_224 Sha3_224
	"SHA3-256 (Keccak)" sha3_256 Sha3_256
	"SHA3-384 (Keccak)" sha3_384 Sha3_384
	"SHA3-512 (Keccak)" sha3_512 Sha3_512
}

hash! {
	secure "shabal" shabal
	"Shabal-192" shabal192 Shabal192
	"Shabal-224" shabal224 Shabal224
	"Shabal-256" shabal256 Shabal256
	"Shabal-384" shabal384 Shabal384
	"Shabal-512" shabal512 Shabal512
}

hash! {
	secure "whirlpool" whirlpool
	"Whirlpool" whirlpool Whirlpool
}

impl<'d, H: Digest + FixedOutputReset, S: Source<'d, N>, const N: usize> From<S> for HashSource<'d, H, S, N> {
	/// Creates a new hash sink, hashing data read from `source` with hash function
	/// [`H`].
	#[inline]
	fn from(source: S) -> Self {
		Self::new(H::new(), source)
	}
}

impl<'d, H: Digest + FixedOutputReset, S: Sink<'d, N>, const N: usize> From<S> for HashSink<'d, H, S, N> {
	/// Creates a new hash sink, hashing data written to `sink` with hash function
	/// [`H`].
	#[inline]
	fn from(sink: S) -> Self {
		Self::new(H::new(), sink)
	}
}
