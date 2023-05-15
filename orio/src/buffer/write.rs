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
use std::hash::Hasher;
use std::io::Read;
use crate::Buffer;
use crate::error::MapOp;
use crate::streams::{BufSink, BufSource, Error, Result, Sink, Source};
use crate::pool::SharedPool;
use crate::streams::OperationKind::BufWrite;

impl<P: SharedPool> Buffer<P> {
	fn write_segments(
		&mut self,
		mut count: usize,
		mut write: impl FnMut(&mut [u8]) -> Result<usize>
	) -> Result<usize> {
		let Self { pool, segments, .. } = self;

		segments.reserve(count, &mut *Self::lock_pool(pool).map_op(BufWrite)?);


		let mut written = 0;
		segments.write(|data| {
			for seg in data {
				let limit = min(count, seg.limit());
				let slice = seg.data_mut(..limit);

				if slice.is_empty() { continue }

				let n = write(slice).map_op(BufWrite)?;
				written += n;
				count -= n;
				seg.grow(n);

				if n == 0 { break }
			}

			Ok::<_, Error>(())
		})?;

		self.tidy().map_op(BufWrite)?;
		Ok(written)
	}

	pub(crate) fn write_std<R: Read>(&mut self, reader: &mut R, count: usize) -> Result<usize> {
		self.write_segments(count, |seg| Ok(reader.read(seg)?))
	}
}

impl<P: SharedPool> Sink for Buffer<P> {
	fn write(&mut self, source: &mut Buffer<impl SharedPool>, count: usize) -> Result<usize> {
		source.read(self, count).map_op(BufWrite)
	}

	fn write_all(&mut self, source: &mut Buffer<impl SharedPool>) -> Result<usize> {
		BufSource::read_all(source, self).map_op(BufWrite)
	}

	fn close_sink(&mut self) -> Result { self.close() }
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
			  .map_op(BufWrite)
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
				seg.copy_from_slice(&value[..n]);
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
