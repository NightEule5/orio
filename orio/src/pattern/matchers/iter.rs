// SPDX-License-Identifier: Apache-2.0

use std::borrow::Cow;
use std::ops::Range;
use crate::pattern::{Matcher, MatchStep};
use crate::pattern::internal::AlignedUtf8Iter;

// Todo: this code is very trait-constraint heavy and ugly, refactor.

pub struct FragmentSteps<'a, M, F: ToOwned + ?Sized> {
	matcher: M,
	current: Cow<'a, F>,
	matched: usize,
	offset: usize,
}

pub struct StepsInner<'a, I: Iterator, M, F: ToOwned + ?Sized> {
	fragments: I,
	steps: FragmentSteps<'a, M, F>,
}

pub type StrSteps<'a, I, M> = StepsInner<'a, I, M, str>;

pub enum Steps<'a, I: Iterator<Item = &'a [u8]>, M> {
	Charwise(StepsInner<'a, AlignedUtf8Iter<'a, I>, M, str>),
	Bytewise(StepsInner<'a, I, M, [u8]>)
}

impl<M: Matcher> Iterator for FragmentSteps<'_, M, [u8]> {
	type Item = MatchStep;

	fn next(&mut self) -> Option<Self::Item> {
		let Self { matcher, current, matched, offset } = self;
		let step = matcher.next(&current[*matched..], *offset)?;
		let consumed = step.consumed_bytes(..current.len() - *matched);
		*matched += consumed;
		Some(step)
	}
}

impl<M: Matcher> Iterator for FragmentSteps<'_, M, str> {
	type Item = MatchStep;

	fn next(&mut self) -> Option<Self::Item> {
		let Self { matcher, current, matched, offset } = self;
		let step = matcher.next_in_str(&current[*matched..], *offset)?;
		let consumed = step.consumed_bytes(..current.len() - *matched);
		*matched += consumed;
		Some(step)
	}
}

impl<'a, I: Iterator, M: Matcher, F: ToOwned + ?Sized> StepsInner<'a, I, M, F>
where FragmentSteps<'a, M, F>: Iterator<Item = MatchStep>,
	  Cow<'a, F>: From<I::Item> {
	fn next_spec(&mut self) -> Option<MatchStep> where F: Slice {
		let Self { fragments, steps } = self;
		loop {
			let len = steps.current.len_spec();
			if steps.matched >= len {
				let Some(fragment) = fragments.next() else {
					return steps.matcher.end()
				};
				steps.current = fragment.into();
				steps.offset += len;
				steps.matched = 0;
			}

			if let Some(step) = steps.next() {
				break Some(step)
			}
		}
	}
}

impl<'a, I: Iterator, M: Matcher> Iterator for StepsInner<'a, I, M, [u8]>
where Cow<'a, [u8]>: From<I::Item> {
	type Item = MatchStep;

	#[inline]
	fn next(&mut self) -> Option<Self::Item> {
		self.next_spec()
	}
}

impl<'a, I: Iterator, M: Matcher> Iterator for StepsInner<'a, I, M, str>
where Cow<'a, str>: From<I::Item> {
	type Item = MatchStep;

	#[inline]
	fn next(&mut self) -> Option<Self::Item> {
		self.next_spec()
	}
}

impl<'a, I: Iterator<Item = &'a [u8]>, M: Matcher> Iterator for Steps<'a, I, M> {
	type Item = MatchStep;

	fn next(&mut self) -> Option<Self::Item> {
		match self {
			Self::Bytewise(inner) => inner.next(),
			Self::Charwise(inner) => inner.next()
		}
	}
}

impl<'a, M: Matcher, F: ToOwned + ?Sized> FragmentSteps<'a, M, F> {
	pub(super) fn new(matcher: M, fragment: Cow<'a, F>) -> Self {
		Self {
			matcher,
			current: fragment,
			matched: 0,
			offset: 0,
		}
	}
}

impl<'a, I: Iterator, M: Matcher, F: ToOwned<Owned: Default> + ?Sized> StepsInner<'a, I, M, F> {
	pub(super) fn new(fragments: I, matcher: M) -> Self {
		Self {
			fragments,
			steps: FragmentSteps::new(matcher, Default::default()),
		}
	}
}

pub struct CompleteMatches<'a, I: Iterator, M, F: ToOwned + ?Sized>(StepsInner<'a, I, M, F>);

pub enum Matches<'a, I: Iterator<Item = &'a [u8]>, M> {
	Charwise(CompleteMatches<'a, AlignedUtf8Iter<'a, I>, M, str>),
	Bytewise(CompleteMatches<'a, I, M, [u8]>),
}

pub type StrMatches<'a, I, M> = CompleteMatches<'a, I, M, str>;

impl<'a, I: Iterator, M: Matcher, F: ToOwned<Owned: Default> + ?Sized> CompleteMatches<'a, I, M, F> {
	pub(super) fn new(steps: StepsInner<'a, I, M, F>) -> Self {
		Self(steps)
	}
}

impl<'a, I: Iterator, M, F: ToOwned + ?Sized> Iterator for CompleteMatches<'a, I, M, F>
where StepsInner<'a, I, M, F>: Iterator<Item = MatchStep> {
	type Item = Range<usize>;

	fn next(&mut self) -> Option<Self::Item> {
		self.0.find_map(MatchStep::into_range)
	}
}

impl<'a, I: Iterator<Item = &'a [u8]>, M: Matcher> Iterator for Matches<'a, I, M> {
	type Item = Range<usize>;

	fn next(&mut self) -> Option<Self::Item> {
		match self {
			Self::Charwise(inner) => inner.next(),
			Self::Bytewise(inner) => inner.next()
		}
	}
}

trait Slice: ToOwned {
	fn len_spec(&self) -> usize;
}

impl Slice for [u8] {
	#[inline]
	fn len_spec(&self) -> usize { self.len() }
}

impl Slice for str {
	#[inline]
	fn len_spec(&self) -> usize { self.len() }
}
