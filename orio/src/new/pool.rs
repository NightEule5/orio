// SPDX-License-Identifier: Apache-2.0

use std::cell::{BorrowMutError, RefCell, RefMut};
use std::cmp::min;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use itertools::Itertools;
use once_cell::sync::Lazy;
use super::element::Element;
use super::segment::{Block, Seg, SIZE};
use crate::new::ring;
use crate::new::ring::RingBuf;
use crate::SEGMENT_SIZE;

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("failed to borrow the pool")]
	Borrow(#[from] BorrowMutError),
}

pub trait Pool<const N: usize, T: Element>
where for<'p> <Self::Ref<'p> as Deref>::Target: MutPool<N, T> {
	type Ref<'p>: DerefMut where Self: 'p;

	/// Borrows the pool mutably, locking it for the duration of the borrow.
	fn try_borrow(&self) -> Result<Self::Ref<'_>, Error>;

	/// Claims a single segment.
	fn claim_one<'d>(&self) -> Result<Seg<'d, N, T>, Error> {
		Ok(self.try_borrow()?.claim_one())
	}

	/// Claims `count` segments into `target`.
	fn claim_count(&self, target: &mut RingBuf<N, T>, count: usize) -> Result<(), Error> {
		Ok(self.try_borrow()?.claim_count(target, count))
	}

	/// Claims many segments into the container, at least `min_size` in total size.
	fn claim_size(&self, target: &mut RingBuf<N, T>, min_size: usize) -> Result<(), Error> {
		Ok(self.try_borrow()?.claim_size(target, min_size))
	}

	/// Collects a single segment back into the pool.
	fn collect_one(&self, segment: Seg<N, T>) -> Result<(), Error> {
		if !segment.is_writable() { return Ok(()) }

		Ok(self.try_borrow()?.collect_one(segment))
	}

	/// Collects many segments back into the pool. Handling of shared segments is
	/// left up to implementation; the default implementation discards them.
	fn collect(&self, segments: ring::Drain<N, T>) -> Result<(), Error> {
		Ok(self.try_borrow()?.collect(segments))
	}

	/// Clears segments from the pool to free space. The actual segment count to be
	/// cleared is left up to implementation.
	fn shed(&self) -> Result<(), Error> {
		Ok(self.try_borrow()?.shed())
	}
}

pub trait MutPool<const N: usize, T: Element> {
	/// Claims a single segment.
	///
	/// Lifetime note: the returned segment *must* be valid for any lifetime. This
	/// means all ownership of the data in the segment is given to the caller. The
	/// segment's internal buffer is guaranteed not to be a borrowed slice. It may
	/// not be writable, since this lifetime doesn't preclude a shared `Rc` block
	/// nor a boxed array.
	///
	/// On the implementation side, this makes it impossible to store segments in
	/// the pool as-is. The default implementation only stores uniquely-owned `Rc`
	/// blocks, then reconstructs segments from them.
	fn claim_one<'d>(&mut self) -> Seg<'d, N, T>;

	/// Claims `count` segments into `target`.
	fn claim_count(&mut self, target: &mut RingBuf<N, T>, count: usize);

	/// Claims many segments into the container, at least `min_size` in total size.
	fn claim_size(&mut self, target: &mut RingBuf<N, T>, min_size: usize) {
		let count = min_size.next_multiple_of(SEGMENT_SIZE) / SEGMENT_SIZE;

		self.claim_count(target, count)
	}

	/// Collects a single segment back into the pool.
	fn collect_one(&mut self, segment: Seg<N, T>);

	/// Collects many segments back into the pool. Handling of shared segments is
	/// left up to implementation; the default implementation discards them.
	fn collect(&mut self, segments: ring::Drain<N, T>);

	/// Clears segments from the pool to free space. The actual segment count to be
	/// cleared is left up to implementation.
	fn shed(&mut self);
}

#[derive(Default)]
pub struct DefaultPool(Vec<Block<SIZE, u8>>);

#[derive(Clone)]
pub struct PoolContainer<const N: usize, T: Element>(
	Rc<RefCell<dyn MutPool<N, T>>>
);

impl<const N: usize, T: Element> PoolContainer<N, T> {
	fn new(pool: impl MutPool<N, T> + 'static) -> Self {
		Self(Rc::new(RefCell::new(pool)))
	}
}

impl Default for PoolContainer<SIZE, u8> {
	fn default() -> Self {
		Self::new(DefaultPool::default())
	}
}

impl<const N: usize, T: Element> Pool<N, T> for PoolContainer<N, T> {
	type Ref<'p> = RefMut<'p, dyn MutPool<N, T>>;

	fn try_borrow(&self) -> Result<Self::Ref<'_>, Error> {
		Ok(self.0.try_borrow_mut()?)
	}
}

/// Clones a shared reference to the default segment pool.
pub fn pool() -> PoolContainer<SIZE, u8> { POOL.clone() }

#[thread_local]
static POOL: Lazy<PoolContainer<SIZE, u8>> = Lazy::new(PoolContainer::default);

impl MutPool<SIZE, u8> for DefaultPool {
	fn claim_one<'d>(&mut self) -> Seg<'d, SIZE, u8> {
		self.0.pop().unwrap_or_else(|| Box::pin([0; SIZE])).into()
	}

	fn claim_count(&mut self, target: &mut RingBuf<SIZE, u8>, count: usize) {
		let existing = min(count, self.0.len());
		let allocate = count - existing;
		target.extend_back(
			self.0
				.drain(..existing)
				.chain((0..allocate).map(|_| Box::pin([0; SIZE])))
				.map_into()
		)
	}

	fn collect_one(&mut self, segment: Seg<SIZE, u8>) {
		if let Some(block) = segment.into_block() {
			self.0.push(block)
		}
	}

	fn collect(&mut self, segments: ring::Drain<SIZE, u8>) {
		self.0.extend(segments.filter_map(Seg::into_block))
	}

	fn shed(&mut self) { self.0.clear() }
}
