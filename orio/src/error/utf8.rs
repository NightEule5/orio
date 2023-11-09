// SPDX-License-Identifier: Apache-2.0

use amplify_derive::Display;
use simdutf8::compat;
use thiserror::Error;

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
