// SPDX-License-Identifier: Apache-2.0

use crate::SIZE;

/// Options for tuning [`Buffer`](super::Buffer)'s behavior and performance.
///
/// # Share threshold
///
/// The minimum size for segment data to be shared rather than writing it to another
/// segment. Defaults to `1024B`, one eighth the default segment size. With a value
/// more than the segment size, segments are never shared.
///
/// Sharing is significantly faster than copying for large segments, O(1) vs O(n)
/// complexity. The tradeoffs may not be worth it for small segments, however. When
/// the deque is full and needs to resize, an O(n) cost is incurred to move segments.
/// Reading may be slower if the buffer contains many small segments.
///
/// # Borrow threshold
///
/// The minimum length for a slice to be borrowed rather than writing it to another
/// segment. Defaults to `1024B`, one eighth the default segment size. With a value
/// more than the segment size, slices are never borrowed. Currently this only
/// affects [`Buffer::push_slice`] and similar methods, where the lifetime is known
/// at compile-time to outlive the buffer, and operations between buffers holding
/// slices.
///
/// As with sharing, borrowing data is significantly faster than copying it, but
/// the cost of storing many small segments may outweigh this speedup.
///
/// # Allocation
///
/// By default, the buffer will fallback to allocating memory if borrowing the pool
/// fails. It can also be set to always allocate, ignoring the pool, or to never
/// allocate.
///
/// [`Buffer::push_slice`]: super::Buffer::push_slice
#[derive(Copy, Clone, Debug)]
#[non_exhaustive]
pub struct BufferOptions {
	pub share_threshold: usize,
	pub borrow_threshold: usize,
	pub allocation: Allocate,
}

/// The segment allocation mode.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum Allocate {
	/// Claim segments from the pool instead of allocating, don't allocate if the
	/// pool cannot be borrowed.
	Never,
	/// Always allocate segments, ignoring the pool.
	Always,
	/// Claim segments from the pool, allocating if the pool cannot be borrowed.
	#[default]
	OnError,
}

impl Allocate {
	/// Returns `true` if the mode is [`Never`](Self::Never).
	pub fn is_never(&self) -> bool {
		matches!(self, Self::Never)
	}

	/// Returns `true` if the mode is [`Always`](Self::Always).
	pub fn is_always(&self) -> bool {
		matches!(self, Self::Always)
	}

	/// Returns `true` if the mode is [`OnError`](Self::OnError).
	pub fn is_on_error(&self) -> bool {
		matches!(self, Self::OnError)
	}
}

impl Default for BufferOptions {
	fn default() -> Self { Self::new() }
}

impl BufferOptions {
	/// Creates a new set of buffer options.
	pub const fn new() -> Self {
		Self {
			share_threshold: SIZE / 8,
			borrow_threshold: SIZE / 8,
			allocation: Allocate::OnError,
		}
	}

	/// Presets the options to create a "lean" buffer, disabling data sharing and
	/// borrowing. The buffer will always copies shared or borrowed data to owned
	/// segments.
	#[inline]
	pub const fn lean() -> Self {
		Self {
			share_threshold: usize::MAX,
			borrow_threshold: usize::MAX,
			..Self::new()
		}
	}

	/// Returns the segment share threshold.
	#[inline]
	pub const fn share_threshold(&self) -> usize { self.share_threshold }

	/// Returns the segment borrow threshold.
	#[inline]
	pub const fn borrow_threshold(&self) -> usize { self.borrow_threshold }

	/// Returns the segment allocation mode.
	#[inline]
	pub const fn allocation(&self) -> Allocate { self.allocation }

	/// Sets the segment share threshold.
	#[inline]
	pub fn set_share_threshold(&mut self, value: usize) {
		self.share_threshold = value;
	}

	/// Sets the segment borrow threshold.
	#[inline]
	pub fn set_borrow_threshold(&mut self, value: usize) {
		self.borrow_threshold = value;
	}

	/// Sets the segment allocation mode.
	#[inline]
	pub fn set_allocation(&mut self, value: Allocate) {
		self.allocation = value;
	}

	/// Sets segment allocation to [`Always`](Allocate::Always).
	#[inline]
	pub fn set_always_allocate(&mut self) {
		self.set_allocation(Allocate::Always)
	}

	/// Sets segment allocation to [`Never`](Allocate::Never).
	#[inline]
	pub fn set_never_allocate(&mut self) {
		self.set_allocation(Allocate::Never)
	}

	/// Sets segment allocation to [`OnError`](Allocate::OnError).
	#[inline]
	pub fn set_allocate_on_error(&mut self) {
		self.set_allocation(Allocate::OnError)
	}

	/// Sets the segment share threshold.
	#[inline]
	pub const fn with_share_threshold(mut self, value: usize) -> Self {
		self.share_threshold = value;
		self
	}

	/// Sets the segment borrow threshold.
	#[inline]
	pub const fn with_borrow_threshold(mut self, value: usize) -> Self {
		self.borrow_threshold = value;
		self
	}

	/// Sets the segment allocation mode.
	#[inline]
	pub const fn with_allocation(mut self, value: Allocate) -> Self {
		self.allocation = value;
		self
	}

	/// Sets segment allocation to [`Always`](Allocate::Always).
	#[inline]
	pub const fn always_allocate(self) -> Self {
		self.with_allocation(Allocate::Always)
	}

	/// Sets segment allocation to [`Never`](Allocate::Never).
	#[inline]
	pub const fn never_allocate(self) -> Self {
		self.with_allocation(Allocate::Never)
	}

	/// Sets segment allocation to [`OnError`](Allocate::OnError).
	#[inline]
	pub const fn allocate_on_error(self) -> Self {
		self.with_allocation(Allocate::OnError)
	}
}
