// SPDX-License-Identifier: Apache-2.0

use std::iter::FusedIterator;
use std::mem;
use simdutf8::compat::from_utf8;
use crate::Utf8Error;

/// An iterator over valid UTF-8 or invalid byte slices.
pub struct Utf8OrBytes<'a> {
	bytes: &'a [u8],
	invalid: &'a [u8],
}

impl<'a> From<&'a [u8]> for Utf8OrBytes<'a> {
	fn from(bytes: &'a [u8]) -> Self {
		Self {
			bytes,
			invalid: &[]
		}
	}
}

impl<'a> Iterator for Utf8OrBytes<'a> {
	type Item = Result<&'a str, &'a [u8]>;

	fn next(&mut self) -> Option<Self::Item> {
		if !self.invalid.is_empty() {
			let invalid = mem::take(&mut self.invalid);
			return Some(Err(invalid))
		}

		if self.bytes.is_empty() {
			return None
		}

		Some(
			match from_utf8(self.bytes).map_err(Utf8Error::from) {
				Ok(str) => Ok(str),
				Err(err) => {
					let (valid, invalid) = unsafe { err.split_valid(self.bytes) };
					if valid.is_empty() {
						self.bytes = &self.bytes[invalid.len()..];
						Err(invalid)
					} else {
						self.invalid = invalid;
						self.bytes = &self.bytes[valid.len() + invalid.len()..];
						Ok(valid)
					}
				}
			}
		)
	}
}

impl FusedIterator for Utf8OrBytes<'_> { }
