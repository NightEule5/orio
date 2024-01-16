// SPDX-License-Identifier: Apache-2.0

//! Ugly hacks to enable bulk Pool trait methods for references to unsized MutPool
//! impls. Uses the claim_count and collect methods if sized, falling back to loops
//! if not. Only usable from the Pool trait where the types are known.

use super::{MutPool, Seg};

pub trait MutPoolSpec<const N: usize> {
	fn claim_size_spec<'d>(&mut self, target: &mut impl Extend<Seg<'d, N>>, min_size: usize);

	fn claim_count_spec<'d>(&mut self, target: &mut impl Extend<Seg<'d, N>>, count: usize);

	fn collect_spec<'d>(&mut self, source: impl IntoIterator<Item = Seg<'d, N>>);
}

impl<const N: usize, P: MutPool<N>> MutPoolSpec<N> for P {
	fn claim_size_spec<'d>(&mut self, target: &mut impl Extend<Seg<'d, N>>, min_size: usize) {
		self.claim_size(target, min_size)
	}

	fn claim_count_spec<'d>(&mut self, target: &mut impl Extend<Seg<'d, N>>, count: usize) {
		self.claim_count(target, count)
	}

	fn collect_spec<'d>(&mut self, source: impl IntoIterator<Item = Seg<'d, N>>) {
		self.collect(source)
	}
}

impl<const N: usize, P: MutPool<N> + ?Sized> MutPoolSpec<N> for P {
	default fn claim_size_spec<'d>(&mut self, target: &mut impl Extend<Seg<'d, N>>, min_size: usize) {
		self.claim_count_spec(target, min_size.div_ceil(N))
	}

	default fn claim_count_spec<'d>(&mut self, target: &mut impl Extend<Seg<'d, N>>, count: usize) {
		target.extend_reserve(count);
		for _ in 0..count {
			target.extend_one(self.claim_one())
		}
	}

	default fn collect_spec<'d>(&mut self, source: impl IntoIterator<Item = Seg<'d, N>>) {
		for seg in source {
			self.collect_one(seg)
		}
	}
}

impl<const N: usize, P: MutPool<N> + ?Sized> MutPool<N> for &mut P {
	#[inline]
	fn claim_reserve(&mut self, count: usize) {
		P::claim_reserve(self, count);
	}

	#[inline]
	fn claim_one<'d>(&mut self) -> Seg<'d, N> {
		P::claim_one(self)
	}

	#[inline]
	fn claim_count<'d>(&mut self, target: &mut impl Extend<Seg<'d, N>>, count: usize) where Self: Sized {
		P::claim_count_spec(self, target, count);
	}

	#[inline]
	fn claim_size<'d>(&mut self, target: &mut impl Extend<Seg<'d, N>>, min_size: usize) where Self: Sized {
		P::claim_size_spec(self, target, min_size);
	}

	#[inline]
	fn collect_reserve(&mut self, count: usize) {
		P::collect_reserve(self, count);
	}

	#[inline]
	fn collect_one(&mut self, segment: Seg<N>) {
		P::collect_one(self, segment);
	}

	#[inline]
	fn collect<'d>(&mut self, segments: impl IntoIterator<Item = Seg<'d, N>>) where Self: Sized {
		P::collect_spec(self, segments);
	}

	#[inline]
	fn shed(&mut self) {
		P::shed(self);
	}
}
