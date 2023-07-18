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
use std::io::Write;
use simdutf8::compat::from_utf8;
use crate::{Buffer, ByteString, Context::BufRead, Error, Result, ResultExt};
use crate::streams::{BufSource, OffsetUtf8Error, Sink, Source};
use crate::pool::SharedPool;

impl<P: SharedPool> Buffer<P> {
	fn read_segments(
		&mut self,
		mut max_count: usize,
		mut consume: impl FnMut(&[u8]) -> Result<usize>,
	) -> Result<usize> {
		let mut count = 0;
		self.segments.read(|data| {
			for seg in data {
				if max_count == 0 { break }
				let len = min(max_count, seg.len());
				let read = consume(seg.data(..len))?;
				count += read;
				max_count -= count;
				seg.consume(read);
			}
			Ok::<_, Error>(())
		})?;

		self.tidy().context(BufRead)?;
		Ok(count)
	}

	pub(crate) fn read_std<W: Write>(&mut self, writer: &mut W, count: usize) -> Result<usize> {
		self.read_segments(count, |seg| Ok(writer.write(seg)?))
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
				sink.segments.push_back(seg);
			} else {
				// Share the last partial segment.
				sink.segments.push_back(seg.share(count));
				segments.push_front(seg);
			}

			count -= len;
			read += len;
		}

		let result: Result = try {
			let self_tidy = self.tidy();
			let sink_tidy = sink.tidy();
			self_tidy?;
			sink_tidy?;
		};
		result.context(BufRead)?;

		Ok(read)
	}

	#[inline]
	fn read_all(&mut self, sink: &mut Buffer<impl SharedPool>) -> Result<usize> {
		self.read(sink, self.count())
	}

	fn close_source(&mut self) -> Result { self.close() }
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
			.context(BufRead)
	}

	fn read_i8(&mut self) -> Result<i8> {
		self.read_u8().map(|n| n as i8)
	}

	fn read_u8(&mut self) -> Result<u8> {
		self.require(1)?;

		let byte = self.segments.read(|segments| {
			let seg = segments.first_mut().expect(
				"should be at least one segment after require operation"
			);

			seg.pop().expect(
				"should be at least one byte available after require operation"
			)
		});

		self.tidy().context(BufRead)?;
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
		let count = dst.len();
		self.require(count)?;

		let mut off = 0;
		self.read_segments(count, |seg| {
			if off >= count { return Ok(0) }

			let len = min(dst.len(), seg.len());
			dst[off..off + len].copy_from_slice(&seg[..len]);
			off += len;
			Ok(len)
		})?;

		assert_eq!(off, dst.len(), "exact slice length should have been read");

		Ok(())
	}

	fn read_utf8(&mut self, str: &mut String, byte_count: usize) -> Result<usize> {
		let mut off = 0;
		self.read_segments(byte_count, |seg| {
			let utf8 = from_utf8(seg).map_err(|err|
				Error::new(BufRead, OffsetUtf8Error::new(err, off).into())
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
				Error::new(BufRead, OffsetUtf8Error::new(err, off).into())
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
