// SPDX-License-Identifier: Apache-2.0

mod internal;
mod iter;

use std::borrow::Borrow;
use std::cmp::min;
use std::ops::{Range, RangeTo};
use std::slice;
use all_asserts::assert_le;
use itertools::Itertools;
pub use iter::*;
use crate::pattern::Whitespace;

/// The matcher alignment, whether the matcher operates on characters or bytes.
#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq)]
pub enum Alignment {
	/// The matcher is aligned character-wise. The stream aligns the matcher input
	/// along UTF-8 character boundaries. This is slower than [`Byte`](Self::Byte)
	/// alignment.
	Char,
	/// The matcher is aligned bytewise, no alignment is needed.
	Byte
}

#[derive(Copy, Clone, Debug)]
pub enum MatchStep {
	/// Match completed
	Complete {
		/// The start index of the match.
		start: usize,
		/// The number of bytes matched.
		count: usize,
		/// The number of bytes consumed in this step.
		consumed: usize,
	},
	/// Matched partially
	Partial {
		/// The start index of the current partial match.
		start: usize,
		/// The number of bytes matched in this step.
		count: usize,
	},
	/// Rejected
	Reject {
		/// The number of bytes consumed in this step.
		consumed: usize,
	}
}

impl MatchStep {
	pub fn complete(start: usize, count: usize, consumed: usize) -> Self {
		Self::Complete { start, count, consumed }
	}

	pub fn partial(start: usize, count: usize) -> Self {
		Self::Partial { start, count }
	}

	pub fn reject(consumed: usize) -> Self {
		Self::Reject { consumed }
	}

	pub fn is_complete(&self) -> bool {
		matches!(self, Self::Complete { .. })
	}

	/// Returns the number of bytes consumed in this match step for a fragment
	/// `count` bytes in length.
	pub fn consumed_bytes(&self, count: RangeTo<usize>) -> usize {
		match self {
			&Self::Complete { consumed, .. } |
			&Self::Reject { consumed } => consumed.min(count.end),
			// Partial matches occur at the end of a slice, implying all bytes were
			// consumed.
			_ => count.end
		}
	}

	/// Converts a complete match step into a range, returning `None` if the match
	/// is not complete.
	pub fn into_range(self) -> Option<Range<usize>> {
		let Self::Complete { start, count, .. } = self else {
			return None
		};
		Some(start..start + count)
	}
}

impl MatchStep {
	fn set_consumed(&mut self, value: usize) {
		match self {
			MatchStep::Complete { consumed, .. } |
			MatchStep::Reject { consumed } => *consumed = value,
			_ => { }
		}
	}
}

/// A pattern matcher. This trait searches for non-overlapping matches in a byte
/// sequence split across multiple fragments. This is an analog to [`Searcher`]
/// from `std`.
///
/// # Implementation
///
/// Matchers search slices from a fragmented input for matches. These matches may
/// be completely contained within one slice, or partially contained in one or more
/// slices. When some bytes at the end of a slice match, the matcher keeps track of
/// the start index and the number of matching bytes. This partial match may be
/// discarded in subsequent slices, or extended if more bytes match to the left.
///
/// Because [`next`](Self::next) may be called more than once for each fragment,
/// the caller passes a fragment offset to the matcher. This is added to the start
/// index of any matches.
///
/// See [`SliceMatcher`] for an example.
///
/// [`Searcher`]: std::str::pattern::Searcher
pub trait Matcher {
	/// Finds the next match in `haystack`, with the start index offset by `offset`
	/// bytes. Returns `None` if `haystack` is empty.
	///
	/// If `haystack` contains a partial match—some bytes in the pattern matching
	/// but clipped at the end—the matcher will return [`MatchStep::Partial`] and
	/// try to complete the match in subsequent fragments. If this is unsuccessful,
	/// the matcher will return [`MatchStep::Reject`].
	fn next(&mut self, haystack: &[u8], offset: usize) -> Option<MatchStep>;
	/// Finds the next match in `haystack`, with the start index offset by `offset`
	/// bytes. Returns `None` if `haystack` is empty.
	///
	/// If `haystack` contains a partial match—some bytes in the pattern matching
	/// but clipped at the end—the matcher will return [`MatchStep::Partial`] and
	/// try to complete the match in subsequent fragments. If this is unsuccessful,
	/// the matcher will return [`MatchStep::Reject`].
	#[inline]
	fn next_in_str(&mut self, haystack: &str, offset: usize) -> Option<MatchStep> {
		self.next(haystack.as_bytes(), offset)
	}
	/// Ends the current partial match, if any. May return a complete match if the
	/// matcher is "greedy", i.e. a partial match is a valid one but the matcher
	/// waits for a longer match. Matchers with this behavior include [`LineTerminatorMatcher`],
	/// where a partial match of `\r` may be completed by `\n`, but `\r` by itself
	/// is also a valid line terminator, and [`WhitespaceMatcher`], which consumes
	/// the longest whitespace sequence found in the input.
	#[inline]
	fn end(&mut self) -> Option<MatchStep> { None }
	/// Returns the pattern alignment. When [`Char`] is returned, haystack fragments
	/// must be aligned such that they contains only valid UTF-8 characters before
	/// being passed to [`next`](Self::next). If alignment is not respected, the
	/// matcher may miss some matches around fragment boundaries.
	///
	/// Char-wise pattern matching is slower than bytewise matching, due to the added
	/// UTF-8 validation and alignment steps.
	///
	/// [`Char`]: Alignment::Char
	/// [`next`]: Self::next_in_str
	#[inline]
	fn alignment(&self) -> Alignment {
		Alignment::Byte
	}
}

/// Provides methods for iterating over matcher steps.
pub trait MatchIter: Matcher + Sized {
	/// Iterates over all match steps in a `haystack` iterator. [`alignment`] is
	/// respected.
	///
	/// [`alignment`]: Self::alignment
	fn steps<'a, I: IntoIterator<Item = &'a [u8]>>(self, haystack: I) -> Steps<'a, I::IntoIter, Self> {
		match self.alignment() {
			Alignment::Char => Steps::Charwise(StepsInner::new(haystack.into(), self)),
			Alignment::Byte => Steps::Bytewise(StepsInner::new(haystack.into_iter(), self))
		}

	}

	/// Iterates over all match steps in a `haystack` iterator.
	fn str_steps<'a, I: IntoIterator<Item = &'a str>>(self, haystack: I) -> StrSteps<'a, I::IntoIter, Self> {
		StepsInner::new(haystack.into_iter(), self)
	}

	/// Iterates over matching ranges in a `haystack` iterator. [`alignment`] is
	/// respected.
	///
	/// [`alignment`]: Self::alignment
	fn matches<'a, I: IntoIterator<Item = &'a [u8]>>(self, haystack: I) -> Matches<'a, I::IntoIter, Self> {
		match self.steps(haystack) {
			Steps::Charwise(steps) => Matches::Charwise(CompleteMatches::new(steps)),
			Steps::Bytewise(steps) => Matches::Bytewise(CompleteMatches::new(steps))
		}
	}

	/// Iterates over matching ranges in a `haystack` iterator.
	fn str_matches<'a, I: IntoIterator<Item = &'a str>>(self, haystack: I) -> StrMatches<'a, I::IntoIter, Self> {
		CompleteMatches::new(self.str_steps(haystack))
	}

	/// Finds the first matching range in a `haystack` iterator. [`alignment`] is
	/// respected.
	///
	/// [`alignment`]: Self::alignment
	fn find<'a>(self, haystack: impl IntoIterator<Item = &'a [u8]>) -> Option<Range<usize>> {
		self.matches(haystack).next()
	}

	/// Finds the first matching range in a `haystack` iterator.
	fn str_find<'a>(self, haystack: impl IntoIterator<Item = &'a str>) -> Option<Range<usize>> {
		self.str_matches(haystack).next()
	}

	/// Returns `true` if a match is found in a `haystack` iterator. [`alignment`] is
	/// respected.
	///
	/// [`alignment`]: Self::alignment
	fn has_match<'a>(self, haystack: impl IntoIterator<Item = &'a [u8]>) -> bool {
		self.steps(haystack)
			.any(|step| step.is_complete())
	}

	/// Returns `true` if a match if found in a `haystack` iterator.
	fn has_str_match<'a>(self, haystack: impl IntoIterator<Item = &'a str>) -> bool {
		self.str_steps(haystack)
			.any(|step| step.is_complete())
	}
}

impl<T: Matcher> MatchIter for T { }

/// A matcher for a single byte.
#[derive(Copy, Clone, Debug, amplify_derive::From)]
pub struct ByteMatcher(u8);

impl Matcher for ByteMatcher {
	fn next(&mut self, haystack: &[u8], offset: usize) -> Option<MatchStep> {
		find_byte(haystack, offset, |b| b == &self.0)
	}
}

#[derive(Copy, Clone, Debug, Default)]
struct PartialMatch {
	start: usize,
	count: usize
}

impl PartialMatch {
	fn is_empty(&self) -> bool {
		self.count == 0
	}

	fn reset_invalid(&mut self, offset: usize) {
		if self.start + self.count < offset {
			self.count = 0;
		}
	}

	fn reset(&mut self) -> (usize, usize) {
		let count = self.count;
		self.count = 0;
		(self.start, count)
	}

	fn start(&mut self, start: usize, count: usize) {
		self.start = start;
		self.count = count;
	}

	fn extend_by(&mut self, count: usize) -> usize {
		self.count += count;
		self.count
	}

	fn remaining_in<'a>(&self, pattern: &'a [u8]) -> &'a [u8] {
		&pattern[self.count..]
	}
}

trait SliceMatcherSpec {
	fn state(&self) -> &PartialMatch;
	fn state_mut(&mut self) -> &mut PartialMatch;
	fn pattern(&self) -> &[u8];
	fn width(&self) -> usize {
		self.pattern().len()
	}
	fn remaining_pattern(&self) -> &[u8] {
		self.state().remaining_in(self.pattern())
	}
}

/// A matcher for a byte sequence slice.
#[derive(Copy, Clone, Debug)]
pub struct SliceMatcher<'a> {
	pattern: &'a [u8],
	partial: PartialMatch
}

impl<'a> From<&'a [u8]> for SliceMatcher<'a> {
	fn from(pattern: &'a [u8]) -> Self {
		Self {
			pattern,
			partial: PartialMatch::default()
		}
	}
}

impl<'a> From<&'a str> for SliceMatcher<'a> {
	fn from(pattern: &'a str) -> Self {
		pattern.as_bytes().into()
	}
}

impl SliceMatcherSpec for SliceMatcher<'_> {
	fn state(&self) -> &PartialMatch {
		&self.partial
	}
	fn state_mut(&mut self) -> &mut PartialMatch {
		&mut self.partial
	}
	fn pattern(&self) -> &[u8] { self.pattern }
}

/// A matcher for a unicode `char`.
#[derive(Copy, Clone, Debug)]
pub struct UnicodeMatcher {
	bytes: [u8; 4],
	width: usize,
	partial: PartialMatch
}

impl From<char> for UnicodeMatcher {
	fn from(value: char) -> Self {
		let mut bytes = [0; 4];
		let width = value.encode_utf8(&mut bytes).len();
		Self {
			bytes,
			width,
			partial: PartialMatch::default()
		}
	}
}

impl SliceMatcherSpec for UnicodeMatcher {
	fn state(&self) -> &PartialMatch {
		&self.partial
	}
	fn state_mut(&mut self) -> &mut PartialMatch {
		&mut self.partial
	}
	fn pattern(&self) -> &[u8] {
		&self.bytes[..self.width]
	}
	fn width(&self) -> usize { self.width }
}

impl<T: SliceMatcherSpec> Matcher for T {
	fn next(&mut self, haystack: &[u8], offset: usize) -> Option<MatchStep> {
		if haystack.is_empty() {
			return None
		}

		self.state_mut().reset_invalid(offset);

		let step = if self.state().is_empty() {
			if let Some((start, count)) = find_partial(haystack, self.pattern()) {
				if count == self.width() {
					MatchStep::complete(start + offset, count, start + self.width())
				} else {
					self.state_mut().start(start + offset, count);
					MatchStep::partial(start + offset, count)
				}
			} else {
				MatchStep::reject(haystack.len())
			}
		} else if let Some(count) = extend_partial(haystack, self.remaining_pattern()) {
			let partial_count = self.state_mut().extend_by(count);
			assert_le!(partial_count, self.width());
			if partial_count == self.width() {
				let consumed = count;
				let (start, count) = self.state_mut().reset();
				MatchStep::complete(start, count, consumed)
			} else {
				MatchStep::partial(self.state().start, count)
			}
		} else {
			MatchStep::reject(haystack.len())
		};

		Some(step)
	}

	fn end(&mut self) -> Option<MatchStep> {
		match self.state_mut().reset() {
			(_, 0) => None,
			(_, _) => Some(MatchStep::reject(0))
		}
	}
}

macro_rules! enum_matcher {
    (
		$(#[$meta:meta])*
		enum $name:ident$(<$($param:ident: $constraint:path),*>)? {
			$($entry:ident($entry_ty:ident$(<$($entry_ty_arg:path),*>)?)),+
		}
	) => {
		$(#[$meta])*
		pub enum $name$(<$($param: $constraint),*>)? {
			$($entry($entry_ty$(<$($entry_ty_arg),*>)?)),+
		}

		impl$(<$($param: $constraint),*>)? Matcher for $name$(<$($param)?>)? {
			#[inline]
			fn next(&mut self, haystack: &[u8], offset: usize) -> Option<MatchStep> {
				match self {
					$(
					Self::$entry(matcher) => matcher.next(haystack, offset),
					)+
				}
			}

			#[inline]
			fn next_in_str(&mut self, haystack: &str, offset: usize) -> Option<MatchStep> {
				match self {
					$(
					Self::$entry(matcher) => matcher.next_in_str(haystack, offset),
					)+
				}
			}

			#[inline]
			fn alignment(&self) -> Alignment {
				match self {
					$(Self::$entry(matcher) => matcher.alignment()),+
				}
			}
		}
	};
}

enum_matcher! {
	/// A `char` matcher, matching ASCII with a faster byte matcher.
	#[derive(Copy, Clone, Debug)]
	enum CharMatcher {
		Ascii(ByteMatcher),
		Unicode(UnicodeMatcher)
	}
}

impl From<char> for CharMatcher {
	fn from(value: char) -> Self {
		if value.is_ascii() {
			Self::Ascii((value as u8).into())
		} else {
			Self::Unicode(value.into())
		}
	}
}

/// A matcher for a byte predicate.
#[derive(Copy, Clone, Debug, amplify_derive::From)]
pub struct BytePredicateMatcher<P: FnMut(&u8) -> bool>(P);

impl<P: FnMut(&u8) -> bool> Matcher for BytePredicateMatcher<P> {
	fn next(&mut self, haystack: &[u8], offset: usize) -> Option<MatchStep> {
		find_byte(haystack, offset, &mut self.0)
	}
}

/// A matcher for a `char` predicate.
#[derive(Copy, Clone, Debug, amplify_derive::From)]
pub struct CharPredicateMatcher<P: FnMut(&char) -> bool>(P);

impl<P: FnMut(&char) -> bool> Matcher for CharPredicateMatcher<P> {
	fn next(&mut self, haystack: &[u8], offset: usize) -> Option<MatchStep> {
		find_char(haystack, offset, &mut self.0)
	}

	#[inline]
	fn alignment(&self) -> Alignment {
		Alignment::Char
	}
}

mod sealed {
	use std::borrow::Borrow;

	pub trait CharList: Borrow<[char]> {
		type AsciiRepr: AsciiList;
	}

	pub trait AsciiList {
		fn ascii_contains(&self, byte: &u8) -> bool;
	}

	impl CharList for &[char] {
		type AsciiRepr = Self;
	}

	impl<const N: usize> CharList for [char; N] {
		type AsciiRepr = [u8; N];
	}

	impl AsciiList for &[char] {
		#[inline]
		fn ascii_contains(&self, &byte: &u8) -> bool {
			self.contains(&char::from(byte))
		}
	}

	impl<const N: usize> AsciiList for [u8; N] {
		#[inline]
		fn ascii_contains(&self, byte: &u8) -> bool {
			self.contains(byte)
		}
	}
}

/// A matcher for ASCII character lists, providing faster matching than
/// [`UnicodeCharListMatcher`].
#[derive(Copy, Clone, Debug)]
pub struct AsciiCharListMatcher<L>(L);

impl<'a> TryFrom<&'a [char]> for AsciiCharListMatcher<&'a [char]> {
	type Error = &'a [char];
	fn try_from(value: &'a [char]) -> Result<Self, Self::Error> {
		if value.iter().all(char::is_ascii) {
			Ok(Self(value))
		} else {
			Err(value)
		}
	}
}

impl<const N: usize> TryFrom<[char; N]> for AsciiCharListMatcher<[u8; N]> {
	type Error = [char; N];
	fn try_from(value: [char; N]) -> Result<Self, Self::Error> {
		if value.iter().all(char::is_ascii) {
			Ok(Self(value.map(|c| c as u8)))
		} else {
			Err(value)
		}
	}
}

impl<L: sealed::AsciiList> Matcher for AsciiCharListMatcher<L> {
	fn next(&mut self, haystack: &[u8], offset: usize) -> Option<MatchStep> {
		find_byte(haystack, offset, |b| self.0.ascii_contains(b))
	}
}

/// A matcher for a list of UTF-8 characters. This implementation is slower than
/// [`AsciiCharListMatcher`] for lists containing only ASCII characters.
///
/// For ease of implementation, this matcher is currently character-aligned. This
/// will likely change in the future.
#[derive(Clone, Debug)]
pub struct UnicodeCharListMatcher<L>(L);

impl<'a> From<&'a [char]> for UnicodeCharListMatcher<&'a [char]> {
	fn from(value: &'a [char]) -> Self {
		Self(value)
	}
}

impl<const N: usize> From<[char; N]> for UnicodeCharListMatcher<[char; N]> {
	fn from(value: [char; N]) -> Self {
		Self(value)
	}
}

impl<L: Borrow<[char]>> Matcher for UnicodeCharListMatcher<L> {
	fn next(&mut self, haystack: &[u8], offset: usize) -> Option<MatchStep> {
		find_char(haystack, offset, |c| self.0.borrow().contains(c))
	}

	#[inline]
	fn alignment(&self) -> Alignment {
		Alignment::Char
	}
}

enum_matcher! {
	/// A matcher for any `char` in a list. Lists with containing only ASCII
	/// characters are matched using a faster single-byte implementation.
	enum CharListMatcher<L: sealed::CharList> {
		Ascii(AsciiCharListMatcher<L::AsciiRepr>),
		Unicode(UnicodeCharListMatcher<L>)
	}
}

impl<'a> From<&'a [char]> for CharListMatcher<&'a [char]> {
	fn from(list: &'a [char]) -> Self {
		list.try_into()
			.map_or_else(|l| Self::Unicode(l.into()), Self::Ascii)
	}
}

impl<const N: usize> From<[char; N]> for CharListMatcher<[char; N]> {
	fn from(list: [char; N]) -> Self {
		list.try_into()
			.map_or_else(|l| Self::Unicode(l.into()), Self::Ascii)
	}
}

/// A matcher for line terminators, matching `"\r\n"`, `'\r'`, or `'\n'` greedily.
#[derive(Copy, Clone, Debug, Default)]
pub struct LineTerminatorMatcher {
	cr_index: Option<usize>
}

impl Matcher for LineTerminatorMatcher {
	fn next(&mut self, haystack: &[u8], offset: usize) -> Option<MatchStep> {
		if haystack.is_empty() {
			return None
		}

		let step = if let Some(cr) = self.cr_index.take().filter(|&cr| offset - 1 <= cr) {
			let (count, consumed) = match haystack {
				[b'\n', ..] => (2, 1),
				_           => (1, 0)
			};
			MatchStep::complete(cr, count, consumed)
		} else {
			match haystack.iter().find_position(|b| matches!(b, b'\r' | b'\n')) {
				Some((pos, b'\r')) if pos == haystack.len() - 1 => {
					let cr = pos + offset;
					self.cr_index = Some(cr);
					MatchStep::partial(cr, 1)
				}
				Some((pos, b'\r')) if haystack[pos + 1] == b'\n' =>
					MatchStep::complete(pos + offset, 2, pos + 2),
				Some((pos, _)) =>
					MatchStep::complete(pos + offset, 1, pos + 1),
				None => MatchStep::reject(haystack.len())
			}
		};
		Some(step)
	}

	fn end(&mut self) -> Option<MatchStep> {
		self.cr_index.take().map(|i| MatchStep::complete(i, 1, 0))
	}
}

/// A greedy whitespace matcher. Supports either [ASCII] or [Unicode] whitespace.
/// ASCII matching is faster than Unicode, so it is preferred for input known to
/// only contain ASCII text.
///
/// [ASCII]: WhitespaceMatcher::ascii
/// [Unicode]: WhitespaceMatcher::unicode
pub struct WhitespaceMatcher {
	state: PartialMatch,
	kind: Whitespace,
}

impl WhitespaceMatcher {
	/// Creates an ASCII whitespace matcher.
	pub fn ascii() -> Self {
		Whitespace::Ascii.into()
	}

	/// Creates a Unicode whitespace matcher.
	pub fn unicode() -> Self {
		Whitespace::Unicode.into()
	}
}

impl From<Whitespace> for WhitespaceMatcher {
	fn from(kind: Whitespace) -> Self {
		Self {
			state: PartialMatch::default(),
			kind
		}
	}
}

impl WhitespaceMatcher {
	fn next_ascii(&mut self, haystack: &[u8], offset: usize) -> MatchStep {
		self.next_with(
			offset,
			haystack.len(),
			|| haystack.iter().copied().enumerate(),
			|&(_, c)| !c.is_ascii_whitespace()
		)
	}

	fn next_unicode(&mut self, haystack: &str, offset: usize) -> MatchStep {
		self.next_with(
			offset,
			haystack.len(),
			|| haystack.char_indices(),
			|&(_, c)| !c.is_whitespace()
		)
	}

	fn next_with<I: Iterator<Item = (usize, T)>, T: Copy>(
		&mut self,
		offset: usize,
		len: usize,
		mut indices: impl FnMut() -> I,
		is_non_whitespace: fn(&(usize, T)) -> bool
	) -> MatchStep {
		fn count<T>(
			mut iter: impl Iterator<Item = (usize, T)>,
			len: usize,
			is_non_whitespace: impl Fn(&(usize, T)) -> bool
		) -> usize {
			iter.find(is_non_whitespace)
				.map_or(len, |(i, _)| i)
		}

		let mut find = || {
			let mut whitespace = indices().skip_while(is_non_whitespace).peekable();
			whitespace.peek().copied().map(|(start, _)|
				start..count(whitespace, len, is_non_whitespace)
			)
		};

		match &mut self.state {
			state if state.is_empty() =>
				match find() {
					Some(range) if range.end < len =>
						MatchStep::complete(range.start + offset, range.len(), range.len()),
					Some(range) => {
						state.start(range.start + offset, range.len());
						MatchStep::partial(state.start, state.count)
					},
					None => MatchStep::reject(len)
				},
			state => {
				let count = count(indices(), len, is_non_whitespace);
				state.extend_by(count);
				if count < len {
					MatchStep::partial(state.start, count)
				} else {
					let (start, total) = state.reset();
					MatchStep::complete(start, total, count)
				}
			}
		}
	}
}

impl Matcher for WhitespaceMatcher {
	fn next(&mut self, haystack: &[u8], offset: usize) -> Option<MatchStep> {
		if haystack.is_empty() {
			return None
		}

		self.state.reset_invalid(offset);

		let step = match self.kind {
			Whitespace::Ascii => self.next_ascii(haystack, offset),
			Whitespace::Unicode =>
				match internal::decode_valid(haystack) {
					(Some(haystack), checked) =>
						match self.next_unicode(haystack, offset) {
							MatchStep::Partial { start, count } => {
								self.state.reset();
								MatchStep::complete(start, count, checked)
							}
							mut step => {
								step.set_consumed(checked);
								step
							}
						},
					(None, checked) => self.end().unwrap_or(MatchStep::reject(checked))
				}
		};
		Some(step)
	}

	fn next_in_str(&mut self, haystack: &str, offset: usize) -> Option<MatchStep> {
		if haystack.is_empty() {
			return None
		}

		self.state.reset_invalid(offset);

		let step = match self.kind {
			Whitespace::Ascii => self.next_ascii(haystack.as_bytes(), offset),
			Whitespace::Unicode => self.next_unicode(haystack, offset)
		};

		Some(step)
	}

	fn end(&mut self) -> Option<MatchStep> {
		match self.state.reset() {
			(_, 0) => None,
			(s, n) => Some(MatchStep::complete(s, n, 0))
		}
	}

	fn alignment(&self) -> Alignment {
		match self.kind {
			Whitespace::Ascii => Alignment::Byte,
			Whitespace::Unicode => Alignment::Char
		}
	}
}

struct ShrinkingWindows<'a, T> {
	last: &'a [T],
	windows: slice::Windows<'a, T>
}

impl<'a, T> Iterator for ShrinkingWindows<'a, T> {
	type Item = &'a [T];

	fn next(&mut self) -> Option<&'a [T]> {
		if let Some(window) = self.windows.next() {
			return Some(window)
		}

		(!self.last.is_empty()).then(|| {
			let last = self.last;
			self.last = &last[1..];
			last
		})
	}
}

fn find_partial(haystack: &[u8], needle: &[u8]) -> Option<(usize, usize)> {
	if let [byte] = needle {
		return haystack.iter().position(|b| b == byte).map(|i| (i, 1))
	}

	let last_start = if haystack.len() >= needle.len() {
		haystack.len() - needle.len() + 1
	} else {
		0
	};
	let windows = ShrinkingWindows {
		last: &haystack[last_start..],
		windows: haystack.windows(needle.len()),
	};
	windows.enumerate().find_map(|(i, window)|
		(window == &needle[..window.len()]).then(||
			(i, window.len())
		)
	)
}

fn extend_partial(haystack: &[u8], needle: &[u8]) -> Option<usize> {
	let len = min(haystack.len(), needle.len());
	(haystack[..len] == needle[..len]).then_some(len)
}

fn find_byte(haystack: &[u8], offset: usize, predicate: impl FnMut(&u8) -> bool) -> Option<MatchStep> {
	match haystack.iter().position(predicate) {
		Some(pos) => Some(MatchStep::complete(pos + offset, 1, pos + 1)),
		None if haystack.is_empty() => None,
		_ => Some(MatchStep::reject(haystack.len()))
	}
}

fn find_char(haystack: &[u8], offset: usize, mut predicate: impl FnMut(&char) -> bool) -> Option<MatchStep> {
	let checked = simdutf8::basic::from_utf8(haystack).expect(
		"haystack fragment should be valid UTF-8"
	);
	match checked.char_indices().find(|(_, c)| predicate(c)) {
		Some((pos, char)) => {
			let len = char.len_utf8();
			Some(MatchStep::complete(pos + offset, len, pos + len))
		}
		None if haystack.is_empty() => None,
		_ => Some(MatchStep::reject(haystack.len()))
	}
}
