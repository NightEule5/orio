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

//! ## How it works
//!
//! Data is written to and read from reusable bits of memory called *segments*.
//! When a segment is consumed, it's returned to a *pool*. To write data, segments
//! are claimed from this pool. When the pool is exhausted, segments are created up
//! to a set limit. The default pool instance has two modes: with the `shared-pool`
//! feature its segment container is wrapped in an `Arc<Mutex<...>>`, otherwise
//! `RefCell` is used. The latter is faster but must be thread-local, meaning each
//! thread has its own pool.
//!
//! ### Segments
//!
//! Segments are reusable chunks of memory arranged in a ring buffer. Memory within
//! segments can either be owned by or shared between segments, avoiding expensive
//! mem-copy operations as much as possible. Shared memory is copy-on-write; it can
//! be read by multiple segments, only copying when written. Small amounts of data
//! under a set threshold (1024B by default) are not shared, as a tradeoff between
//! memory allocation performance and speed.
//!
//! The ring buffer behaves as a continuous byte deque. Bytes are read from one end
//! and written to the other, claiming new segments from the pool as it fills. Data
//! can have gaps where some segments are not filled or partially read, called *voids*.
//! Compacting these on every write could be costly, but keeping them is less space
//! efficient. To fix this, as void size reaches a threshold, 4096B by default, all
//! segments are compacted. This can also be triggered manually with the `compact`
//! function.
//!
//! As the segments are consumed, empty segments may be returned to the pool. Some
//! ratio of empty segments to full segments, the *retention ratio*, are kept for
//! further writes; the rest are recycled. The default ratio is 1, meaning for one
//! byte in the buffer, at least one byte of free space is kept for future writes.
//! This ratio allows buffers to keep segments they will likely need, but not keep
//! too many segments.
//!
//! Segments can be allocated when: 1) a buffer requests one but the pool has none
//! left, or 2) a shared segment is created then written to.

#![allow(incomplete_features)]
#![feature(
	associated_type_defaults,
	get_mut_unchecked,
	return_position_impl_trait_in_trait,
	specialization,
	thread_local,
	type_alias_impl_trait,
)]

mod segment;
mod pool;
mod buffer;
mod buffered_wrappers;

use std::error;
pub(crate) use segment::*;
pub use pool::*;
pub use buffer::*;
use buffered_wrappers::*;

use amplify_derive::Display;

pub(crate) const DEFAULT_SEGMENT_SIZE: usize = 8 * 1024;

#[derive(Copy, Clone, Debug, Display)]
pub enum ErrorKind {
	#[cfg(feature = "shared-pool")]
	#[display("could not get lock, mutex was poisoned")]
	Poison,
	#[display("could not borrow the pool, already in use")]
	PoolBorrow,
	#[display("invalid operation on closed stream")]
	Closed,
	#[display("could not clear the buffer")]
	BufClear,
	#[display("buffered write from source failed")]
	BufWrite,
	#[display("buffered read to sink failed")]
	BufRead,
	#[display("buffered sink could not be flushed to inner sink")]
	BufFlush,
	#[display("buffered stream could not be closed")]
	BufClose,
}

#[derive(Debug, Display)]
#[display("{kind}")]
pub struct Error {
	kind: ErrorKind,
	source: Option<Box<dyn error::Error>>,
}

impl error::Error for Error {
	fn source(&self) -> Option<&(dyn error::Error + 'static)> {
		self.source.as_deref()
	}
}

impl Error {
	fn new(
		kind: ErrorKind,
		source: impl error::Error + 'static
	) -> Self {
		Self {
			kind,
			source: Some(Box::new(source)),
		}
	}

	#[cfg(feature = "shared-pool")]
	pub(crate) fn poison(error: std::sync::PoisonError<&mut Vec<Segment>>) -> Self {
		Self {
			kind: ErrorKind::Poison,
			source: Some(Box::new(error)),
		}
	}

	pub(crate) fn borrow(error: impl error::Error + 'static) -> Self {
		Self::new(ErrorKind::PoolBorrow, error)
	}

	pub(crate) fn closed() -> Self {
		Self {
			kind: ErrorKind::Closed,
			source: None
		}
	}

	pub(crate) fn buf_clear(error: impl error::Error + 'static) -> Self {
		Self::new(ErrorKind::BufClear, error)
	}

	pub(crate) fn buf_write(error: impl error::Error + 'static) -> Self {
		Self::new(ErrorKind::BufWrite, error)
	}

	pub(crate) fn buf_read(error: impl error::Error + 'static) -> Self {
		Self::new(ErrorKind::BufRead, error)
	}

	pub(crate) fn buf_flush(error: impl error::Error + 'static) -> Self {
		Self::new(ErrorKind::BufFlush, error)
	}

	pub(crate) fn buf_close(error: impl error::Error + 'static) -> Self {
		Self::new(ErrorKind::BufClose, error)
	}
}

/// A data stream, either [`Source`] or [`Sink`]
pub trait Stream {
	type Error: error::Error + 'static = Error;

	/// Closes the stream. All default streams close automatically when dropped.
	/// Closing is idempotent, [`close`] may be called more than once with no
	/// effect.
	fn close(&mut self) -> Result<(), Self::Error> { Ok(()) }
}

/// A data source.
pub trait Source: Stream {
	/// Reads `count` bytes from the source into the buffer.
	fn read<const N: usize>(&mut self, sink: &mut Buffer<N, impl Pool<N>>, count: usize) -> Result<usize, Self::Error>;

	/// Reads all bytes from the source into the buffer.
	#[inline]
	fn read_all<const N: usize>(&mut self, sink: &mut Buffer<N, impl Pool<N>>) -> Result<usize, Self::Error> {
		self.read(sink, usize::MAX)
	}
}

pub trait SourceBuffer<const N: usize>: Source + Sized {
	/// Wrap the source in a buffered source.
	fn buffer<P: Pool<N> + Default>(self) -> impl BufSource<N> { buffer_source::<N, _, P>(self) }
}

impl<const N: usize, S: Source> SourceBuffer<N> for S { }

/// A data sink.
pub trait Sink: Stream {
	/// Writes `count` bytes from the buffer into the sink.
	fn write<const N: usize>(
		&mut self,
		source: &mut Buffer<N, impl Pool<N>>,
		count: usize
	) -> Result<usize, Self::Error>;

	/// Writes all bytes from the buffer into the sink.
	#[inline]
	fn write_all<const N: usize>(&mut self, source: &mut Buffer<N, impl Pool<N>>) -> Result<usize, Self::Error> {
		self.write(source, source.count())
	}

	/// Writes all buffered data to its final target.
	fn flush(&mut self) -> Result<(), Self::Error> { Ok(()) }
}

impl<S: Sink> Stream for S {
	/// Flushes and closes the stream. All default streams close automatically when
	/// dropped. Closing is idempotent, [`close`] may be called more than once with
	/// no effect.
	default fn close(&mut self) -> Result<(), Self::Error> { self.flush() }
}

pub trait SinkBuffer<const N: usize>: Sink + Sized {
	/// Wrap the sink in a buffered sink.
	fn buffer<P: Pool<N> + Default>(self) -> impl BufSink<N> { buffer_sink::<N, _, P>(self) }
}

impl<const N: usize, S: Sink> SinkBuffer<N> for S { }

pub trait BufStream<const N: usize = DEFAULT_SEGMENT_SIZE> {
	type Pool: Pool<N> = DefaultPool<N>;
	fn buf(&mut self) -> &mut Buffer<N, Self::Pool>;
}

pub trait BufSource<const N: usize = DEFAULT_SEGMENT_SIZE>: BufStream<N> + Source {
	fn read_all(&mut self, sink: &mut impl Sink) -> Result<usize, Self::Error>;
}

pub trait BufSink<const N: usize = DEFAULT_SEGMENT_SIZE>: BufStream<N> + Sink {
	fn write_all(&mut self, source: &mut impl Source) -> Result<usize, Self::Error>;
}
