// SPDX-License-Identifier: Apache-2.0

//! A pattern API, similar to [`std::str::pattern`], but matching in non-contiguous
//! byte sequences rather than strings. This allows use on segmented buffers, where
//! either the pattern or the data may not be valid UTF-8, and the pattern may be
//! found on a segment boundary (called the *segment boundary case*). Matches also
//! include the number of matched bytes in addition to the starting position.

mod internal;
mod matchers;

use std::ops::Range;
pub use matchers::*;
use crate::util::{AssertNonZero, IsTrue};

pub trait Pattern: Sized {
	type Matcher: Matcher;

	/// Returns `true` if the pattern is found in a `haystack` iterator.
	fn contained_in<'a, I: IntoIterator<Item = &'a [u8]>>(self, haystack: I) -> bool {
		self.into_matcher().has_match(haystack)
	}

	/// Finds matching ranges in a `haystack` iterator.
	fn matches_in<'a, I: IntoIterator<Item = &'a [u8]>>(self, haystack: I) -> Matches<'a, I::IntoIter, Self::Matcher> {
		self.into_matcher().matches(haystack)
	}

	/// Finds matching ranges in a `haystack` iterator.
	fn matches_in_str<'a, I: IntoIterator<Item = &'a str>>(self, haystack: I) -> StrMatches<'a, I::IntoIter, Self::Matcher> {
		self.into_matcher().str_matches(haystack)
	}

	/// Finds the first match in a `haystack` iterator, returning the matching range
	/// if found.
	fn find_in<'a>(self, haystack: impl IntoIterator<Item = &'a [u8]>) -> Option<Range<usize>> {
		self.into_matcher().find(haystack)
	}

	/// Finds the first match in a `haystack` iterator, returning the matching range
	/// if found.
	fn find_in_str<'a>(self, haystack: impl IntoIterator<Item = &'a str>) -> Option<Range<usize>> {
		self.into_matcher().str_find(haystack)
	}

	/// Creates a matcher for the pattern.
	fn into_matcher(self) -> Self::Matcher;
}

/// A pattern matching line terminator sequences.
#[derive(Copy, Clone, Debug, Default)]
pub struct LineTerminator;

/// A pattern matching either [`Ascii`] or [`Unicode`] whitespace greedily.
///
/// [`Ascii`]: Whitespace::Ascii,
/// [`Unicode`]: Whitespace::Unicode
#[derive(Copy, Clone, Debug, Default, Ord, PartialOrd, Eq, PartialEq)]
pub enum Whitespace {
	/// Matches ASCII whitespace as defined by [`u8::is_ascii_whitespace`], i.e.
	/// ` `, `\t`, `\n`, `\u{0C}`, and `\r`.
	#[default]
	Ascii,
	/// Matches Unicode whitespace as defined by [`char::is_whitespace`].
	Unicode
}

impl Pattern for u8 {
	type Matcher = ByteMatcher;

	/// Creates a matcher for a byte.
	#[inline]
	fn into_matcher(self) -> Self::Matcher {
		self.into()
	}
}

impl Pattern for &u8 {
	type Matcher = ByteMatcher;

	#[inline]
	fn into_matcher(self) -> Self::Matcher {
		(*self).into()
	}
}

impl Pattern for char {
	type Matcher = CharMatcher;

	/// Creates a matcher for a character, using a faster implementation if the
	/// character is ASCII.
	#[inline]
	fn into_matcher(self) -> Self::Matcher {
		self.into()
	}
}

impl Pattern for &char {
	type Matcher = CharMatcher;

	/// Creates a matcher for a character, using a faster implementation if the
	/// character is ASCII.
	#[inline]
	fn into_matcher(self) -> Self::Matcher {
		(*self).into()
	}
}

impl<'p> Pattern for &'p [u8] {
	type Matcher = SliceMatcher<'p>;

	/// Creates a matcher for the slice. Panics if the slice is empty.
	#[inline]
	fn into_matcher(self) -> Self::Matcher {
		assert!(!self.is_empty(), "pattern slice length should be non-zero");
		self.into()
	}
}

impl<'p> Pattern for &'p str {
	type Matcher = SliceMatcher<'p>;

	/// Creates a matcher for the slice. Panics if the slice is empty.
	#[inline]
	fn into_matcher(self) -> Self::Matcher {
		self.as_bytes().into_matcher()
	}
}

// Pattern trait can't be implemented for both FnMut(&u8) and FnMut(&char), or we
// get the "conflicting implementations" error. We can only do blanket impls for
// either one or neither, so we'll just do the latter for now. This can be revisited
// later.

impl Pattern for fn(&u8) -> bool {
	type Matcher = BytePredicateMatcher<Self>;

	/// Creates a matcher for matching a byte with a predicate.
	#[inline]
	fn into_matcher(self) -> Self::Matcher {
		self.into()
	}
}

impl Pattern for fn(&char) -> bool {
	type Matcher = CharPredicateMatcher<Self>;

	/// Creates a matcher for matching a character with a predicate.
	#[inline]
	fn into_matcher(self) -> Self::Matcher {
		self.into()
	}
}

impl Pattern for &[char] {
	type Matcher = CharListMatcher<Self>;

	/// Creates a matcher for a character list. Panics if the list is empty.
	#[inline]
	fn into_matcher(self) -> Self::Matcher {
		assert!(!self.is_empty(), "char list length should be non-zero");
		self.into()
	}
}

impl<const N: usize> Pattern for [char; N] where AssertNonZero<N>: IsTrue {
	type Matcher = CharListMatcher<Self>;

	/// Creates a matcher for a non-zero length character array.
	#[inline]
	fn into_matcher(self) -> Self::Matcher {
		self.into()
	}
}

impl Pattern for LineTerminator {
	type Matcher = LineTerminatorMatcher;

	/// Creates a line terminator matcher.
	#[inline]
	fn into_matcher(self) -> Self::Matcher {
		LineTerminatorMatcher::default()
	}
}

impl Pattern for Whitespace {
	type Matcher = WhitespaceMatcher;

	/// Creates a whitespace matcher.
	#[inline]
	fn into_matcher(self) -> Self::Matcher {
		self.into()
	}
}
