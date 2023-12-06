// SPDX-License-Identifier: Apache-2.0

//! Util for decoding UTF-8 strings spread across multiple byte slices.

use core::str::utf8_char_width;
use std::cmp::min;
use std::mem;
use std::str::from_utf8_unchecked;
use all_asserts::{assert_range, debug_assert_lt};
use simdutf8::compat::{from_utf8, Utf8Error as SimdUtf8Error};
use crate::{expect, Utf8Error, Utf8ErrorKind};

struct PartialUtf8Decoder<'a, I> {
	iter: I,
	buf: [u8; 4],
	part: &'a [u8],
}

impl<'a, I: Iterator<Item = &'a [u8]>> Iterator for PartialUtf8Decoder<'a, I> {
	type Item = Result<Decoded<'a>, Utf8Error>;

	fn next(&mut self) -> Option<Self::Item> {
		let Some(mut bytes) = self.iter.next() else {
			return self.take_partial()
					   .map(|p| Ok(Decoded::Incomplete(p)))
		};

		if let Some(part) = self.take_partial() {
			debug_assert!(utf8_char_width(part[0]) > part.len());
			debug_assert!(part.len() < 4);
			let rem = utf8_char_width(part[0]) - part.len();
			self.buf[..part.len()].copy_from_slice(part);
			self.buf[part.len()..].copy_from_slice(&bytes[..rem]);
			bytes = &bytes[rem..];

			let Some(char) = char::from_u32(u32::from_be_bytes(self.buf)) else {

			}
		} else {

		}

		None
	}
}

impl<I> PartialUtf8Decoder<'_, I> {
	fn take_partial(&mut self) -> Option<&[u8]> {
		Some(mem::take(&mut self.part)).filter(<[u8]>::is_empty)
	}
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
			let mask = !(0xFFFFFFFF << (current * 8));
			let code = u32::from_be_bytes(self.buf) >> ((4 - current) * 8);
			char::try_from(code).map_err(|err|
				Utf8Error::invalid_seq(0, self.buf, current)
			)
		})
	}
}

pub enum Decoded<'a> {
	Str(&'a str),
	Char(char)
}

pub fn from_partial_utf8<'a>(bytes: &mut &'a [u8], part: &mut PartialChar) -> Result<Decoded<'a>, Utf8Error> {
	part.fill(bytes);
	if let Some(char) = part.pop() {
		return char.map(Decoded::Char)
	}

	match from_utf8(bytes).map_err(Into::into) {
		Ok(str) => Ok(Decoded::Str(str)),
		Err(err) if err.kind.is_invalid_sequence() => Err(err),
		Err(err) => {
			let (valid, incomplete) = bytes.split_at(err.valid_up_to);
			part.push(incomplete);

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
		assert!(
			matches!(
				from_partial_utf8(&mut &b"Hello World!"[..], &mut PartialChar::default()),
				Ok(Decoded::Str("Hello World!"))
			)
		);
	}

	#[test]
	fn boundary() {
		// — = \u2014
		let (mut a, mut b) = "Hello—World!".as_bytes().split_at(6);

		let ref mut partial_char = PartialChar::default();
		assert!(
			matches!(
				from_partial_utf8(&mut a, partial_char), Ok(Decoded::Str("Hello"))
			)
		);
		assert!(
			matches!(
				from_partial_utf8(&mut b, partial_char), Ok(Decoded::Char('—'))
			)
		);
		assert!(
			matches!(
				from_partial_utf8(&mut b, partial_char), Ok(Decoded::Str("World!"))
			)
		);
	}
}