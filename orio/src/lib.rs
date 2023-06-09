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

//! ## How it works
//!
//! Data is written to and read from reusable bits of memory called *segments*.
//! When a segment is consumed, it's returned to a *pool*. To write data, segments
//! are claimed from this pool. When the pool is exhausted, segments are created up
//! to a set limit. The default pool instance has two modes: with the `shared-pool`
//! feature its segment container is wrapped in an `Arc<Mutex<...>>`, otherwise
//! `RefCell` is used. The latter is faster but must be thread-local, meaning each
//! thread has its own pool.
//!
//! ### Segments
//!
//! Segments are reusable chunks of memory arranged in a ring buffer. Memory within
//! segments can either be owned by or shared between segments, avoiding expensive
//! mem-copy operations as much as possible. Shared memory is copy-on-write; it can
//! be read by multiple segments, only copying when written. Small amounts of data
//! under a set threshold (1024B by default) are not shared, as a tradeoff between
//! memory allocation performance and speed.
//!
//! The ring buffer behaves as a continuous byte deque. Bytes are read from one end
//! and written to the other, claiming new segments from the pool as it fills. Data
//! can have gaps where some segments are not filled or partially read, called *voids*.
//! Compacting these on every write could be costly, but keeping them is less space
//! efficient which would lead to more allocations. As void size reaches a threshold,
//! 4096B by default, all segments are compacted. This can also be triggered manually
//! with the `compact` function.
//!
//! As the segments are consumed, empty segments may be returned to the pool. Some
//! ratio of empty segments to full segments, the *retention ratio*, are kept for
//! further writes; the rest are recycled. The default ratio is 1, meaning for one
//! byte in the buffer, at least one byte of free space is kept for future writes.
//! This ratio allows buffers to keep segments they will likely need, but not keep
//! too many segments.
//!
//! Segments can be allocated when: 1) a buffer requests one but the pool has none
//! left, or 2) a shared segment is created then written to.

#![allow(incomplete_features)]
#![feature(
associated_type_bounds,
	associated_type_defaults,
	drain_filter,
	extend_one,
	generic_const_exprs,
	return_position_impl_trait_in_trait,
	slice_range,
	specialization,
	thread_local,
	type_alias_impl_trait,
)]

mod buffer;
mod buffered_wrappers;
mod error;
pub mod streams;
mod segment;
mod element;
pub mod pool;
mod util;
mod byte_str;

pub use error::*;
pub use buffer::*;
pub use segment::{Segment, SIZE as SEGMENT_SIZE};
pub use byte_str::*;
