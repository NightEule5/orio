// SPDX-License-Identifier: Apache-2.0

use std::io::SeekFrom;
use crate::expect;
use super::Result;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum SeekOffset {
	/// Reset the stream to the start. Equivalent to `FromStart(0)`.
	Reset,
	/// Move forward by an offset.
	Forward(usize),
	/// Move back by an offset.
	Back(usize),
	/// Seek a position from the start of the stream.
	FromStart(usize),
	/// Seek a position from the end of the stream. A positive position will seek
	/// beyond the stream, the behavior of which is implementation-dependent.
	FromEnd(isize),
}

impl SeekOffset {
	/// Converts to a start-based position given a current `pos` and `len`.
	pub fn to_pos(self, pos: usize, len: usize) -> usize {
		match self {
			SeekOffset::Reset => 0,
			SeekOffset::Forward(off) => pos.saturating_add(off),
			SeekOffset::Back   (off) => pos.saturating_sub(off),
			SeekOffset::FromStart(pos) => pos,
			SeekOffset::FromEnd(pos @ 0..) => len.saturating_add(pos as usize),
			SeekOffset::FromEnd(pos      ) => len.saturating_add_signed(pos)
		}
	}

	/// Convert into [`std::io`]'s [`SeekFrom`] enum.
	///
	/// # Panics
	///
	/// Panics when a `usize` offset is too large to convert to an `i64` value, in
	/// the case of [`Forward`](Self::Forward) or [`Back`](Self::Back).
	pub fn into_seek_from(self) -> SeekFrom {
		fn conv_signed(off: usize) -> i64 {
			expect!(
				off.try_into(),
				"usize offset {off} is too large to fit in an i64 value"
			)
		}

		match self {
			SeekOffset::Reset          => SeekFrom::Start(0),
			SeekOffset::Forward  (off) => SeekFrom::Current(conv_signed(off)),
			SeekOffset::Back     (off) => SeekFrom::Current(-conv_signed(off)),
			SeekOffset::FromStart(pos) => SeekFrom::Start(pos as u64),
			SeekOffset::FromEnd  (pos) => SeekFrom::End(pos as i64)
		}
	}
}

impl From<SeekFrom> for SeekOffset {
	fn from(value: SeekFrom) -> Self {
		fn conv_signed(off: i64) -> isize {
			expect!(
				off.try_into(),
				"i64 offset {off} is too large to fit in an isize value"
			)
		}

		fn conv(off: u64) -> usize {
			expect!(
				off.try_into(),
				"u64 offset {off} is too large to fit in an usize value"
			)
		}

		match value {
			SeekFrom::Start  (pos)       => SeekOffset::FromStart(conv(pos)),
			SeekFrom::End    (pos)       => SeekOffset::FromEnd(conv_signed(pos)),
			SeekFrom::Current(off @ 0..) => SeekOffset::Forward(conv(off as u64)),
			SeekFrom::Current(off      ) => SeekOffset::Back(conv(-off as u64))
		}
	}
}

/// A stream that supports seeking. Based on the [`std::io::Seek`] trait.
pub trait Seekable {
	/// Seeks to an `offset`, returning the new position.
	fn seek(&mut self, offset: SeekOffset) -> Result<usize>;

	/// Seeks to the end of the stream then back to the current position, returning
	/// the length.
	fn seek_len(&mut self) -> Result<usize> {
		let pos = self.seek_pos()?;
		let len = self.seek(SeekOffset::FromEnd(0))?;

		if pos != len {
			self.seek(SeekOffset::FromStart(pos))?;
		}

		Ok(len)
	}

	/// Returns the current position.
	fn seek_pos(&mut self) -> Result<usize> {
		self.seek(SeekOffset::Forward(0))
	}
}

/// A convenience extension for [`Seekable`].
pub trait SeekableExt: Seekable {
	/// Resets to the start of the stream. Shorthand for `seek(SeekOffset::Reset)`.
	fn reset(&mut self) -> Result {
		self.seek(SeekOffset::Reset)?;
		Ok(())
	}

	/// Seeks forward `offset` bytes relative to the current position, returning
	/// the new position. Shorthand for `seek(SeekOffset::Forward(offset))`.
	fn seek_forward(&mut self, offset: usize) -> Result<usize> {
		self.seek(SeekOffset::Forward(offset))
	}

	/// Seeks back `offset` bytes relative to the current position, returning the
	/// new position. Shorthand for `seek(SeekOffset::Back(offset))`.
	fn seek_back(&mut self, offset: usize) -> Result<usize> {
		self.seek(SeekOffset::Back(offset))
	}

	/// Seeks forward `offset` bytes relative to the start of the stream, returning
	/// the new position. Shorthand for `seek(SeekOffset::FromStart(offset))`.
	fn seek_from_start(&mut self, offset: usize) -> Result<usize> {
		self.seek(SeekOffset::FromStart(offset))
	}

	/// Seeks `offset` bytes relative to the end of the stream, returning the new
	/// position. Shorthand for `seek(SeekOffset::FromEnd(offset))`. The new
	/// position is `min(0, len + offset)`. A positive offset will seek beyond the
	/// stream, the behavior of which is implementation-dependent.
	fn seek_from_end(&mut self, offset: isize) -> Result<usize> {
		self.seek(SeekOffset::FromEnd(offset))
	}
}

impl<S: Seekable> SeekableExt for S { }
