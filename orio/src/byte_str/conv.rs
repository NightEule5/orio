// SPDX-License-Identifier: Apache-2.0

use std::borrow::{Borrow, Cow};
use super::{ByteStr, ByteString};
use crate::{Seg, segment::RBuf};

impl<'a> From<Vec<&'a [u8]>> for ByteStr<'a> {
	fn from(data: Vec<&'a [u8]>) -> Self {
		let len = data.iter().copied().map(<[u8]>::len).sum();
		Self {
			utf8: None,
			data,
			len,
		}
	}
}

impl<'a> From<&'a [u8]> for ByteStr<'a> {
	fn from(value: &'a [u8]) -> Self {
		Self::from(vec![value])
	}
}

impl<'a, const N: usize> From<&'a RBuf<Seg<'a, N>>> for ByteStr<'a> {
	fn from(value: &'a RBuf<Seg<'a, N>>) -> Self {
		Self {
			utf8: None,
			data: value.iter_slices().collect(),
			len: value.count(),
		}
	}
}

impl<'a> FromIterator<&'a [u8]> for ByteStr<'a> {
	fn from_iter<T: IntoIterator<Item = &'a [u8]>>(iter: T) -> Self {
		let data: Vec<_> = iter.into_iter().collect();
		data.into()
	}
}

impl<'a> AsRef<[&'a [u8]]> for ByteStr<'a> {
	fn as_ref(&self) -> &[&'a [u8]] {
		&self.data
	}
}

impl<'a> Borrow<[&'a [u8]]> for ByteStr<'a> {
	fn borrow(&self) -> &[&'a [u8]] {
		&self.data
	}
}


impl From<String> for ByteString {
	fn from(data: String) -> Self {
		Self {
			is_utf8: true.into(),
			data: data.into_bytes()
		}
	}
}

impl From<Vec<u8>> for ByteString {
	fn from(data: Vec<u8>) -> Self {
		Self {
			is_utf8: false.into(),
			data
		}
	}
}

impl From<&str> for ByteString {
	fn from(value: &str) -> Self {
		value.to_owned().into()
	}
}

impl From<&[u8]> for ByteString {
	fn from(value: &[u8]) -> Self {
		value.to_owned().into()
	}
}

impl<'a> From<Cow<'a, str>> for ByteString {
	fn from(value: Cow<'a, str>) -> Self {
		value.to_owned().into()
	}
}

impl<'a> From<Cow<'a, [u8]>> for ByteString {
	fn from(value: Cow<'a, [u8]>) -> Self {
		value.to_owned().into()
	}
}

impl<'a> FromIterator<&'a u8> for ByteString {
	fn from_iter<T: IntoIterator<Item = &'a u8>>(iter: T) -> Self {
		iter.into_iter()
			.copied()
			.collect()
	}
}

impl FromIterator<u8> for ByteString {
	fn from_iter<T: IntoIterator<Item = u8>>(iter: T) -> Self {
		iter.into_iter()
			.collect::<Vec<_>>()
			.into()
	}
}

impl<'a> FromIterator<&'a [u8]> for ByteString {
	fn from_iter<T: IntoIterator<Item = &'a [u8]>>(iter: T) -> Self {
		iter.into_iter()
			.flatten()
			.copied()
			.collect()
	}
}

impl Borrow<[u8]> for ByteString {
	fn borrow(&self) -> &[u8] {
		&self.data
	}
}

impl AsRef<[u8]> for ByteString {
	fn as_ref(&self) -> &[u8] { &self.data }
}
