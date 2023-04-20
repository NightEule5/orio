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
use std::fmt;
use std::fmt::{Debug, Formatter};
use std::ops::RangeBounds;
use crate::pool::{DefaultPool, Pool};
use crate::segment::{Segment, Segments, SegmentSlice};
use crate::streams::{BufSink, BufSource, BufStream, Error, OperationKind, Result, Sink, Source, Stream};
use crate::streams::OperationKind::{BufRead, BufWrite};

pub struct Buffer<P: Pool = DefaultPool> {
	pool: P,
	segments: Segments,
	closed: bool,
}

impl<P: Pool + Default> Default for Buffer<P> {
	fn default() -> Self { Self::new(P::default()) }
}

impl<P: Pool> Debug for Buffer<P> {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.debug_struct("Buffer")
			.field("segments", &self.segments)
			.field("closed", &self.closed)
			.finish_non_exhaustive()
	}
}

impl<Pl: Pool> Buffer<Pl> {
	pub fn new(pool: Pl) -> Self {
		Self {
			pool,
			segments: Segments::default(),
			closed: false,
		}
	}

	pub fn count(&self) -> usize {
		self.segments.count()
	}

	pub fn clear(&mut self) -> Result {
		if !self.closed {
			self.segments
				.clear(&mut self.pool)
				.map_err(Into::into)
				.map_err(Error::with_op_buf_clear)
		} else {
			Ok(())
		}
	}

	/// Copies `byte_count` bytes into `sink`. Memory is either actually copied or
	/// shared for performance; the tradeoff between wasted space by sharing small
	/// segments and large, expensive mem-copies is managed by the implementation.
	pub fn copy_to(&self, sink: &mut Buffer<impl Pool>, mut byte_count: usize) {
		if byte_count == 0 { return }

		let ref mut dst = sink.segments;

		for seg in self.segments.as_slice().into_inner() {
			let len = seg.len();

			if len >= byte_count {
				dst.push(seg.share(byte_count));
				break
			}

			byte_count -= len;
			dst.push(seg.share_all());
		}
	}

	/// Copies all byte into `sink`. Memory is either actually copied or shared for
	/// performance; the tradeoff between wasted space by sharing small segments and
	/// large, expensive mem-copies is managed by the implementation.
	pub fn copy_all_to(&self, sink: &mut Buffer<impl Pool>) {
		let ref mut dst = sink.segments;

		for seg in self.segments.as_slice().into_inner() {
			dst.push(seg.share_all())
		}
	}

	/// Returns the index of a `char` in the buffer, or `None` if not found or if
	/// the buffer is closed.
	pub fn find_utf8_char(&self, char: char) -> Option<usize> {
		if self.closed { return None }
		self.segments
			.as_slice()
			.find_utf8(char)
	}

	/// Returns the index of a `char` in the buffer within `range`, or `None` if not
	/// found or if the buffer is closed.
	pub fn find_utf8_char_in<R: RangeBounds<usize>>(
		&self,
		char: char,
		range: R
	) -> Option<usize> {
		if self.closed { return None }
		self.segments
			.as_slice()
			.slice(range)
			.find_utf8(char)
	}

	fn require_open(&self, op: OperationKind) -> Result {
		if self.closed {
			Err(Error::closed(op))
		} else {
			Ok(())
		}
	}

	fn consume(&mut self, mut byte_count: usize) {
		while byte_count > 0 && self.count() > 0 {
			if let Some(mut seg) = self.segments.pop_front() {
				let n = min(seg.len(), byte_count);
				seg.consume(n);
				byte_count -= n;
				self.segments.push(seg);
			} else {
				return;
			}
		}
	}

	fn read_segments<F: FnOnce(SegmentSlice) -> Result<usize>>(
		&mut self,
		required: usize,
		read: F
	) -> Result<usize> {
		self.require_open(BufRead)?;
		self.require(required)?;
		let read_count = read(
			self.segments.as_slice().slice(..required)
		)?;
		self.consume(read_count);
		Ok(read_count)
	}

	fn write_segment<F: FnOnce(&mut Segment)>(&mut self, write: F) -> Result {
		self.require_open(BufWrite)?;

		let mut seg = if let Some(seg) = self.segments.pop_back() {
			seg
		} else {
			self.pool.claim_one()?
		};

		write(&mut seg);
		self.segments.push(seg);
		Ok(())
	}
}

impl<P: Pool> Drop for Buffer<P> {
	fn drop(&mut self) {
		let _ = self.close();
	}
}

impl<P: Pool> Stream for Buffer<P> {
	fn close(&mut self) -> Result {
		if !self.closed {
			self.closed = true;
			self.clear()
		} else {
			Ok(())
		}
	}
}

impl<P: Pool> Source for Buffer<P> {
	fn read(&mut self, sink: &mut Buffer<impl Pool>, mut count: usize) -> Result<usize> {
		self.require_open(BufRead)?;
		let mut read = 0;
		count = count.clamp(0, self.count());

		let Self { segments, .. } = self;
		while count > 0 {
			let Some(seg) = segments.pop_front() else { break };
			let len = seg.len();

			if seg.len() <= count {
				// Move full segments to the sink.
				sink.segments.push(seg);
			} else {
				// Share the last partial segment.
				sink.segments.push(seg.share(count));
				segments.push(seg);
			}

			count -= len;
			read += len;
		}

		Ok(read)
	}

	fn read_all(&mut self, sink: &mut Buffer<impl Pool>) -> Result<usize> {
		self.read(sink, self.count())
	}
}

impl<P: Pool> Sink for Buffer<P> {
	fn write(&mut self, source: &mut Buffer<impl Pool>, count: usize) -> Result<usize> {
		source.read(self, count).map_err(Error::with_op_buf_write)
	}

	fn write_all(&mut self, source: &mut Buffer<impl Pool>) -> Result<usize> {
		BufSource::read_all(source, self).map_err(Error::with_op_buf_write)
	}
}

impl<P: Pool> BufStream for Buffer<P> {
	fn buf(&self) -> &Self { self }
	fn buf_mut(&mut self) -> &mut Self { self }
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

impl<P: Pool> BufSource for Buffer<P> {
	fn request(&mut self, byte_count: usize) -> Result<bool> {
		Ok(!self.closed && self.count() >= byte_count)
	}

	fn read_all(&mut self, sink: &mut impl Sink) -> Result<usize> {
		sink.write_all(self)
			.map_err(Error::with_op_buf_read)
	}

	fn read_i8(&mut self) -> Result<i8> {
		self.read_u8().map(|n| n as i8)
	}

	fn read_u8(&mut self) -> Result<u8> {
		self.require_open(BufRead)?;
		self.require(1)?;
		let mut seg = self.segments.pop_front().unwrap();
		let byte = seg.pop().unwrap();
		self.segments.push(seg);
		Ok(byte)
	}

	gen_int_reads! {
		read_i16   read_i16_le   i16   read_u16   read_u16_le u16,
		read_i32   read_i32_le   i32   read_u32   read_u32_le u32,
		read_i64   read_i64_le   i64   read_u64   read_u64_le u64,
		read_isize read_isize_le isize read_usize read_usize_le usize
	}

	fn read_into_slice(&mut self, dst: &mut [u8]) -> Result<usize> {
		let n = min(dst.len(), self.count());
		self.read_into_slice_exact(&mut dst[..n])?;
		Ok(n)
	}

	fn read_into_slice_exact(&mut self, dst: &mut [u8]) -> Result {
		let len = dst.len();
		self.read_segments(len, |seg| {
			seg.copy_into_slice(dst);
			Ok(len)
		})?;
		Ok(())
	}

	fn read_utf8(&mut self, str: &mut String, byte_count: usize) -> Result<usize> {
		self.read_segments(byte_count, |seg| {
			let utf8 = {
				let (valid, err) = seg.valid_utf8();
				if let Some(err) = err { return Err(Error::invalid_utf8(BufRead, err)) }
				valid
			};

			Ok(
				utf8.decode_utf8()
					.fold(0, |acc, cur| {
						str.push_str(cur);
						acc + cur.len()
					})
			)
		})
	}

	fn read_utf8_line(&mut self, str: &mut String) -> Result<bool> {
		let mut line_term_found = false;
		self.read_segments(self.count(), |seg| {
			let (utf8, err) = seg.valid_utf8();
			let Some(mut pos) = utf8.find_utf8('\n') else {
				return if let Some(err) = err {
					Err(Error::invalid_utf8(BufRead, err))
				} else {
					let len = utf8.len();
					str.reserve(len);

					for data in utf8.decode_utf8() {
						str.push_str(data)
					}

					Ok(len)
				}
			};

			let mut n = pos;

			// Check for CRLF
			if pos > 0 && seg[pos - 1] == b'\r' {
				pos -= 1;
				n += 1;
			}

			for data in seg.slice(..pos).decode_utf8() {
				str.push_str(data)
			}

			line_term_found = true;
			Ok(n)
		})?;
		Ok(line_term_found)
	}

	fn read_utf8_into_slice(&mut self, mut str: &mut str) -> Result<usize> {
		self.read_segments(str.len(), move |mut seg| {
			let len = min(str.len(), seg.len());
			seg = seg.slice(..len);
			let utf8 = {
				let (valid, err) = seg.valid_utf8();
				if let Some(err) = err { return Err(Error::invalid_utf8(BufRead, err)) }
				valid
			};
			str = &mut str[..utf8.len()];

			unsafe {
				for text in utf8.decode_utf8() {
					let bytes = str.as_bytes_mut();
					bytes.copy_from_slice(text.as_bytes());
					str = &mut str[text.len()..];
				}
			}
			Ok(utf8.len())
		})
	}
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

impl<P: Pool> BufSink for Buffer<P> {
	fn write_all(&mut self, source: &mut impl Source) -> Result<usize> {
		source.read_all(self)
			  .map_err(Error::with_op_buf_write)
	}

	fn write_i8(&mut self, value: i8) -> Result {
		self.write_u8(value as u8)
	}

	fn write_u8(&mut self, value: u8) -> Result {
		self.write_segment(|seg| {
			seg.push(value);
		})
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
			self.write_segment(|seg|
				value = &value[seg.push_slice(value)..]
			)?;
		}
		Ok(())
	}

	fn write_utf8(&mut self, value: &str) -> Result {
		self.write_from_slice(value.as_bytes())
	}
}
