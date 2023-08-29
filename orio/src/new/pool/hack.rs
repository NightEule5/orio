// SPDX-License-Identifier: Apache-2.0

//! Ugly hacks to enable bulk Pool trait methods for references to unsized MutPool
//! impls. Uses the claim_count and collect methods if sized, falling back to loops
//! if not. Only usable from the Pool trait where the types are known.

use super::{Element, MutPool, Seg, calc_claim_count};

pub trait MutPoolSpec<const N: usize, T: Element> {
	fn claim_size_spec<'d>(&mut self, target: &mut impl Extend<Seg<'d, N, T>>, min_size: usize);

	fn claim_count_spec<'d>(&mut self, target: &mut impl Extend<Seg<'d, N, T>>, count: usize);

	fn collect_spec<'d>(&mut self, source: impl IntoIterator<Item = Seg<'d, N, T>>);
}

impl<const N: usize, T: Element, P: MutPool<N, T>> MutPoolSpec<N, T> for P {
	fn claim_size_spec<'d>(&mut self, target: &mut impl Extend<Seg<'d, N, T>>, min_size: usize) {
		self.claim_size(target, min_size)
	}

	fn claim_count_spec<'d>(&mut self, target: &mut impl Extend<Seg<'d, N, T>>, count: usize) {
		self.claim_count(target, count)
	}

	fn collect_spec<'d>(&mut self, source: impl IntoIterator<Item = Seg<'d, N, T>>) {
		self.collect(source)
	}
}

impl<const N: usize, T: Element, P: MutPool<N, T> + ?Sized> MutPoolSpec<N, T> for P {
	default fn claim_size_spec<'d>(&mut self, target: &mut impl Extend<Seg<'d, N, T>>, min_size: usize) {
		self.claim_count_spec(target, calc_claim_count(min_size, N))
	}

	default fn claim_count_spec<'d>(&mut self, target: &mut impl Extend<Seg<'d, N, T>>, count: usize) {
		target.extend_reserve(count);
		for _ in 0..count {
			target.extend_one(self.claim_one())
		}
	}

	default fn collect_spec<'d>(&mut self, source: impl IntoIterator<Item = Seg<'d, N, T>>) {
		for seg in source {
			self.collect_one(seg)
		}
	}
}
