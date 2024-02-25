// SPDX-License-Identifier: Apache-2.0

#![cfg(feature = "bincode")]

use bincode::de::read::Reader;
use bincode::enc::write::Writer;
use bincode::error::{DecodeError, EncodeError};
use crate::Buffer;
use crate::pool::Pool;
use crate::streams::{BufSink, BufSource};

pub struct BufferWriter<'a, 'b, const N: usize, P: Pool<N>> {
	buffer: &'b mut Buffer<'a, N, P>,
	offset: usize,
}
pub type BufferReader<'a, 'b, const N: usize, P> = &'b mut Buffer<'a, N, P>;

pub trait AsBincodeWriter<'data, const N: usize>: BufSink<'data, N> {
	type Writer<'borrow>: Writer where Self: 'borrow, 'data: 'borrow;

	/// Converts the sink into a Bincode [`Writer`].
	fn as_bincode_writer(&mut self) -> Self::Writer<'_>;
}

pub trait AsBincodeReader<'data, const N: usize>: BufSource<'data, N> {
	type Reader<'borrow>: Reader where Self: 'borrow, 'data: 'borrow;

	/// Converts the source into a Bincode [`Reader`].
	fn as_bincode_reader(&mut self) -> Self::Reader<'_>;
}

impl<'a, const N: usize, T: BufSink<'a, N>> AsBincodeWriter<'a, N> for T {
	type Writer<'b> = BufferWriter<'a, 'b, N, T::Pool> where Self: 'b, 'a: 'b;

	#[inline]
	fn as_bincode_writer(&mut self) -> Self::Writer<'_> {
		BufferWriter {
			buffer: self.buf_mut(),
			offset: 0,
		}
	}
}

impl<'a, const N: usize, T: BufSource<'a, N>> AsBincodeReader<'a, N> for T {
	type Reader<'b> = BufferReader<'a, 'b, N, T::Pool> where Self: 'b, 'a: 'b;

	#[inline]
	fn as_bincode_reader(&mut self) -> Self::Reader<'_> {
		self.buf_mut()
	}
}

impl<'a, const N: usize, P: Pool<N>> Writer for BufferWriter<'a, '_, N, P> {
	fn write(&mut self, bytes: &[u8]) -> Result<(), EncodeError> {
		let count = self.buffer.write_from_slice(bytes).map_err(|err|
			EncodeError::Io {
				inner: err.into(),
				index: self.offset
			}
		)?;
		self.offset += count;
		Ok(())
	}
}

impl<'a, const N: usize, P: Pool<N>> Reader for BufferReader<'a, '_, N, P> {
	fn read(&mut self, bytes: &mut [u8]) -> Result<(), DecodeError> {
		self.read_slice_exact(bytes).map_err(|err|
			DecodeError::Io {
				inner: err.into(),
				additional: bytes.len()
			}
		)?;
		Ok(())
	}

	#[inline]
	fn peek_read(&mut self, count: usize) -> Option<&[u8]> {
		Some(self.peek(count))
	}

	#[inline]
	fn consume(&mut self, count: usize) {
		Buffer::skip(self, count);
	}
}
