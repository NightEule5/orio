// SPDX-License-Identifier: Apache-2.0

use std::cmp::min;

pub trait SliceExt<T> {
	fn copy_from_pair(&mut self, pair: (&[T], &[T])) -> usize;
	fn copy_into_pair(&self, pair: (&mut [T], &mut [T])) -> usize;
}

impl<T: Copy> SliceExt<T> for [T] {
	fn copy_from_pair(&mut self, (a, mut b): (&[T], &[T])) -> usize {
		let count = min(a.len() + b.len(), self.len());
		let split = min(a.len(), count);
		let (buf_a, buf_b) = self[..count].split_at_mut(split);
		b = &b[..count - split];
		buf_a.copy_from_slice(a);
		buf_b.copy_from_slice(b);
		count
	}

	fn copy_into_pair(&self, (a, mut b): (&mut [T], &mut [T])) -> usize {
		let count = min(a.len() + b.len(), self.len());
		let split = min(a.len(), count);
		let (buf_a, buf_b) = self[..count].split_at(split);
		b = &mut b[..count - split];
		a.copy_from_slice(buf_a);
		b.copy_from_slice(buf_b);
		count
	}
}
