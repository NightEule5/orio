// SPDX-License-Identifier: Apache-2.0

use std::fs::File;
use std::io;
use std::path::Path;
use crate::{Buffer, BufferResult, StreamResult};
use crate::pool::Pool;
use super::{ReaderSource, Seekable, SeekOffset, Source, Stream, WriterSink};

/// A [`Source`] reading from a [file](File).
pub struct FileSource {
	source: ReaderSource<File>,
	read_count: usize,
	len: Option<usize>,
}

/// A [`Sink`] writing to a [file](File).
pub type FileSink = WriterSink<File>;

impl FileSource {
	pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
		File::open(path).map(Into::into)
	}

	/// Sets whether vectored read operations are allowed.
	#[inline]
	pub fn set_allow_vectored(&mut self, value: bool) {
		self.source.set_allow_vectored(value);
	}
}

impl From<File> for FileSource {
	fn from(value: File) -> Self {
		let len = value.metadata().ok().map(|meta| meta.len() as usize);
		Self {
			source: value.into(),
			read_count: 0,
			len,
		}
	}
}

impl<const N: usize> Stream<N> for FileSource {
	fn is_closed(&self) -> bool {
		Stream::<N>::is_closed(&self.source)
	}

	fn close(&mut self) -> StreamResult {
		Stream::<N>::close(&mut self.source)
	}
}

impl<const N: usize> Source<'_, N> for FileSource {
	/// Returns `true` if the *initial* end of file was reached, using the length
	/// from its metadata if it exists. Because files can be written after being
	/// opened for reading, more data could possibly be read from the file than
	/// this length. To conform to the terminality rule, once this returns `true`
	/// no more bytes will be read, even if more bytes have been written to the
	/// file after the source was created.
	fn is_eos(&self) -> bool {
		self.len.is_some_and(|len| self.read_count >= len)
	}

	fn fill(&mut self, sink: &mut Buffer<'_, N, impl Pool<N>>, mut count: usize) -> BufferResult<usize> {
		count = self.len.map_or(count, |len| {
			let remaining = len - self.read_count;
			count.min(remaining)
		});
		let read_count = self.source.fill(sink, count)?;
		self.read_count += read_count;
		Ok(read_count)
	}
}

impl Seekable for FileSource {
	fn seek(&mut self, offset: SeekOffset) -> StreamResult<usize> {
		self.source.seek(offset)
	}
}


