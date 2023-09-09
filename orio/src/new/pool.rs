// SPDX-License-Identifier: Apache-2.0

mod hack;

use std::cell::{BorrowMutError, RefCell, RefMut};
use std::default::Default;
use std::ops::DerefMut;
use std::rc::Rc;
use itertools::Itertools;
use once_cell::sync::Lazy;
use crate::new::alloc_block;
use super::segment::{Seg, SIZE};

#[derive(Copy, Clone, Debug, thiserror::Error)]
#[error("failed to borrow the pool")]
pub struct PoolError;

impl From<BorrowMutError> for PoolError {
	fn from(_: BorrowMutError) -> Self { Self }
}

const fn calc_claim_count(min_size: usize, size: usize) -> usize {
	min_size.next_multiple_of(size) / size
}

pub trait Pool<const N: usize = SIZE>: Clone {
	type Pool: MutPool<N> + ?Sized;
	type Ref<'p>: DerefMut<Target = Self::Pool> where Self: 'p;

	/// Borrows the pool mutably, locking it for the duration of the borrow.
	fn try_borrow(&self) -> Result<Self::Ref<'_>, PoolError>;

	/// Claims a single segment.
	fn claim_one<'d>(&self) -> Result<Seg<'d, N>, PoolError> {
		Ok(self.try_borrow()?.claim_one())
	}

	/// Claims `count` segments into `target`.
	fn claim_count<'d>(&self, target: &mut impl Extend<Seg<'d, N>>, count: usize) -> Result<(), PoolError> {
		use hack::MutPoolSpec;

		Ok(self.try_borrow()?.claim_count_spec(target, count))
	}

	/// Claims many segments into the container, at least `min_size` in total size.
	fn claim_size<'d>(&self, target: &mut impl Extend<Seg<'d, N>>, min_size: usize) -> Result<(), PoolError> {
		use hack::MutPoolSpec;

		Ok(self.try_borrow()?.claim_size_spec(target, min_size))
	}

	/// Collects a single segment back into the pool.
	fn collect_one(&self, segment: Seg<N>) -> Result<(), PoolError> {
		if segment.is_shared() { return Ok(()) }

		Ok(self.try_borrow()?.collect_one(segment))
	}

	/// Collects many segments back into the pool. Handling of shared segments is
	/// left up to implementation; the default implementation discards them.
	fn collect<'d>(&self, segments: impl IntoIterator<Item = Seg<'d, N>>) -> Result<(), PoolError> {
		use hack::MutPoolSpec;

		Ok(self.try_borrow()?.collect_spec(segments))
	}

	/// Clears segments from the pool to free space. The actual segment count to be
	/// cleared is left up to implementation.
	fn shed(&self) -> Result<(), PoolError> {
		Ok(self.try_borrow()?.shed())
	}
}

/// A mutably-borrowed pool, usually from a [`RefCell`].
///
/// Note on object-safety: this trait is object-safe for single-segment operations,
/// but not for bulk operations.
pub trait MutPool<const N: usize = SIZE> {
	/// Reserves at least `count` segments in the pool.
	fn claim_reserve(&mut self, count: usize);

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
	fn claim_one<'d>(&mut self) -> Seg<'d, N>;

	/// Claims `count` segments into `target`.
	fn claim_count<'d>(&mut self, target: &mut impl Extend<Seg<'d, N>>, count: usize) where Self: Sized;

	/// Claims many segments into the container, at least `min_size` in total size.
	fn claim_size<'d>(&mut self, target: &mut impl Extend<Seg<'d, N>>, min_size: usize) where Self: Sized {
		self.claim_count(target, calc_claim_count(min_size, N))
	}

	/// Reserves space to collect at least `count` segments into the pool.
	fn collect_reserve(&mut self, count: usize);

	/// Collects a single segment back into the pool.
	fn collect_one(&mut self, segment: Seg<N>);

	/// Collects many segments back into the pool. Handling of shared segments is
	/// left up to implementation; the default implementation discards them.
	fn collect<'d>(&mut self, segments: impl IntoIterator<Item = Seg<'d, N>>) where Self: Sized;

	/// Clears segments from the pool to free space. The actual segment count to be
	/// cleared is left up to implementation.
	fn shed(&mut self);
}

#[derive(Default)]
pub struct DefaultPool(Vec<Box<[u8; SIZE]>>);

pub struct PoolContainer<const N: usize, P>(Rc<RefCell<P>>)
where P: MutPool<N> + ?Sized;

impl<const N: usize, P> From<Rc<RefCell<P>>> for PoolContainer<N, P>
	where P: MutPool<N> + ?Sized {
	fn from(pool: Rc<RefCell<P>>) -> Self {
		Self(pool)
	}
}

impl<const N: usize, P> From<RefCell<P>> for PoolContainer<N, P>
	where P: MutPool<N> + Sized {
	fn from(pool: RefCell<P>) -> Self {
		Rc::new(pool).into()
	}
}

impl<const N: usize, P> From<P> for PoolContainer<N, P>
	where P: MutPool<N> + Sized {
	fn from(pool: P) -> Self {
		RefCell::new(pool).into()
	}
}

impl Default for DefaultPoolContainer {
	fn default() -> Self {
		DefaultPool::default().into()
	}
}

impl<const N: usize, P> Clone for PoolContainer<N, P>
	where P: MutPool<N> + ?Sized {
	fn clone(&self) -> Self {
		self.0.clone().into()
	}
}

impl<const N: usize, P> Pool<N> for PoolContainer<N, P>
where P: MutPool<N> + ?Sized,
	  for<'p> P: 'p {
	type Pool = P;
	type Ref<'p> = RefMut<'p, P>;

	fn try_borrow(&self) -> Result<Self::Ref<'_>, PoolError> {
		Ok(self.0.try_borrow_mut()?)
	}
}

pub(crate) type DefaultPoolContainer = PoolContainer<SIZE, DefaultPool>;

/// Clones a shared reference to the default segment pool.
pub fn pool() -> DefaultPoolContainer { POOL.clone() }

#[thread_local]
static POOL: Lazy<DefaultPoolContainer> = Lazy::new(PoolContainer::default);

impl MutPool for DefaultPool {
	fn claim_reserve(&mut self, mut count: usize) {
		let Self(vec) = self;
		let len = vec.len();
		vec.reserve(count);

		count = vec.len().saturating_sub(len);
		let vec = &mut vec[len..];
		vec[..count].fill_with(alloc_block);
	}

	fn claim_one<'d>(&mut self) -> Seg<'d> {
		self.0.pop().unwrap_or_else(alloc_block).into()
	}

	fn claim_count<'d>(&mut self, target: &mut impl Extend<Seg<'d>>, count: usize) where Self: Sized {
		self.claim_reserve(count);
		target.extend(
			self.0
				.drain(..count)
				.map_into()
		)
	}

	fn collect_reserve(&mut self, count: usize) {
		self.0.reserve(count)
	}

	fn collect_one(&mut self, segment: Seg) {
		if let Some(block) = segment.into_block() {
			self.0.push(block)
		}
	}

	fn collect<'d>(&mut self, segments: impl IntoIterator<Item = Seg<'d>>) {
		self.0.extend(segments.into_iter().filter_map(Seg::into_block))
	}

	fn shed(&mut self) { self.0.clear() }
}
