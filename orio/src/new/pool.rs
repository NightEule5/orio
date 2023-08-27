// SPDX-License-Identifier: Apache-2.0

use std::cell::{BorrowMutError, RefCell, RefMut};
use std::cmp::min;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use itertools::Itertools;
use once_cell::sync::Lazy;
use super::element::Element;
use super::segment::{Block, Seg, SIZE};
use crate::SEGMENT_SIZE;

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("failed to borrow the pool")]
	Borrow(#[from] BorrowMutError),
}

pub trait Pool<const N: usize, T: Element>: Clone {
	type Pool: MutPool<N, T> + ?Sized;
	type Ref<'p>: DerefMut<Target = Self::Pool> where Self: 'p;

	/// Borrows the pool mutably, locking it for the duration of the borrow.
	fn try_borrow(&self) -> Result<Self::Ref<'_>, Error>;

	/// Claims a single segment.
	fn claim_one<'d>(&self) -> Result<Seg<'d, N, T>, Error> {
		Ok(self.try_borrow()?.claim_one())
	}

	/// Claims `count` segments into `target`.
	fn claim_count<'d>(&self, target: &mut impl Extend<Seg<'d, N, T>>, count: usize) -> Result<(), Error>
	where for<'p> <Self::Ref<'p> as Deref>::Target: Sized {
		Ok(self.try_borrow()?.claim_count(target, count))
	}

	/// Claims many segments into the container, at least `min_size` in total size.
	fn claim_size<'d>(&self, target: &mut impl Extend<Seg<'d, N, T>>, min_size: usize) -> Result<(), Error>
	where for<'p> <Self::Ref<'p> as Deref>::Target: Sized {
		Ok(self.try_borrow()?.claim_size(target, min_size))
	}

	/// Collects a single segment back into the pool.
	fn collect_one(&self, segment: Seg<N, T>) -> Result<(), Error> {
		if !segment.is_writable() { return Ok(()) }

		Ok(self.try_borrow()?.collect_one(segment))
	}

	/// Collects many segments back into the pool. Handling of shared segments is
	/// left up to implementation; the default implementation discards them.
	fn collect<'d>(&self, segments: impl IntoIterator<Item = Seg<'d, N, T>>) -> Result<(), Error>
	where for<'p> <Self::Ref<'p> as Deref>::Target: Sized {
		Ok(self.try_borrow()?.collect(segments))
	}

	/// Clears segments from the pool to free space. The actual segment count to be
	/// cleared is left up to implementation.
	fn shed(&self) -> Result<(), Error> {
		Ok(self.try_borrow()?.shed())
	}
}

/// A mutably-borrowed pool, usually from a [`RefCell`].
///
/// Note on object-safety: this trait is object-safe for single-segment operations,
/// but not for bulk operations.
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
	fn claim_count<'d>(&mut self, target: &mut impl Extend<Seg<'d, N, T>>, count: usize) where Self: Sized;

	/// Claims many segments into the container, at least `min_size` in total size.
	fn claim_size<'d>(&mut self, target: &mut impl Extend<Seg<'d, N, T>>, min_size: usize) where Self: Sized {
		let count = min_size.next_multiple_of(SEGMENT_SIZE) / SEGMENT_SIZE;

		self.claim_count(target, count)
	}

	/// Collects a single segment back into the pool.
	fn collect_one(&mut self, segment: Seg<N, T>);

	/// Collects many segments back into the pool. Handling of shared segments is
	/// left up to implementation; the default implementation discards them.
	fn collect<'d>(&mut self, segments: impl IntoIterator<Item = Seg<'d, N, T>>) where Self: Sized;

	/// Clears segments from the pool to free space. The actual segment count to be
	/// cleared is left up to implementation.
	fn shed(&mut self);
}

#[derive(Default)]
pub struct DefaultPool(Vec<Block<SIZE, u8>>);

pub struct PoolContainer<const N: usize, T, P>(Rc<RefCell<P>>, PhantomData<T>)
where T: Element,
	  P: MutPool<N, T> + ?Sized;

impl<const N: usize, T, P> From<Rc<RefCell<P>>> for PoolContainer<N, T, P>
	where T: Element,
		  P: MutPool<N, T> + ?Sized {
	fn from(pool: Rc<RefCell<P>>) -> Self {
		Self(pool, PhantomData)
	}
}

impl<const N: usize, T, P> From<RefCell<P>> for PoolContainer<N, T, P>
	where T: Element,
		  P: MutPool<N, T> + Sized {
	fn from(pool: RefCell<P>) -> Self {
		Rc::new(pool).into()
	}
}

impl<const N: usize, T, P> From<P> for PoolContainer<N, T, P>
	where T: Element,
		  P: MutPool<N, T> + Sized {
	fn from(pool: P) -> Self {
		RefCell::new(pool).into()
	}
}

impl Default for PoolContainer<SIZE, u8, DefaultPool> {
	fn default() -> Self {
		DefaultPool::default().into()
	}
}

impl<const N: usize, T, P> Clone for PoolContainer<N, T, P>
	where T: Element,
		  P: MutPool<N, T> + ?Sized {
	fn clone(&self) -> Self {
		self.0.clone().into()
	}
}

impl<const N: usize, T, P> Pool<N, T> for PoolContainer<N, T, P>
where T: Element,
	  P: MutPool<N, T> + ?Sized,
	  for<'p> P: 'p {
	type Pool = P;
	type Ref<'p> = RefMut<'p, P>;

	fn try_borrow(&self) -> Result<Self::Ref<'_>, Error> {
		Ok(self.0.try_borrow_mut()?)
	}
}

/// Clones a shared reference to the default segment pool.
pub fn pool() -> PoolContainer<SIZE, u8, DefaultPool> { POOL.clone() }

#[thread_local]
static POOL: Lazy<PoolContainer<SIZE, u8, DefaultPool>> = Lazy::new(PoolContainer::default);

impl MutPool<SIZE, u8> for DefaultPool {
	fn claim_one<'d>(&mut self) -> Seg<'d, SIZE, u8> {
		self.0.pop().unwrap_or_else(|| Box::pin([0; SIZE])).into()
	}

	fn claim_count<'d>(&mut self, target: &mut impl Extend<Seg<'d, SIZE, u8>>, count: usize) where Self: Sized {
		let existing = min(count, self.0.len());
		let allocate = count - existing;
		target.extend(
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

	fn collect<'d>(&mut self, segments: impl IntoIterator<Item = Seg<'d, SIZE, u8>>) {
		self.0.extend(segments.into_iter().filter_map(Seg::into_block))
	}

	fn shed(&mut self) { self.0.clear() }
}

// A workaround for bulk claiming and collection on a dyn pool. References are
// sized, so implementing for a mutable reference exposes these for the trait object.
// The catch is these cannot be implemented as bulk operations, they have to use
// loops.
impl<'a> MutPool<SIZE, u8> for &'a mut dyn MutPool<SIZE, u8> {
	fn claim_one<'d>(&mut self) -> Seg<'d, SIZE, u8> {
		(*self).claim_one()
	}

	fn claim_count<'d>(&mut self, target: &mut impl Extend<Seg<'d, SIZE, u8>>, count: usize)
	where Self: Sized {
		target.extend((0..count).map(|_| self.claim_one()))
	}

	fn collect_one(&mut self, segment: Seg<SIZE, u8>) {
		(*self).collect_one(segment)
	}

	fn collect<'d>(&mut self, segments: impl IntoIterator<Item = Seg<'d, SIZE, u8>>) where Self: Sized {
		for seg in segments {
			self.collect_one(seg)
		}
	}

	fn shed(&mut self) { (*self).shed() }
}
