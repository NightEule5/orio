// SPDX-License-Identifier: Apache-2.0

use std::cell::{RefCell, RefMut};
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::result;
use itertools::Itertools;
use once_cell::sync::Lazy;
use crate::{ErrorBox, SEGMENT_SIZE};
use crate::segment::Segment;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
	#[error("pool already being borrowed")]
	Borrowed,
	#[error("out of memory")]
	OutOfMemory,
	#[error(transparent)]
	Other(#[from] ErrorBox),
}

#[derive(Copy, Clone, Debug, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub enum Context {
	Claim,
	Collect,
	Shed,
}

pub type Result<T = (), E = Error> = result::Result<T, E>;

pub type DefaultPool = LocalPool;

/// A segment pool.
pub trait Pool: Sized {
	/// Claims a single segment.
	fn claim_one(&mut self) -> Segment;

	/// Claims `count` segments into `target`.
	fn claim_count(&mut self, target: &mut impl Extend<Segment>, count: usize) {
		target.extend_reserve(count);
		for _ in 0..count {
			target.extend_one(self.claim_one())
		}
	}

	/// Claims many segments into the container, at least `min_size` in total size.
	fn claim_size(&mut self, target: &mut impl Extend<Segment>, min_size: usize) {
		let count = min_size.next_multiple_of(SEGMENT_SIZE) / SEGMENT_SIZE;

		self.claim_count(target, count)
	}

	/// Collects a single segment back into the pool.
	fn collect_one(&mut self, segment: Segment);

	/// Collects many segments back into the pool. Handling of shared segments is
	/// left up to implementation; the default implementation discards them.
	fn collect(&mut self, segments: impl IntoIterator<Item = Segment>) {
		for mut seg in segments {
			if !seg.is_shared() {
				seg.clear();
				self.collect_one(seg);
			}
		}
	}

	/// Clears segments from the pool to free space. The actual segment count to be
	/// cleared is left up to implementation.
	fn shed(&mut self);
}

/// A shared, internally-mutable segment pool.
pub trait SharedPool {
	/// Gets a shared instance of the pool.
	fn get() -> Self;
	
	/// Locks the pool for the duration of the borrow. Useful for batch operations.
	fn lock(&self) -> Result<impl DerefMut<Target = impl Pool> + '_>;

	/// Claims a single segment.
	fn claim_one(&self) -> Result<Segment> {
		Ok(self.lock()?.claim_one())
	}

	/// Claims `count` segments into `target`.
	fn claim_count(&self, target: &mut impl Extend<Segment>, count: usize) -> Result {
		Ok(self.lock()?.claim_count(target, count))
	}

	/// Claims many segments into the container, at least `min_size` in total size.
	fn claim_size(&self, target: &mut impl Extend<Segment>, min_size: usize) -> Result {
		Ok(self.lock()?.claim_size(target, min_size))
	}

	/// Collects a single segment back into the pool.
	fn collect_one(&self, segment: Segment) -> Result {
		self.lock()?.collect_one(segment);
		Ok(())
	}

	/// Collects many segments back into the pool. Handling of shared segments is
	/// left up to implementation; the default implementation discards them.
	fn collect(&self, segments: impl IntoIterator<Item = Segment>) -> Result {
		self.lock()?.deref_mut().collect(segments);
		Ok(())
	}

	/// Clears segments from the pool to free space. The actual segment count to be
	/// cleared is left up to implementation.
	fn shed(&self) -> Result {
		self.lock()?.shed();
		Ok(())
	}
}

/// A basic [`Pool`] implementation using a [`Vec`].
#[derive(Default)]
pub struct BasicPool {
	segments: Vec<Segment>
}

impl Pool for BasicPool {
	fn claim_one(&mut self) -> Segment {
		self.claim().unwrap_or_default()
	}

	fn claim_count(&mut self, target: &mut impl Extend<Segment>, count: usize) {
		let ref mut segments = self.segments;
		segments.resize_with(segments.len() + count, Default::default);
		target.extend(segments.drain(..count));
	}

	fn collect_one(&mut self, segment: Segment) {
		self.segments.push(segment);
	}

	fn collect(&mut self, segments: impl IntoIterator<Item = Segment>) {
		self.segments.extend(
			segments.into_iter()
					.filter(|seg| !seg.is_shared())
					.update(|seg| seg.clear())
		);
	}

	fn shed(&mut self) {
		self.segments.clear();
	}
}

impl BasicPool {
	fn claim(&mut self) -> Option<Segment> {
		self.segments.pop()
	}
}

/// A [`Pool`] implementation with no storage. Instead, collected segments are
/// dropped and claimed segments are created on-demand.
#[derive(Copy, Clone, Default)]
pub struct VoidPool;

impl Deref for VoidPool {
	type Target = Self;

	fn deref(&self) -> &Self { self }
}

impl DerefMut for VoidPool {
	fn deref_mut(&mut self) -> &mut Self { self }
}

impl Pool for VoidPool {
	fn claim_one(&mut self) -> Segment { Segment::default() }

	fn collect_one(&mut self, _: Segment) { }

	fn shed(&mut self) { }
}

impl SharedPool for VoidPool {
	fn get() -> Self { Self }

	fn lock(&self) -> Result<Self> { Ok(*self) }
}

#[thread_local]
static LOCAL: Lazy<LocalPool> = Lazy::new(LocalPool::default);

/// The default thread-local [`SharedPool`] implementation.
#[derive(Clone)]
pub struct LocalPool {
	inner: Rc<RefCell<BasicPool>>
}

impl Default for LocalPool {
	fn default() -> Self {
		Self { inner: Rc::new(RefCell::default()) }
	}
}

impl SharedPool for LocalPool {
	fn get() -> Self { LOCAL.clone() }

	fn lock(&self) -> Result<RefMut<'_, BasicPool>> {
		self.inner
			.try_borrow_mut()
			.map_err(|_| Error::Borrowed)
	}
}
