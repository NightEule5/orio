// SPDX-License-Identifier: Apache-2.0

#![allow(incomplete_features)]
#![feature(adt_const_params)]
#![feature(round_char_boundary)]

use std::iter::repeat_with;
use std::ops::Range;
use std::str::from_utf8_unchecked;
use itertools::Itertools;
use orio::pattern::{LineTerminator, Pattern};
use pretty_assertions::assert_eq;
use quickcheck::{Arbitrary, Gen, TestResult};
use quickcheck_macros::quickcheck;

const ALPHABET: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";

#[derive(Copy, Clone, Debug)]
struct FullSplits<const HAYSTACK: &'static str, P> {
	slice_a: &'static str,
	slice_b: &'static str,
	pattern: P,
}

impl<const HAYSTACK: &'static str> Arbitrary for FullSplits<HAYSTACK, &'static str> {
	fn arbitrary(g: &mut Gen) -> Self {
		let split_point = usize::arbitrary(g) % HAYSTACK.len();
		let pattern_start = usize::arbitrary(g) % HAYSTACK.len();
		let pattern_end = (usize::arbitrary(g) % (HAYSTACK.len() - pattern_start)) + pattern_start;
		let (slice_a, slice_b) = HAYSTACK.split_at(split_point);
		let pattern = &HAYSTACK[pattern_start..pattern_end];
		Self { slice_a, slice_b, pattern }
	}

	fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
		let v = *self;
		Box::new((0..self.slice_a.len()).rev().map(move |split_point| {
			let (slice_a, slice_b) = HAYSTACK.split_at(split_point);
			Self { slice_a, slice_b, ..v }
		}))
	}
}

#[quickcheck]
fn match_slice(FullSplits { slice_a, slice_b, pattern, .. }: FullSplits<ALPHABET, &'static str>) -> TestResult {
	if pattern.is_empty() {
		return TestResult::discard()
	}
	let match_str = pattern.find_in([slice_a.as_bytes(), slice_b.as_bytes()].into_iter())
		.map(|range| &ALPHABET[range]);
	assert_eq!(match_str, Some(pattern));
	TestResult::passed()
}

impl<const HAYSTACK: &'static str> Arbitrary for FullSplits<HAYSTACK, u8> {
	fn arbitrary(g: &mut Gen) -> Self {
		let split_point = usize::arbitrary(g) % HAYSTACK.len();
		let pattern_idx = usize::arbitrary(g) % HAYSTACK.len();
		let (slice_a, slice_b) = HAYSTACK.split_at(split_point);
		let pattern = HAYSTACK.as_bytes()[pattern_idx];
		Self { slice_a, slice_b, pattern }
	}

	fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
		let v = *self;
		Box::new((0..self.slice_a.len()).rev().map(move |split_point| {
			let (slice_a, slice_b) = HAYSTACK.split_at(split_point);
			Self { slice_a, slice_b, ..v }
		}))
	}
}

#[quickcheck]
fn match_byte(FullSplits { slice_a, slice_b, pattern, .. }: FullSplits<ALPHABET, u8>) {
	let match_str = pattern.find_in([slice_a.as_bytes(), slice_b.as_bytes()].into_iter())
						   .map(|range| &ALPHABET[range]);
	assert_eq!(match_str, Some(unsafe { from_utf8_unchecked(&[pattern]) }));
}

#[derive(Clone, Debug)]
struct Unicode<P> {
	haystack: String,
	lengths: Vec<usize>,
	pattern: P,
	pattern_range: Range<usize>,
}

impl<P: Clone> Unicode<P> {
	fn pattern_str(&self) -> (Range<usize>, &str) {
		let range = &self.pattern_range;
		(range.clone(), &self.haystack[range.clone()])
	}

	fn gen(g: &mut Gen, align: bool, gen_pattern: fn(&mut Gen, &str) -> (Range<usize>, P)) -> Self {
		let haystack = loop {
			// Generate a non-empty string containing at least one ASCII character,
			// truncated to <=64 bytes.
			match String::arbitrary(g) {
				value if value.is_empty() || !value.contains(|c: char| c.is_ascii()) => { }
				mut value => {
					value.truncate(value.floor_char_boundary(64));
					break value
				}
			}
		};
		let (pattern_range, pattern) = gen_pattern(g, &haystack);
		let splits = usize::arbitrary(g) % 4;
		let lengths = if align {
			(0..=splits).scan(haystack.len(), |len, i|
				if i < splits {
					let split_point = haystack.floor_char_boundary(
						usize::arbitrary(g) % *len
					);
					*len -= split_point;
					Some(split_point)
				} else {
					Some(*len)
				}
			).collect()
		} else {
			(0..=splits).scan(haystack.len(), |len, i|
				if i < splits {
					let split_point = usize::arbitrary(g) % *len;
					*len -= split_point;
					Some(split_point)
				} else {
					Some(*len)
				}
			).collect()
		};
		Self { haystack, lengths, pattern, pattern_range }
	}

	fn split(&self) -> impl Iterator<Item = &[u8]> {
		let haystack = self.haystack.as_bytes();
		self.lengths.iter().scan(0, |start, &l| {
			let slice = &haystack[*start..*start + l];
			*start += l;
			Some(slice)
		})
	}

	/// Shrinks the first fragment by one byte, grows the last fragment, and keeps
	/// fragments in between the same length, such that the fragment count trends
	/// toward one as fragments at the front are drained.
	fn shrink_once(&mut self) {
		let Self { lengths, .. } = self;
		let last_index = lengths.len() - 1;
		lengths[0] -= 1;
		lengths[last_index] += 1;
		if lengths[0] == 0 {
			lengths.remove(0);
		}
	}
}

impl Unicode<u8> {
	fn choose_byte_in(g: &mut Gen, str: &str) -> (Range<usize>, u8) {
		let bytes = str.chars()
					   .filter(char::is_ascii)
					   .collect_vec();
		let choice = g.choose(&bytes).unwrap();
		str.char_indices()
		   .find_map(|(i, c)|
			   (&c == choice).then_some(
				   (i..i + 1, c as u8)
			   )
		   ).unwrap()
	}
}

impl Arbitrary for Unicode<u8> {
	fn arbitrary(g: &mut Gen) -> Self {
		Self::gen(g, false, Self::choose_byte_in)
	}

	fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
		let mut last = self.clone();
		Box::new(
			repeat_with(move || {
				last.shrink_once();
				last.clone()
			}).take_while(|Self { lengths, .. }|
				lengths.len() > 1
			)
		)
	}
}

#[quickcheck]
fn match_unicode_byte(data: Unicode<u8>) {
	let matched = data.pattern
					  .find_in(data.split())
					  .map(|r| (r.clone(), &data.haystack[r]));
	assert_eq!(matched, Some(data.pattern_str()));
}

impl Unicode<char> {
	fn choose_char_in(g: &mut Gen, str: &str) -> (Range<usize>, char) {
		let chars = str.chars().collect_vec();
		let choice = g.choose(&chars).unwrap();
		str.char_indices()
		   .find_map(|(i, c)|
			   (&c == choice).then_some(
				   (i..i + c.len_utf8(), c)
			   )
		   )
		   .unwrap()
	}
}

impl Arbitrary for Unicode<char> {
	fn arbitrary(g: &mut Gen) -> Self {
		Self::gen(g, false, Self::choose_char_in)
	}

	fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
		let mut last = self.clone();
		Box::new(
			repeat_with(move || {
				last.shrink_once();
				last.clone()
			}).take_while(|Self { lengths, .. }|
				lengths.len() > 1
			)
		)
	}
}

#[quickcheck]
fn match_char(data: Unicode<char>) {
	let matched = data.pattern
		.find_in(data.split())
		.map(|r| (r.clone(), &data.haystack[r]));
	assert_eq!(matched, Some(data.pattern_str()));
}

#[test]
fn match_line_terminator() {
	const LF: &str = "line 1\nline 2";
	const CR: &str = "line 1\rline 2";
	const CRLF: &str = "line 1\r\nline 2";
	let cases = [
		(&[&b"line 1\nline 2"[..]][..], LF,   6..7, "\n"  ),
		(&[b"line 1\r\nline 2"       ], CRLF, 6..8, "\r\n"),
		(&[b"line 1\rline 2"         ], CR,   6..7, "\r"  ),
		(&[b"line 1\r", b"\nline 2"  ], CRLF, 6..8, "\r\n"),
		(&[b"line 1\r", b"line 2"    ], CR,   6..7, "\r"  ),
	];

	for (haystack, str, range, matched) in cases {
		assert_eq!(
			LineTerminator.find_in(haystack.iter().cloned()).map(|r| (r.clone(), &str[r.clone()])),
			Some((range, matched))
		);
	}
}
