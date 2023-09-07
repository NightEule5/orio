// SPDX-License-Identifier: Apache-2.0

use std::borrow::Cow;
use std::cmp::min;
use std::collections::VecDeque;
use std::fmt;
use std::rc::Rc;
use super::block_deque::BlockDeque;

/// A segment buffer.
#[derive(Clone, Debug)]
pub enum Buf<'d, const N: usize> {
	/// A fixed-size segment buffer. This is used by default.
	Block(BlockDeque<N>),
	/// A variable-size segment buffer. This is used when writing boxed data, such
	/// as a `Vec`, `Box<[u8]>`, `String`, etc. It may be written to, but never
	/// grown.
	Boxed(BoxedBuf),
	/// A read-only, slice segment buffer.
	Slice(&'d [u8]),
}

#[derive(Clone)]
pub struct BoxedBuf {
	pub buf: Rc<VecDeque<u8>>,
	pub off: usize,
	pub len: usize
}

impl BoxedBuf {
	pub fn is_shared(&self) -> bool {
		Rc::strong_count(&self.buf) != 1
	}

	pub fn as_slices(&self) -> (&[u8], &[u8]) {
		let &Self { off, len, .. } = self;
		let mut slices = self.buf.as_slices();
		if len < self.buf.len() {
			let (a, b) = &mut slices;
			let off_a = min(off, a.len());
			let len_a = min(len, a.len());
			*a = &a[off_a..off_a + len_a];
			let off_b = min(off - off_a, b.len());
			let len_b = min(len - len_a, b.len());
			*b = &b[off_b..off_b + len_b];
		}
		slices
	}

	pub fn as_mut_slices(&mut self) -> Option<(&mut [u8], &mut [u8])> {
		let &mut Self { off, len, .. } = self;
		let buf_len = self.buf.len();
		let mut slices = self.buf()?.as_mut_slices();
		if len < buf_len {
			let (a, b) = &mut slices;
			let off_a = min(off, a.len());
			let len_a = min(len, a.len());
			*a = &mut a[off_a..off_a + len_a];
			let off_b = min(off - off_a, b.len());
			let len_b = min(len - len_a, b.len());
			*b = &mut b[off_b..off_b + len_b];
		}
		Some(slices)
	}

	pub fn clear(&mut self) {
		self.off = 0;
		self.len = 0;
		let Some(buf) = self.buf() else { return };
		buf.clear();
	}

	pub fn impose(&mut self) {
		if self.off > 0 || self.len < self.buf.len() {
			let Some(buf) = self.buf() else { return };
			buf.drain(..self.off);
			self.off = 0;
			buf.truncate(self.len);
			self.len = buf.len();
		}
	}

	pub fn buf(&mut self) -> Option<&mut VecDeque<u8>> {
		Rc::get_mut(&mut self.buf)
	}
}

impl fmt::Debug for BoxedBuf {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_list()
		 .entries(
			 self.buf
				 .iter()
				 .skip(self.off)
				 .take(self.len)
		 ).finish()
	}
}

impl<const N: usize> Buf<'_, N> {
	pub fn len(&self) -> usize {
		match self {
			Buf::Block(block) => block.len(),
			Buf::Boxed(boxed) => boxed.len,
			Buf::Slice(slice) => slice.len(),
		}
	}

	pub fn limit(&self) -> usize {
		match self {
			Buf::Block(block) if !block.is_shared() => block.limit(),
			Buf::Boxed(boxed) if !boxed.is_shared() => boxed.buf.capacity() - boxed.len,
			_ => 0
		}
	}
}

impl<const N: usize> Default for Buf<'_, N> {
	fn default() -> Self {
		BlockDeque::default().into()
	}
}

impl<const N: usize> From<BlockDeque<N>> for Buf<'_, N> {
	fn from(value: BlockDeque<N>) -> Self {
		Self::Block(value)
	}
}

impl<const N: usize> From<Box<[u8; N]>> for Buf<'_, N> {
	fn from(value: Box<[u8; N]>) -> Self {
		BlockDeque::from(value).into()
	}
}

impl<const N: usize> From<Rc<VecDeque<u8>>> for Buf<'_, N> {
	fn from(buf: Rc<VecDeque<u8>>) -> Self {
		Self::Boxed(BoxedBuf { buf, off: 0, len: 0 })
	}
}

impl<const N: usize> From<VecDeque<u8>> for Buf<'_, N> {
	fn from(value: VecDeque<u8>) -> Self {
		Rc::new(value).into()
	}
}

impl<const N: usize> From<Vec<u8>> for Buf<'_, N> {
	fn from(value: Vec<u8>) -> Self {
		VecDeque::from(value).into()
	}
}

impl<'d, const N: usize> From<String> for Buf<'_, N> {
	fn from(value: String) -> Self {
		value.into_bytes().into()
	}
}

impl<const N: usize> From<Cow<'_, str>> for Buf<'_, N> {
	fn from(value: Cow<'_, str>) -> Self {
		match value {
			Cow::Borrowed(slice) => slice.into(),
			Cow::Owned   (owned) => owned.into()
		}
	}
}

impl<const N: usize> From<Cow<'_, [u8]>> for Buf<'_, N> {
	fn from(value: Cow<'_, [u8]>) -> Self {
		match value {
			Cow::Borrowed(slice) => slice.into(),
			Cow::Owned   (owned) => owned.into()
		}
	}
}

impl<const N: usize> From<&[u8]> for Buf<'_, N> {
	fn from(value: &[u8]) -> Self {
		Self::Slice(value)
	}
}

impl<const N: usize> From<&str> for Buf<'_, N> {
	fn from(value: &str) -> Self {
		value.as_bytes().into()
	}
}

impl<const N: usize, T: bytemuck::NoUninit> From<&[T]> for Buf<'_, N> {
	default fn from(value: &[T]) -> Self {
		Self::Slice(bytemuck::cast_slice(value))
	}
}

#[cfg(feature = "bytes")]
impl<const N: usize> From<&bytes::Bytes> for Buf<'_, N> {
	fn from(value: &bytes::Bytes) -> Self {
		value.as_ref().into()
	}
}
