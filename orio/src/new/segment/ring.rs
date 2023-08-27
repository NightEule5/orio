// SPDX-License-Identifier: Apache-2.0

use std::collections::{VecDeque, vec_deque};
use crate::new::Element;
use crate::new::MutPool;
use crate::new::Seg;

pub type Drain<'a, const N: usize, T> = vec_deque::Drain<'a, Seg<'a, N, T>>;


pub struct SegRing<'a, const N: usize, T: Element> {
	ring: VecDeque<Seg<'a, N, T>>
}

impl<'a, const N: usize, T: Element> SegRing<'a, N, T> {
	fn reserve(&mut self, count: usize, pool: &mut dyn MutPool<N, T>) {

	}

	pub fn extend_back(&mut self, iter: impl IntoIterator<Item = Seg<'a, N, T>>) {
		self.ring.extend(iter)
	}
}
