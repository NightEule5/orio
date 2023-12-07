// SPDX-License-Identifier: Apache-2.0

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
//! can have gaps where some segments are not filled or partially read, called *fragmentation*.
//! Compacting these on every write could be costly, but keeping them is less space
//! efficient which would lead to more allocations. As fragmentation size reaches a
//! threshold, 4096B by default, all segments are compacted. This can also be triggered
//! manually with the `compact` function.
//!
//! Segments can be allocated when: 1) a buffer requests one but the pool has none
//! left, or 2) a shared segment is created then written to.

#![allow(incomplete_features)]
#![feature(
	arbitrary_self_types,
	associated_type_bounds,
	associated_type_defaults,
	extend_one,
	extract_if,
	generic_const_exprs,
	int_roundings,
	new_uninit,
	pattern,
	seek_stream_len,
	slice_range,
	specialization,
	str_internals,
	thread_local,
	try_blocks,
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

pub use error::*;
pub use buffer::*;
pub use segment::*;
