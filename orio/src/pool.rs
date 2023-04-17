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
use std::{fmt, result};
use std::fmt::Formatter;
use std::iter::repeat_with;
use std::rc::Rc;
use amplify_derive::Display;
use cfg_if::cfg_if;
use once_cell::unsync::Lazy;
use crate::{DEFAULT_SEGMENT_SIZE, error, ErrorBox, Segment, Segments};
use ErrorKind::Other;
use crate::pool::ErrorKind::Borrowed;
use crate::pool::OperationKind::{Claim, Recycle};

pub type Error = error::Error<OperationKind, ErrorKind>;
pub type Result<T = (), E = Error> = result::Result<T, E>;

#[derive(Copy, Clone, Debug, Display)]
pub enum OperationKind {
	#[display("unknown")]
	Unknown,
	#[display("claim")]
	Claim,
	#[display("recycle")]
	Recycle,
}

impl error::OperationKind for OperationKind {
	fn unknown() -> Self { Self::Unknown }
}

#[derive(Copy, Clone, Debug, Display)]
pub enum ErrorKind {
	#[cfg(feature = "shared-pool")]
	#[display("mutex poisoned")]
	MutexPoison,
	#[display("the pool is already being borrowed")]
	Borrowed,
	#[display("{0}")]
	Other(&'static str)
}

impl error::ErrorKind for ErrorKind {
	fn other(message: &'static str) -> Self { Other(message) }
}

impl Error {
	/// Creates a new "borrowed" error.
	pub fn borrowed(op: OperationKind, source: ErrorBox) -> Self {
		Self::new(op, Borrowed, Some(source))
	}

	fn claim_borrow(source: ErrorBox) -> Self {
		Self::borrowed(Claim, source)
	}

	fn recycle_borrow(source: ErrorBox) -> Self {
		Self::borrowed(Recycle, source)
	}
}

pub trait Pool {
	/// Claims a single segment.
	fn claim_one(&self) -> Result<Segment>;

	/// Claims `count` segments into the container.
	fn claim_count(&self, segments: &mut Segments, count: usize) -> Result {
		for _ in 0..count {
			segments.push(self.claim_one()?)
		}

		Ok(())
	}

	/// Claims many segments into the container, at least `min_size` in total size.
	fn claim_size(&self, segments: &mut Segments, min_size: usize) -> Result {
		const N: usize = DEFAULT_SEGMENT_SIZE;
		let count = min_size / N + (min_size % N > 0) as usize;

		self.claim_count(segments, count)
	}

	/// Recycles a single segment back into the pool.
	fn recycle_one(&self, segment: Segment) -> Result;

	/// Recycles many segments back into the pool.
	fn recycle(&self, segments: impl IntoIterator<Item = Segment>) -> Result {
		for seg in segments {
			self.recycle_one(seg)?;
		}

		Ok(())
	}
}

cfg_if! {
	if #[cfg(feature = "shared-pool")] {
		pub type DefaultPool = SharedPool;
	} else {
		pub type DefaultPool = LocalPool;
	}
}

#[thread_local]
static LOCAL_POOL: Lazy<LocalPool> = Lazy::new(|| LocalPool::default());

#[derive(Clone)]
pub struct LocalPool {
	segments: Rc<RefCell<Vec<Segment>>>
}

impl Pool for LocalPool {
	fn claim_one(&self) -> Result<Segment> {
		Ok(
			self.get_vec()
				.map_err(Error::claim_borrow)
				.pop()
				.unwrap_or_default()
		)
	}

	fn claim_count(&self, segments: &mut Segments, count: usize) -> Result {
		let mut vec = self.get_vec().map_err(Error::claim_borrow);
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

	fn recycle_one(&self, mut segment: Segment) -> Result {
		segment.clear();
		self.get_vec()
			.map_err(Error::recycle_borrow)
			.push(segment);
		Ok(())
	}

	fn recycle(&self, segments: impl IntoIterator<Item = Segment>) -> Result {
		struct Cleared<I>(I);
		impl<I: Iterator<Item = Segment>> Iterator for Cleared<I> {
			type Item = I::Item;

			fn next(&mut self) -> Option<Self::Item> {
				let Self(iter) = self;
				let mut seg = iter.next()?;
				seg.clear();
				Some(seg)
			}

			fn size_hint(&self) -> (usize, Option<usize>) { self.0.size_hint() }
		}

		self.get_vec()
			.map_err(Error::recycle_borrow)?
			.extend(Cleared(segments.into_iter()));
		Ok(())
	}
}

impl LocalPool {
	fn get_vec(&self) -> Result<RefMut<'_, Vec<Segment>>, ErrorBox> {
		Ok(self.segments.try_borrow_mut()?)
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
