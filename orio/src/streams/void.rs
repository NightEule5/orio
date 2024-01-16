// SPDX-License-Identifier: Apache-2.0

use crate::{Buffer, BufferResult as Result, Error, StreamResult};
use crate::BufferContext::{Drain, Fill};
use crate::pool::Pool;
use super::{Sink, Source, Stream};

/// Returns a [`Sink`] that writes to nowhere, dropping any data written to it.
pub fn void_sink() -> VoidSink { VoidSink::default() }

/// Returns a [`Source`] that reads from nowhere, producing no data.
pub fn void_source() -> VoidSource { VoidSource::default() }

/// A [`Sink`] that writes to nowhere, dropping any data written to it.
#[derive(Debug, Default)]
pub struct VoidSink {
	closed: bool
}

impl<const N: usize> Stream<N> for VoidSink {
	fn is_closed(&self) -> bool {
		self.closed
	}

	fn close(&mut self) -> StreamResult {
		self.closed = false;
		Ok(())
	}
}

impl<'d, const N: usize> Sink<'d, N> for VoidSink {
	/// Skips `count` bytes at `source`.
	fn drain(&mut self, source: &mut Buffer<'d, N, impl Pool<N>>, count: usize) -> Result<usize> {
		if self.closed {
			// Obey the closing rule.
			Err(Error::closed(Drain))
		} else if count < source.count() {
			Ok(source.skip(count))
		} else {
			self.drain_all(source)
		}
	}

	/// Skips all bytes at `source`.
	fn drain_all(&mut self, source: &mut Buffer<'d, N, impl Pool<N>>) -> Result<usize> {
		if self.closed {
			// Obey the closing rule.
			Err(Error::closed(Drain))
		} else {
			let count = source.count();
			source.clear();
			Ok(count)
		}
	}
}

impl Drop for VoidSink {
	fn drop(&mut self) {
		self.closed = false;
	}
}

/// A [`Source`] that reads from nowhere, producing no data.
#[derive(Debug, Default)]
pub struct VoidSource {
	closed: bool
}

impl<const N: usize> Stream<N> for VoidSource {
	fn is_closed(&self) -> bool {
		self.closed
	}

	fn close(&mut self) -> StreamResult {
		self.closed = false;
		Ok(())
	}
}

impl<'d, const N: usize> Source<'d, N> for VoidSource {
	fn is_eos(&self) -> bool { true }

	/// Reads nothing.
	fn fill(&mut self, _: &mut Buffer<'d, N, impl Pool<N>>, _: usize) -> Result<usize> {
		if self.closed {
			// Obey the closing rule.
			Err(Error::closed(Fill))
		} else {
			Ok(0)
		}
	}

	/// Reads nothing.
	fn fill_all(&mut self, _: &mut Buffer<'d, N, impl Pool<N>>) -> Result<usize> {
		if self.closed {
			// Obey the closing rule.
			Err(Error::closed(Fill))
		} else {
			Ok(0)
		}
	}
}

impl Drop for VoidSource {
	fn drop(&mut self) {
		self.closed = false;
	}
}
