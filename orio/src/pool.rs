// SPDX-License-Identifier: Apache-2.0

mod hack;

use std::cell::{BorrowMutError, RefCell, RefMut};
use std::default::Default;
use std::iter::Map;
use std::mem::MaybeUninit;
use std::ops::{DerefMut, Range};
use std::rc::Rc;
use std::result;
use itertools::Itertools;
use once_cell::sync::Lazy;
use super::segment::{alloc_block, Block, Seg, SIZE};

#[derive(Copy, Clone, Debug, thiserror::Error)]
#[error("failed to borrow the pool")]
pub struct PoolError;

pub type Result<T = ()> = result::Result<T, PoolError>;

impl From<BorrowMutError> for PoolError {
	fn from(_: BorrowMutError) -> Self { Self }
}

pub trait Pool<const N: usize = SIZE>: Clone {
	type Pool: MutPool<N> + ?Sized;
	type Ref<'p>: DerefMut<Target = Self::Pool> where Self: 'p;

	/// Gets a shared reference to the pool.
	fn get() -> Self;

	/// Borrows the pool mutably, locking it for the duration of the borrow.
	fn try_borrow(&self) -> Result<Self::Ref<'_>>;

	/// Claims a single segment.
	fn claim_one<'d>(&self) -> Result<Seg<'d, N>> {
		Ok(self.try_borrow()?.claim_one())
	}

	/// Claims `count` segments into `target`.
	fn claim_count<'d>(&self, target: &mut impl Extend<Seg<'d, N>>, count: usize) -> Result {
		use hack::MutPoolSpec;

		Ok(self.try_borrow()?.claim_count_spec(target, count))
	}

	/// Claims many segments into the container, at least `min_size` in total size.
	fn claim_size<'d>(&self, target: &mut impl Extend<Seg<'d, N>>, min_size: usize) -> Result {
		use hack::MutPoolSpec;

		Ok(self.try_borrow()?.claim_size_spec(target, min_size))
	}

	/// Collects a single segment back into the pool.
	fn collect_one(&self, segment: Seg<N>) -> Result {
		if segment.is_shared() { return Ok(()) }

		Ok(self.try_borrow()?.collect_one(segment))
	}

	/// Collects many segments back into the pool. Handling of shared segments is
	/// left up to implementation; the default implementation discards them.
	fn collect<'d>(&self, segments: impl IntoIterator<Item = Seg<'d, N>>) -> Result {
		use hack::MutPoolSpec;

		Ok(self.try_borrow()?.collect_spec(segments))
	}

	/// Clears segments from the pool to free space. The actual segment count to be
	/// cleared is left up to implementation.
	fn shed(&self) -> Result {
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
		self.claim_count(target, min_size.div_ceil(N))
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
pub struct DefaultPool(Vec<Box<[MaybeUninit<u8>; SIZE]>>);

#[derive(Clone)]
pub struct DefaultPoolContainer(Rc<RefCell<DefaultPool>>);

impl Default for DefaultPoolContainer {
	fn default() -> Self {
		Self(Rc::new(DefaultPool::default().into()))
	}
}

impl Pool<SIZE> for DefaultPoolContainer {
	type Pool = DefaultPool;
	type Ref<'p> = RefMut<'p, DefaultPool>;
	fn get() -> Self { pool() }

	fn try_borrow(&self) -> Result<Self::Ref<'_>> {
		Ok(self.0.try_borrow_mut()?)
	}
}

/// Clones a shared reference to the default segment pool.
#[inline]
pub fn pool() -> DefaultPoolContainer { POOL.clone() }

#[thread_local]
static POOL: Lazy<DefaultPoolContainer> = Lazy::new(DefaultPoolContainer::default);

impl DefaultPool {
	fn allocate(count: usize) -> Map<Range<usize>, fn(usize) -> Block> {
		(0..count).map(|_| alloc_block())
	}
}

impl MutPool for DefaultPool {
	fn claim_reserve(&mut self, count: usize) {
		let Self(vec) = self;
		let existing_count = count.min(vec.len());
		let allocate_count = count - existing_count;
		vec.extend(Self::allocate(allocate_count));
	}

	fn claim_one<'d>(&mut self) -> Seg<'d> {
		self.0.pop().unwrap_or_else(alloc_block).into()
	}

	fn claim_count<'d>(&mut self, target: &mut impl Extend<Seg<'d>>, count: usize) where Self: Sized {
		if count == 1 {
			target.extend_one(self.claim_one());
		} else {
			let Self(vec) = self;
			let existing_count = count.min(vec.len());
			let allocate_count = count - existing_count;
			target.extend(
				self.0
					.drain(..existing_count)
					.chain(Self::allocate(allocate_count))
					.map_into()
			);
		}
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
