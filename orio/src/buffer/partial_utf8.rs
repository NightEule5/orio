// SPDX-License-Identifier: Apache-2.0

//! Util for decoding UTF-8 strings spread across multiple byte slices.

use std::cmp::min;
use std::str::from_utf8_unchecked;
use all_asserts::assert_range;
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
const fn utf8_char_width(b: u8) -> usize {
	UTF8_CHAR_WIDTH[b as usize] as usize
}

#[derive(Default)]
pub struct PartialChar {
	buf: [u8; 4],
	len: Option<(usize, usize)>,
}

impl PartialChar {
	fn fill(&mut self, bytes: &mut &[u8]) {
		if let Some((current, required)) = self.len.as_mut() {
			let len = *current;
			let rem = min(*required - len, bytes.len());
			*current += rem;
			self.buf[len..len + rem].copy_from_slice(&bytes[..rem]);
			*bytes = &bytes[rem..];
		}
	}

	fn push(&mut self, bytes: &[u8]) {
		if bytes.is_empty() { return }

		let len = utf8_char_width(bytes[0]);
		assert_range!(
			1..=len,
			bytes.len(),
			"length should be no larger than the expected UTF-8 char width"
		);

		self.len = Some((bytes.len(), len));
		self.buf[..bytes.len()].copy_from_slice(bytes);
	}

	fn pop(&mut self) -> Option<Result<char, Utf8Error>> {
		let (current, required) = self.len?;
		(current == required).then(|| {
			self.len = None;
			Ok(from_utf8(&self.buf[..current])?.chars().next().unwrap())
		})
	}
}

#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub enum Decoded<'a> {
	Str(&'a str),
	Char(char)
}

pub fn read_partial_utf8_into(
	mut bytes: &[u8],
	sink: &mut String,
	part: &mut PartialChar,
	offset: &mut usize
) -> Result<usize, Utf8Error> {
	let last_offset = *offset;
	while !bytes.is_empty() {
		*offset += match from_partial_utf8(&mut bytes, part) {
			Ok(Decoded::Str(str)) => {
				sink.push_str(str);
				str.len()
			}
			Ok(Decoded::Char(char)) => {
				sink.push(char);
				char.len_utf8()
			}
			Err(mut err) => {
				err.valid_up_to += *offset;
				return Err(err)
			}
		};
	}
	Ok(*offset - last_offset)
}

pub fn from_partial_utf8<'a>(bytes: &mut &'a [u8], part: &mut PartialChar) -> Result<Decoded<'a>, Utf8Error> {
	part.fill(bytes);
	if let Some(char) = part.pop() {
		return char.map(Decoded::Char)
	}

	match from_utf8(bytes).map_err(Into::<Utf8Error>::into) {
		Ok(str) => {
			*bytes = &bytes[str.len()..];
			Ok(Decoded::Str(str))
		}
		Err(err) if err.kind.is_invalid_sequence() => Err(err),
		Err(err) => {
			let (valid, incomplete) = bytes.split_at(err.valid_up_to);
			part.push(incomplete);
			*bytes = &[];

			debug_assert!(
				from_utf8(valid).is_ok(),
				"data should be valid UTF-8 up to {}",
				err.valid_up_to
			);
			// Safety: Safe as long as simdutf8 produces the expected valid_up_to
			// value.
			unsafe {
				Ok(Decoded::Str(from_utf8_unchecked(valid)))
			}
		}
	}
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn normal() {
		let ref mut partial_char = PartialChar::default();
		assert_eq!(
			from_partial_utf8(&mut &b"Hello World!"[..], partial_char)
				.unwrap(),
			Decoded::Str("Hello World!")
		);
	}

	#[test]
	fn boundary() {
		// — = \u2014
		let (mut a, mut b) = "Hello—World!".as_bytes().split_at(6);

		let ref mut partial_char = PartialChar::default();
		assert_eq!(
			from_partial_utf8(&mut a, partial_char).unwrap(),
			Decoded::Str("Hello")
		);
		assert_eq!(
			from_partial_utf8(&mut b, partial_char).unwrap(),
			Decoded::Char('—')
		);
		assert_eq!(
			from_partial_utf8(&mut b, partial_char).unwrap(),
			Decoded::Str("World!")
		);
	}
}