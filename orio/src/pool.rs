// Copyright 2023 Strixpyrr
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#[cfg(feature = "shared-pool")]
pub use shared::*;

use std::cell::{RefCell, RefMut};
use std::cmp::min;
use std::error;
use std::iter::repeat_with;
use std::rc::Rc;
use cfg_if::cfg_if;
use once_cell::unsync::Lazy;
use crate::{DEFAULT_SEGMENT_SIZE, Error, Segment, Segments};

pub trait Pool<const N: usize> {
	type Error: error::Error + 'static;

	/// Claims a single segment.
	fn claim_one(&self) -> Result<Segment<N>, Self::Error>;

	/// Claims `count` segments into the container.
	fn claim_count(&self, segments: &mut Segments<N>, count: usize) -> Result<(), Self::Error> {
		for _ in 0..count {
			segments.push(self.claim_one()?)
		}

		Ok(())
	}

	/// Claims many segments into the container, at least `min_size` in total size.
	fn claim_size(&self, segments: &mut Segments<N>, min_size: usize) -> Result<(), Self::Error> {
		let count = min_size / N + (min_size % N > 0) as usize;

		self.claim_count(segments, count)
	}

	/// Recycles a single segment back into the pool.
	fn recycle_one(&self, segment: Segment<N>) -> Result<(), Self::Error>;

	/// Recycles many segments back into the pool.
	fn recycle(&self, segments: impl IntoIterator<Item = Segment<N>>) -> Result<(), Self::Error> {
		for seg in segments {
			self.recycle_one(seg)?;
		}

		Ok(())
	}
}

cfg_if! {
	if #[cfg(feature = "shared-pool")] {
		pub type DefaultPool<const N: usize = DEFAULT_SEGMENT_SIZE> = SharedPool<N>;
	} else {
		pub type DefaultPool<const N: usize = DEFAULT_SEGMENT_SIZE> = LocalPool<N>;
	}
}

#[thread_local]
static LOCAL_POOL: Lazy<LocalPool> = Lazy::new(|| LocalPool::default());

#[derive(Clone)]
pub struct LocalPool<const N: usize = DEFAULT_SEGMENT_SIZE> {
	segments: Rc<RefCell<Vec<Segment<N>>>>
}

impl<const N: usize> Pool<N> for LocalPool<N> {
	type Error = Error;

	fn claim_one(&self) -> Result<Segment<N>, Error> {
		Ok(
			self.get_vec()?
				.pop()
				.unwrap_or_default()
		)
	}

	fn claim_count(&self, segments: &mut Segments<N>, count: usize) -> Result<(), Error> {
		let mut vec = self.get_vec()?;
		let len = vec.len();
		let extra = count - len;
		segments.extend_empty(
			vec.drain(0..min(count, len))
			   .chain(
				   repeat_with(Segment::default).take(extra)
			   )
		);
		Ok(())
	}

	fn recycle_one(&self, mut segment: Segment<N>) -> Result<(), Error> {
		segment.clear();
		self.get_vec()?
			.push(segment);
		Ok(())
	}

	fn recycle(&self, segments: impl IntoIterator<Item = Segment<N>>) -> Result<(), Error> {
		struct Cleared<I>(I);
		impl<const N: usize, I: Iterator<Item = Segment<N>>> Iterator for Cleared<I> {
			type Item = I::Item;

			fn next(&mut self) -> Option<Self::Item> {
				let Self(iter) = self;
				let mut seg = iter.next()?;
				seg.clear();
				Some(seg)
			}

			fn size_hint(&self) -> (usize, Option<usize>) { self.0.size_hint() }
		}

		self.get_vec()?
			.extend(Cleared(segments.into_iter()));
		Ok(())
	}
}

impl<const N: usize> LocalPool<N> {
	fn get_vec(&self) -> Result<RefMut<'_, Vec<Segment<N>>>, Error> {
		self.segments
			.try_borrow_mut()
			.map_err(Error::borrow)
	}
}

impl LocalPool {
	pub fn get() -> Self { LOCAL_POOL.clone() }
}

impl Default for LocalPool {
	fn default() -> Self { Self::get() }
}

// Todo: Segment memory can't be sent between threads, won't compile. Split segments
//  into async and sync variants?
#[cfg(feature = "shared-pool")]
mod shared {
	use std::sync::{Arc, Mutex};
	use crate::Error;
	use crate::segment::Segment;

	static SHARED_POOL: SharedPool = SharedPool(Arc::new(Mutex::default()));

	#[derive(Clone)]
	pub struct SharedPool(Arc<Mutex<Vec<Segment>>>);

	impl SharedPool {
		pub fn get() -> Self { SHARED_POOL.clone() }

		fn get_vec(&self) -> Result<&mut Vec<Segment>, Error> {
			let Self(segments) = self;

			Ok(
				&mut segments.lock()
							 .map_err(Error::poison)?
			)
		}
	}
}
