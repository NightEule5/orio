// SPDX-License-Identifier: Apache-2.0

use std::collections::{VecDeque, vec_deque};
use super::Element;
use super::pool::MutPool;
use super::segment::Seg;

pub type Drain<'a, const N: usize, T> = vec_deque::Drain<'a, Seg<'a, N, T>>;

pub struct RingBuf<'a, const N: usize, T: Element> {
	ring: VecDeque<Seg<'a, N, T>>
}

impl<'a, const N: usize, T: Element> RingBuf<'a, N, T> {
	fn reserve(&mut self, count: usize, pool: &mut dyn MutPool<N, T>) {

	}

	pub fn extend_back(&mut self, iter: impl IntoIterator<Item = Seg<'a, N, T>>) {
		self.ring.extend(iter)
	}
}
