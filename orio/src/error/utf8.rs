// SPDX-License-Identifier: Apache-2.0

use std::str::from_utf8_unchecked;
use amplify_derive::Display;
use simdutf8::compat;
use simdutf8::basic::from_utf8;
use thiserror::Error;
use crate::util::partial_utf8::utf8_char_width;

/// A UTF-8 decode error.
#[derive(Copy, Clone, Debug, Error)]
#[error(
	"{kind} UTF-8 byte sequence ({:X?}) from index {valid_up_to}",
	self.bytes()
)]
pub struct Utf8Error {
	/// The length of the valid string before the error.
	pub valid_up_to: usize,
	/// The invalid or incomplete byte sequence, padded with zeros.
	pub bytes: [u8; 4],
	/// The number the bytes in the invalid or incomplete byte sequence.
	pub count: usize,
	/// The error kind.
	pub kind: Utf8ErrorKind
}

#[derive(Copy, Clone, Debug, Display)]
pub enum Utf8ErrorKind {
	/// An invalid byte sequence.
	#[display("invalid")]
	InvalidSequence,
	/// An incomplete character byte sequence.
	#[display("incomplete")]
	IncompleteChar
}

impl Utf8Error {
	pub(crate) fn invalid_seq(valid_up_to: usize, bytes: [u8; 4], count: usize) -> Self {
		Self {
			valid_up_to,
			bytes,
			count,
			kind: Utf8ErrorKind::InvalidSequence
		}
	}

	pub(crate) fn incomplete_char(valid_up_to: usize, bytes: [u8; 4], count: usize) -> Self {
		Self {
			valid_up_to,
			bytes,
			count,
			kind: Utf8ErrorKind::IncompleteChar
		}
	}

	/// The invalid or incomplete byte sequence.
	pub fn bytes(&self) -> &[u8] {
		&self.bytes[..self.count]
	}

	/// Returns the part of the `input` slice containing valid UTF-8 according to
	/// `valid_up_to`, decoded into a string slice.
	///
	/// # Safety
	///
	/// The returned string is only valid if `input` contains the same bytes being
	/// decoded when this error was raised. Passing a different slice could result
	/// in invalid UTF-8, which is undefined behavior.
	pub unsafe fn valid_in<'a>(&self, input: &'a [u8]) -> &'a str {
		debug_assert!(
			from_utf8(&input[..self.valid_up_to]).is_ok(),
			"data should be valid UTF-8 up to {}",
			self.valid_up_to
		);
		from_utf8_unchecked(&input[..self.valid_up_to])
	}

	/// Splits the `input` slice into a string of valid UTF-8 and remaining bytes,
	/// with the valid part being `valid_up_to` bytes long.
	///
	/// # Safety
	///
	/// The returned string is only valid if `input` contains the same bytes being
	/// decoded when this error was raised. Passing a different slice could result
	/// in invalid UTF-8, which is undefined behavior.
	pub unsafe fn split_valid<'a>(&self, input: &'a [u8]) -> (&'a str, &'a [u8]) {
		let (valid, invalid) = input.split_at(self.valid_up_to);
		let invalid_len = if self.count == 0 && !invalid.is_empty() {
			utf8_char_width(invalid[0])
		} else {
			self.count
		};

		debug_assert!(
			from_utf8(valid).is_ok(),
			"data should be valid UTF-8 up to {}",
			self.valid_up_to
		);
		(from_utf8_unchecked(valid), &invalid[..invalid_len.min(invalid.len())])
	}
}

impl Utf8ErrorKind {
	pub fn is_invalid_sequence(&self) -> bool {
		matches!(self, Self::InvalidSequence)
	}
	
	pub fn is_incomplete_char(&self) -> bool {
		matches!(self, Self::IncompleteChar)
	}
}

impl From<compat::Utf8Error> for Utf8Error {
	fn from(value: compat::Utf8Error) -> Self {
		if let Some(error_len) = value.error_len() {
			Self::invalid_seq(value.valid_up_to(), [0; 4], error_len)
		} else {
			Self::incomplete_char(value.valid_up_to(), [0; 4], 0)
		}
	}
}
