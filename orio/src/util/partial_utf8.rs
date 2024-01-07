// SPDX-License-Identifier: Apache-2.0

//! Util for decoding UTF-8 strings spread across multiple byte slices.

use std::borrow::Cow;
use all_asserts::{assert_le, assert_range};
use simdutf8::basic;
use simdutf8::compat::from_utf8;
use crate::Utf8Error;

// Char width copied from std

// https://tools.ietf.org/html/rfc3629
const UTF8_CHAR_WIDTH: &[u8; 256] = &[
	// 1  2  3  4  5  6  7  8  9  A  B  C  D  E  F
	1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 0
	1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 1
	1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 2
	1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 3
	1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 4
	1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 5
	1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 6
	1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 7
	0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 8
	0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // 9
	0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // A
	0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // B
	0, 0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // C
	2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, // D
	3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, // E
	4, 4, 4, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // F
];

#[inline]
pub const fn utf8_char_width(b: u8) -> usize {
	UTF8_CHAR_WIDTH[b as usize] as usize
}

#[derive(Default)]
pub struct CharBuf {
	buf: arrayvec::ArrayVec<u8, 4>
}

impl CharBuf {
	pub fn fill(&mut self, bytes: &mut &[u8]) -> Option<String> {
		let remaining = self.char_width()? - self.buf.len();
		let fill_count = remaining.min(bytes.len());
		self.buf
			.try_extend_from_slice(&bytes[..fill_count])
			.expect("character buffer should be large enough");
		*bytes = &bytes[fill_count..];
		self.decode().map(str::to_owned)
	}

	pub fn push(&mut self, bytes: &[u8]) {
		self.buf.clear();
		let len = utf8_char_width(bytes[0]);
		assert_range!(
			1..=len,
			bytes.len(),
			"length should be no larger than the expected UTF-8 char width"
		);
		self.buf
			.try_extend_from_slice(bytes)
			.expect("character buffer should be large enough");
	}

	fn decode(&mut self) -> Option<&str> {
		self.char_width().and_then(|width| {
			assert_le!(
				self.buf.len(),
				width,
				"character buffer should be no larger than the expected character \
				 width"
			);
			(self.buf.len() == width).then(||
				basic::from_utf8(&self.buf)
					.expect("the character buffer should contain valid UTF-8")
			)
		})
	}

	pub fn char_width(&self) -> Option<usize> {
		self.buf.first().map(|&byte|
			utf8_char_width(byte)
		)
	}
}

pub fn read_partial_utf8_into<'a>(
	slices: impl IntoIterator<Item = &'a [u8]>,
	sink: &mut String
) -> Result<usize, Utf8Error> {
	let mut count = 0;
	let mut part = CharBuf::default();
	for mut slice in slices {
		while !slice.is_empty() {
			match from_partial_utf8(&mut slice, &mut part) {
				Ok(str) => {
					sink.push_str(str.as_ref());
					count += str.len();
				}
				Err(mut err) => {
					err.valid_up_to += count;
					return Err(err)
				}
			};
		}
	}

	if !part.buf.is_empty() {
		let Some(residual) = part.decode() else {
			let count = part.buf.len();
			part.buf.fill(0);
			let bytes = part.buf.into_inner().unwrap();
			return Err(Utf8Error::incomplete_char(count, bytes, count))
		};
		sink.push_str(residual);
		count += residual.len();
	}
	Ok(count)
}

pub fn from_partial_utf8<'a>(bytes: &mut &'a [u8], part: &mut CharBuf) -> Result<Cow<'a, str>, Utf8Error> {
	if let Some(str) = part.fill(bytes) {
		Ok(str.into())
	} else {
		match from_utf8(bytes).map_err(Into::<Utf8Error>::into) {
			Ok(str) => {
				*bytes = &bytes[str.len()..];
				Ok(str.into())
			}
			Err(err) if err.kind.is_invalid_sequence() => Err(err),
			Err(err) => {
				let (valid, incomplete) = unsafe { err.split_valid(bytes) };
				part.push(incomplete);
				*bytes = &[];
				Ok(valid.into())
			}
		}
	}
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn normal() {
		let ref mut char_buf = CharBuf::default();
		assert_eq!(
			from_partial_utf8(&mut &b"Hello World!"[..], char_buf)
				.unwrap(),
			"Hello World!"
		);
	}

	#[test]
	fn boundary() {
		// — = \u2014
		let (mut a, mut b) = "Hello—World!".as_bytes().split_at(6);

		let ref mut char_buf = CharBuf::default();
		assert_eq!(
			from_partial_utf8(&mut a, char_buf).unwrap(),
			"Hello"
		);
		assert_eq!(
			from_partial_utf8(&mut b, char_buf).unwrap(),
			"—"
		);
		assert_eq!(
			from_partial_utf8(&mut b, char_buf).unwrap(),
			"World!"
		);
	}
}