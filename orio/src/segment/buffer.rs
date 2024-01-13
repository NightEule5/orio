// SPDX-License-Identifier: Apache-2.0

use std::borrow::Cow;
use std::cmp::min;
use std::collections::{VecDeque, vec_deque::Iter as DequeIter};
use std::{fmt, slice};
use std::ops::RangeBounds;
use std::rc::Rc;
use super::{BlockDeque, Block};

/// A segment buffer.
#[derive(Clone, Debug, Eq)]
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

#[derive(Clone, Eq)]
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
		let (mut a, mut b) = self.buf()?.as_mut_slices();
		if len < buf_len {
			let off_a = min(off, a.len());
			let len_a = min(len, a.len());
			a = &mut a[off_a..off_a + len_a];
			let off_b = min(off - off_a, b.len());
			let len_b = min(len - len_a, b.len());
			b = &mut b[off_b..off_b + len_b];
		}
		Some((a, b))
	}

	pub fn as_slices_in_range<R: RangeBounds<usize>>(&self, range: R) -> (&[u8], &[u8]) {
		let (mut a, mut b) = self.as_slices();
		let range = slice::range(range, ..self.len);
		let mut len = range.len();

		a = &a[range.start..];
		a = &a[..min(len, a.len())];
		len -= a.len();
		b = &b[..len];

		(a, b)
	}

	pub fn clear(&mut self) {
		self.off = 0;
		self.len = 0;
		let Some(buf) = self.buf() else { return };
		buf.clear();
	}

	pub fn impose(&mut self) {
		let Self { buf, off, len } = self;
		if *off > 0 || *len < buf.len() {
			let Some(buf) = Rc::get_mut(buf) else { return };
			buf.drain(..self.off);
			self.off = 0;
			buf.truncate(self.len);
			self.len = buf.len();
		}
	}

	pub fn buf(&mut self) -> Option<&mut VecDeque<u8>> {
		Rc::get_mut(&mut self.buf)
	}

	pub fn iter(&self) -> DequeIter<u8> {
		self.buf.iter()
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

	pub fn iter(&self) -> impl Iterator<Item = &u8> + '_ {
		use super::block_deque::Iter as BlockIter;
		use slice::Iter as SliceIter;

		enum Iter<'a> {
			Block(BlockIter<'a, u8>),
			Boxed(DequeIter<'a, u8>),
			Slice(SliceIter<'a, u8>)
		}

		impl<'a> Iterator for Iter<'a> {
			type Item = &'a u8;

			fn next(&mut self) -> Option<&'a u8> {
				match self {
					Self::Block(iter) => iter.next(),
					Self::Boxed(iter) => iter.next(),
					Self::Slice(iter) => iter.next()
				}
			}
		}

		match self {
			Self::Block(block) => Iter::Block(block.iter()),
			Self::Boxed(boxed) => Iter::Boxed(boxed.iter()),
			Self::Slice(slice) => Iter::Slice(slice.iter())
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

impl<const N: usize> From<Block<N>> for Buf<'_, N> {
	fn from(value: Block<N>) -> Self {
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

impl<'a, const N: usize> From<Cow<'a, str>> for Buf<'a, N> {
	fn from(value: Cow<'a, str>) -> Self {
		match value {
			Cow::Borrowed(slice) => slice.into(),
			Cow::Owned   (owned) => owned.into()
		}
	}
}

impl<'a, const N: usize> From<Cow<'a, [u8]>> for Buf<'a, N> {
	fn from(value: Cow<'a, [u8]>) -> Self {
		match value {
			Cow::Borrowed(slice) => slice.into(),
			Cow::Owned   (owned) => owned.into()
		}
	}
}

impl<'a, const N: usize> From<&'a [u8]> for Buf<'a, N> {
	fn from(value: &'a [u8]) -> Self {
		Self::Slice(value)
	}
}

impl<'a, const N: usize> From<&'a str> for Buf<'a, N> {
	fn from(value: &'a str) -> Self {
		value.as_bytes().into()
	}
}

impl<'a, const N: usize, T: bytemuck::NoUninit> From<&'a [T]> for Buf<'a , N> {
	default fn from(value: &'a [T]) -> Self {
		Self::Slice(bytemuck::cast_slice(value))
	}
}

#[cfg(feature = "bytes")]
impl<'a, const N: usize> From<&'a bytes::Bytes> for Buf<'a, N> {
	fn from(value: &'a bytes::Bytes) -> Self {
		value.as_ref().into()
	}
}

impl<const N: usize, const O: usize> PartialEq<Buf<'_, O>> for Buf<'_, N> {
	fn eq(&self, other: &Buf<'_, O>) -> bool {
		match (self, other) {
			(Buf::Block(block), &Buf::Slice(other)) => block == other,
			(Buf::Boxed(boxed), &Buf::Slice(other)) => boxed == other,
			(Buf::Slice(slice), Buf::Slice(other)) => slice == other,
			(buf_a, buf_b) if buf_a.len() == buf_b.len() =>
				buf_a.iter().eq(buf_b.iter()),
			_ => false
		}
	}
}

impl<const N: usize> PartialEq<[u8]> for Buf<'_, N> {
	fn eq(&self, other: &[u8]) -> bool {
		match self {
			Buf::Block(block) => block == other,
			Buf::Boxed(boxed) => boxed == other,
			&Buf::Slice(slice) => slice == other,
		}
	}
}

impl PartialEq for BoxedBuf {
	fn eq(&self, other: &Self) -> bool {
		self.len == other.len &&
		self.iter().eq(other.iter())
	}
}

impl PartialEq<[u8]> for BoxedBuf {
	fn eq(&self, other: &[u8]) -> bool {
		let (a, b) = self.as_slices();
		self.len == other.len() &&
		a == &other[..a.len()] &&
		b == &other[a.len()..]
	}
}
