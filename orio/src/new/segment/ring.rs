// SPDX-License-Identifier: Apache-2.0

use std::collections::VecDeque;
use crate::new::{Element, Pool, Seg};

pub(crate) struct SegRing<'a, const N: usize, T: Element, P: Pool<N, T>> {
	ring: VecDeque<Seg<'a, N, T>>,
	pool: P,
}

impl<'a, const N: usize, T: Element, P: Pool<N, T>> SegRing<'a, N, T, P> {
}
