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

use std::error;
use crate::{Buffer, DEFAULT_SEGMENT_SIZE, DefaultPool, Error, Pool};
use crate::buffered_wrappers::{buffer_sink, buffer_source};

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
