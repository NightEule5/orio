// SPDX-License-Identifier: Apache-2.0

use crate::{Buffer, BufferResult as Result, ResultContext};
use crate::BufferContext::Drain;
use crate::pool::Pool;
use super::{Sink, Source, Stream};

/// Returns a [`Sink`] that writes to nowhere, dropping any data written to it.
pub fn void_sink() -> VoidSink { VoidSink }

/// Returns a [`Source`] that reads from nowhere, producing no data.
pub fn void_source() -> VoidSource { VoidSource }

/// A [`Sink`] that writes to nowhere, dropping any data written to it.
#[derive(Copy, Clone, Debug, Default)]
pub struct VoidSink;

impl<const N: usize> Stream<N> for VoidSink { }

impl<'d, const N: usize> Sink<'d, N> for VoidSink {
	/// Skips `count` bytes at `source`.
	fn drain(&mut self, source: &mut Buffer<'d, N, impl Pool<N>>, count: usize) -> Result<usize> {
		if count < source.count() {
			source.skip(count).context(Drain)
		} else {
			self.drain_all(source)
		}
	}

	/// Skips all bytes at `source`.
	fn drain_all(&mut self, source: &mut Buffer<'d, N, impl Pool<N>>) -> Result<usize> {
		let count = source.count();
		source.clear().context(Drain)?;
		Ok(count)
	}
}

/// A [`Source`] that reads from nowhere, producing no data.
#[derive(Copy, Clone, Debug, Default)]
pub struct VoidSource;

impl<const N: usize> Stream<N> for VoidSource { }

impl<'d, const N: usize> Source<'d, N> for VoidSource {
	fn is_eos(&self) -> bool { true }

	/// Reads nothing.
	fn fill(&mut self, _: &mut Buffer<'d, N, impl Pool<N>>, _: usize) -> Result<usize> {
		Ok(0)
	}

	/// Reads nothing.
	fn fill_all(&mut self, _: &mut Buffer<'d, N, impl Pool<N>>) -> Result<usize> {
		Ok(0)
	}
}
