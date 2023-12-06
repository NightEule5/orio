// SPDX-License-Identifier: Apache-2.0

use crate::SIZE;

/// Options for tuning [`Buffer`](super::Buffer)'s behavior and performance.
///
/// # Share threshold
///
/// The minimum size for segment data to be shared rather than writing it to another
/// segment. Defaults to `1024B`, one eighth the default segment size. With a value
/// is more than the segment size, segments are never shared.
///
/// Sharing is significantly faster than copying for large segments, O(1) vs O(n)
/// complexity. The tradeoffs may not be worth it for small segments, however. When
/// the deque is full and needs to resize, an O(n) cost is incurred to move segments.
/// Reading may be slower if the buffer contains many small segments. As memory
/// fragments with small shared segments (see [Compact threshold](#compact-threshold)
/// section), the buffer compacts.
///
/// # Compact threshold
///
/// The total size of fragmentation (gaps where segments have been partially read
/// or written) that triggers compacting. Defaults to `4096B`, half the segment
/// size. With a value of `0`, the buffer always compacts.
#[derive(Copy, Clone, Debug)]
#[non_exhaustive]
pub struct BufferOptions {
	pub share_threshold: usize,
	pub compact_threshold: usize,
}

impl Default for BufferOptions {
	fn default() -> Self {
		Self {
			share_threshold: SIZE / 8,
			compact_threshold: SIZE / 2,
		}
	}
}

impl BufferOptions {
	/// Presets the options to create a "lean" buffer, a buffer that always shares
	/// and compacts.
	pub fn lean() -> Self {
		Self {
			share_threshold: 0,
			compact_threshold: 0,
		}
	}

	/// Returns the segment share threshold.
	pub fn share_threshold(&self) -> usize { self.share_threshold }
	/// Returns the fragmentation-compact threshold.
	pub fn compact_threshold(&self) -> usize { self.compact_threshold }

	/// Sets the segment share threshold.
	pub fn set_share_threshold(mut self, value: usize) -> Self {
		self.share_threshold = value;
		self
	}

	/// Sets the fragmentation-compact threshold.
	pub fn set_compact_threshold(mut self, value: usize) -> Self {
		self.compact_threshold = value;
		self
	}
}
