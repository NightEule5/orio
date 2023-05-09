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

use ErrorKind::Eos;
use crate::Buffer;
use crate::pool::SharedPool;
use crate::streams::{Sink, Source, Result, BufStream, BufSource, Error, BufSink, ErrorKind, Seekable, SeekOffset, SeekableExt};
use crate::streams::OperationKind::{BufFlush, BufRead};
use crate::segment::SIZE;

pub fn buffer_source<S: Source>(source: S) -> BufferedSource<S> {
	BufferedSource::new(source)
}

pub fn buffer_sink<S: Sink>(sink: S) -> BufferedSink<S> {
	BufferedSink {
		buffer: Buffer::default(),
		sink,
		closed: false,
	}
}

pub struct BufferedSource<S: Source> {
	buffer: Buffer,
	source: S,
	closed: bool,
}

impl<S: Source> BufferedSource<S> {
	fn new(source: S) -> Self {
		Self {
			buffer: Buffer::default(),
			source,
			closed: false,
		}
	}
}

impl<S: Source> BufferedSource<S> {
	/// Fills the buffer, rounding up to the nearest segment size.
	fn fill_buf(&mut self, mut byte_count: usize) -> Result<bool> {
		let count = self.buffer.count();
		let seg_count = (count + byte_count + SIZE - 1) / SIZE;
		byte_count = seg_count * SIZE - count;

		let cnt = self.source
					  .read(&mut self.buffer, byte_count)
					  .map_err(Error::with_op_buf_read);
		match cnt {
			Ok(cnt)                      => Ok(cnt > 0),
			Err(Error { kind: Eos, .. }) => Ok(false),
			Err(error)                   => Err(error)
		}
	}
}

impl<S: Source> Source for BufferedSource<S> {
	fn read(&mut self, buffer: &mut Buffer<impl SharedPool>, byte_count: usize) -> Result<usize> {
		if self.closed { return Err(Error::closed(BufRead)) }

		self.request(byte_count)?;
		self.buffer.read(buffer, byte_count)
	}

	fn close_source(&mut self) -> Result {
		if !self.closed {
			self.closed = true;
			let buf_close = self.buffer.close();
			let src_close = self.source.close_source();
			buf_close?;
			src_close
		} else {
			Ok(())
		}
	}
}

impl<S: Source> BufStream for BufferedSource<S> {
	fn buf(&self) -> &Buffer { &self.buffer }
	fn buf_mut(&mut self) -> &mut Buffer { &mut self.buffer }
}

impl<S: Source> BufSource for BufferedSource<S> {
	fn request(&mut self, byte_count: usize) -> Result<bool> {
		if self.closed { return Ok(false) }

		if self.buffer.request(byte_count)? {
			return Ok(true)
		}

		self.fill_buf(byte_count)
	}

	fn read_all(&mut self, sink: &mut impl Sink) -> Result<usize> {
		sink.write_all(self.buf_mut())
			.map_err(Error::with_op_buf_read)
	}
}

impl<S: Source + Seekable> BufferedSource<S> {
	fn seek_back(&mut self, off: usize) -> Result<usize> {
		let cur_pos = self.seek_pos()?;
		let mut new_pos = self.source.seek_back(off)?;
		let count = cur_pos - new_pos;

		if count == 0 {
			return Ok(new_pos)
		}

		let mut seek_buf = Buffer::lean();
		self.source
			.read(&mut seek_buf, count)
			.map_err(Error::with_op_seek)?;
		self.buffer
			.prefix_with(&mut seek_buf)
			.map_err(Error::with_op_seek)?;
		Ok(new_pos)
	}

	fn seek_forward(&mut self, mut off: usize) -> Result<usize> {
		off -= self.buffer.skip(off)?;
		self.source.seek_forward(off)
	}
}

impl<S: Source + Seekable> Seekable for BufferedSource<S> {
	fn seek(&mut self, mut offset: SeekOffset) -> Result<usize> {
		return match offset {
			SeekOffset::Forward(0) |
			SeekOffset::Back   (0) => self.seek_pos(),
			SeekOffset::Forward(off) => self.seek_forward(off),
			SeekOffset::Back   (off) => self.seek_back   (off),
			SeekOffset::FromEnd(_off @ ..=-1) => {
				let len = self.buffer.count();
				let pos = self.buffer.seek(offset)?;

				if pos < len {
					// We didn't seek through the entire buffer, just return the
					// current position.
					self.seek_pos()
				} else {
					// The buffer was exhausted, seek on the source.
					self.source.seek(offset)
				}
			}
			_ => {
				// No clever way to do the rest, just invalidate the buffered data
				// and seek on the source.
				self.buffer.skip_all()?;
				self.source.seek(offset)
			}
		}
	}

	fn seek_len(&mut self) -> Result<usize> { self.source.seek_len() }

	fn seek_pos(&mut self) -> Result<usize> {
		// Offset the source position back by the buffer length to account for
		// buffering.
		Ok(self.source.seek_pos()? - self.buffer.count())
	}
}

impl<S: Source> Drop for BufferedSource<S> {
	fn drop(&mut self) {
		let _ = self.close_source();
	}
}

pub struct BufferedSink<S: Sink> {
	buffer: Buffer,
	sink: S,
	closed: bool,
}

impl<S: Sink> Sink for BufferedSink<S> {
	fn write(&mut self, buffer: &mut Buffer<impl SharedPool>, byte_count: usize) -> Result<usize> {
		let cnt = self.buffer.write(buffer, byte_count)?;
		self.flush()?;
		Ok(cnt)
	}

	fn flush(&mut self) -> Result {
		if !self.closed {
			// Both of these need a chance to run before returning an error.
			let read = self.sink
						   .write_all(&mut self.buffer)
						   .map_err(Error::with_op_buf_flush);
			let flush = self.sink
							.flush()
							.map_err(Error::with_op_buf_flush);
			read?;
			flush?;
			Ok(())
		} else {
			return Err(Error::closed(BufFlush))
		}
	}

	fn close_sink(&mut self) -> Result {
		if !self.closed {
			self.closed = true;
			let flush = self.flush();
			let close = self.sink.close_sink();
			let clear = self.buffer.close();
			flush?;
			close?;
			clear
		} else {
			Ok(())
		}
	}
}

impl<S: Sink> BufStream for BufferedSink<S> {
	fn buf(&self) -> &Buffer { &self.buffer }
	fn buf_mut(&mut self) -> &mut Buffer { &mut self.buffer }
}

impl<S: Sink> BufSink for BufferedSink<S> {
	fn write_all(&mut self, source: &mut impl Source) -> Result<usize> {
		source.read_all(self.buf_mut())
			  .map_err(Error::with_op_buf_write)
	}
}

impl<S: Sink + Seekable> Seekable for BufferedSink<S> {
	fn seek(&mut self, offset: SeekOffset) -> Result<usize> {
		// Todo: Is there some less naive approach than flushing then seeking?
		self.flush().map_err(Error::with_op_seek)?;
		self.sink.seek(offset)
	}

	fn seek_len(&mut self) -> Result<usize> {
		Ok(self.buffer.count() + self.sink.seek_len()?)
	}
}

impl<S: Sink> Drop for BufferedSink<S> {
	fn drop(&mut self) {
		let _ = self.close_sink();
	}
}
