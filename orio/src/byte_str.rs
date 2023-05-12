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

use std::cmp::min;
use std::ops::{Add, RangeBounds};
use std::slice;
use base64::Engine;
use base64::engine::GeneralPurpose;
use base64::prelude::{BASE64_STANDARD_NO_PAD, BASE64_URL_SAFE_NO_PAD};
use simdutf8::compat::{from_utf8, Utf8Error};
use crate::Segment;
use crate::segment::SegmentRing;
use crate::streams::OffsetUtf8Error;

/// A borrowed, segmented string of bytes.
#[derive(Clone, Debug, Default, Eq)]
pub struct ByteStr<'b> {
	utf8: Option<String>,
	data: Vec<&'b [u8]>,
	len: usize,
}

impl<'t> ByteStr<'t> {
	/// Creates a byte string from `str`.
	pub fn from_utf8(str: &'t str) -> Self {
		Self {
			utf8: None,
			data: vec![str.as_bytes()],
			len: str.len(),
		}
	}
}

impl ByteStr<'_> {
	/// Creates an empty byte string.
	pub fn empty() -> Self { Self::default() }

	/// Returns the length in bytes of the byte string.
	pub fn len(&self) -> usize { self.len }

	/// Returns the byte at `index`, or `None` if `index` is out of bounds.
	pub fn get(&mut self, mut index: usize) -> Option<u8> {
		for chunk in self.data.iter() {
			if index < chunk.len() {
				return Some(chunk[index])
			}

			index -= chunk.len();
		}

		None
	}

	/// Decodes and caches the bytes as UTF-8, returning a borrow of the cache.
	/// Subsequent calls to this function will borrow from the cache, and calls to
	/// [`utf8`][] will clone from it.
	///
	/// [`utf8`]: Self::utf8
	pub fn utf8_cache(&mut self) -> Result<&str, OffsetUtf8Error> {
		let Some(ref utf8) = self.utf8 else {
			return Ok(self.utf8.insert(self.utf8()?))
		};

		Ok(utf8)
	}

	/// Returns a string of UTF-8-decoded bytes, cloning from the value cached by
	/// [`utf8_cache`][] if any.
	///
	/// [`utf8_cache`]: Self::utf8_cache
	pub fn utf8(&self) -> Result<String, OffsetUtf8Error> {
		let Some(ref utf8) = self.utf8 else {
			let mut str = String::default();
			for data in self.data.iter() {
				str.push_str(
					from_utf8(data).map_err(|err|
						OffsetUtf8Error::new(err, str.len())
					)?
				)
			}

			return Ok(str)
		};

		Ok(utf8.clone())
	}

	/// Encodes data into a Base64 string.
	pub fn base64(&self) -> String { self.encode(BASE64_STANDARD_NO_PAD) }

	/// Encodes data into a Base64 URL string.
	pub fn base64_url(&self) -> String { self.encode(BASE64_URL_SAFE_NO_PAD) }

	/// Encodes data into a lowercase hex string.
	pub fn hex_lower(&self) -> String { self.encode(Base16Encoder::<false>) }

	/// Encodes data into an uppercase hex string.
	pub fn hex_upper(&self) -> String { self.encode(Base16Encoder::<true>) }

	fn encode<'b, E: Encoder + Into<RollingEncoder<'b, E>>>(&'b self, encoder: E) -> String {
		let Self { data, .. } = self;
		let mut dst = String::default();

		if data.len() == 1 {
			encoder.encode_data(data[0], &mut dst);
			dst
		} else {
			let mut enc = encoder.into();
			for data in data {
				enc.encode(data, &mut dst);
			}

			enc.finish(&mut dst);
			dst
		}
	}

	/// Clones the borrowed data into an owned [`ByteString`].
	pub fn to_byte_string(&self) -> ByteString {
		self.data.iter().fold(
			Vec::with_capacity(self.len),
			|mut acc, cur| {
				acc.extend_from_slice(cur);
				acc
			}
		).into()
	}

	pub(crate) fn iter(&self) -> impl Iterator<Item = &[u8]> + '_ {
		self.data
			.iter()
			.cloned()
	}
}

impl PartialEq for ByteStr<'_> {
	fn eq(&self, other: &Self) -> bool {
		if self.len != other.len {
			return false
		}

		for (a, b) in self.iter().flatten().zip(other.iter().flatten()) {
			if a != b {
				return false
			}
		}

		true
	}
}

impl PartialEq<[u8]> for ByteStr<'_> {
	fn eq(&self, other: &[u8]) -> bool {
		if self.len != other.len() {
			return false
		}

		for (a, b) in self.iter().flatten().zip(other) {
			if a != b {
				return false
			}
		}

		true
	}
}

impl PartialEq<ByteString> for ByteStr<'_> {
	fn eq(&self, other: &ByteString) -> bool {
		self == other.data.as_slice()
	}
}

impl<'b> From<Vec<&'b [u8]>> for ByteStr<'b> {
	fn from(data: Vec<&'b [u8]>) -> Self {
		let len = data.iter().map(|v| v.len()).sum();
		Self {
			utf8: None,
			data,
			len,
		}
	}
}

impl<'b> From<&'b [u8]> for ByteStr<'b> {
	fn from(value: &'b [u8]) -> Self {
		Self::from(vec![value])
	}
}

impl<'b> From<&'b SegmentRing> for ByteStr<'b> {
	fn from(value: &'b SegmentRing) -> Self {
		Self {
			utf8: None,
			data: value.iter().map(Segment::data).collect(),
			len: value.count(),
		}
	}
}

impl Add for ByteStr<'_> {
	type Output = Self;

	fn add(mut self, Self { ref utf8, ref data, len }: Self) -> Self {
		self.utf8 = if let Some(utf8_a) = self.utf8 {
			if let Some(utf8_b) = utf8 {
				Some(utf8_a.add(utf8_b))
			} else {
				None
			}
		} else {
			None
		};

		self.data.extend_from_slice(data);
		self.len += len;
		self
	}
}

/// An owned string of bytes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ByteString {
	data: Vec<u8>
}

impl ByteString {
	/// Creates an empty byte string.
	pub fn empty() -> Self { Self::default() }

	pub(crate) fn with_capacity(capacity: usize) -> Self {
		Self { data: Vec::with_capacity(capacity) }
	}

	/// Returns the length in bytes of the byte string.
	pub fn len(&self) -> usize { self.data.len() }

	/// Returns the byte at `index`, or `None` if `index` is out of bounds.
	pub fn get(&mut self, index: usize) -> Option<u8> {
		self.data.get(index).cloned()
	}

	/// Returns a string of UTF-8-decoded bytes.
	pub fn utf8(&self) -> Result<&str, Utf8Error> {
		from_utf8(&self.data)
	}

	/// Encodes data into a Base64 string.
	pub fn base64(&self) -> String { self.encode(BASE64_STANDARD_NO_PAD) }

	/// Encodes data into a Base64 URL string.
	pub fn base64_url(&self) -> String { self.encode(BASE64_URL_SAFE_NO_PAD) }

	/// Encodes data into a lowercase hex string.
	pub fn hex_lower(&self) -> String { self.encode(Base16Encoder::<false>) }

	/// Encodes data into an uppercase hex string.
	pub fn hex_upper(&self) -> String { self.encode(Base16Encoder::<true>) }

	fn encode<E: Encoder>(&self, encoder: E) -> String {
		let mut dst = String::default();
		encoder.encode_data(&self.data, &mut dst);
		dst
	}

	pub(crate) fn extend_from_slice(&mut self, slice: &[u8]) {
		self.data.extend_from_slice(slice);
	}

	/// Borrows the data into a [`ByteStr`].
	pub fn as_byte_str(&self) -> ByteStr<'_> {
		ByteStr {
			utf8: None,
			data: vec![&self.data],
			len: self.len(),
		}
	}

	/// Returns the internal data as a slice of bytes.
	pub fn as_slice(&self) -> &[u8] { self.data.as_slice() }

	/// Returns a slice of the byte string bounded by `range`.
	pub fn substr<R: RangeBounds<usize>>(&self, range: R) -> ByteStr<'_> {
		let range = slice::range(range, ..self.len());
		let slice = &self.as_slice()[range];
		ByteStr {
			utf8: None,
			data: vec![slice],
			len: slice.len(),
		}
	}
}

#[cfg(feature = "hash")]
impl ByteStr<'_> {
	pub fn hash(&self, mut digest: impl digest::Digest) -> ByteString {
		for data in &*self.data {
			digest.update(data)
		}
		digest.finalize().as_slice().into()
	}
}

#[cfg(feature = "hash")]
impl ByteString {
	pub fn hash(&self, mut digest: impl digest::Digest) -> Self {
		digest.update(&self.data);
		digest.finalize().as_slice().into()
	}
}

macro_rules! hash_fn {
    (secure $name:literal$fn:ident$module:ident$hasher:ident) => {
		/// Computes a
		#[doc = $name]
		/// hash of the byte string. There are no known attacks for this hash
		/// function; it can be considered suitable for cryptography.
		pub fn $fn(&self) -> ByteString {
			self.hash($module::$hasher::default())
		}
	};
    (broken $name:literal$fn:ident$module:ident$hasher:ident) => {
		/// Computes a
		#[doc = $name]
		/// hash of the byte string. This hash function has been broken; its use in
		/// cryptography is ***not*** secure. Use for checksums only.
		pub fn $fn(&self) -> ByteString {
			self.hash($module::$hasher::default())
		}
	};
}

macro_rules! hash {
    ($sec:tt$feature:literal$module:ident
	$($size_name:literal$size_fn:ident$size_hasher:ident)+
	) => {
		#[cfg(feature = $feature)]
		impl ByteString {
			$(hash_fn! { $sec$size_name$size_fn$module$size_hasher })+
		}
		#[cfg(feature = $feature)]
		impl ByteStr<'_> {
			$(hash_fn! { $sec$size_name$size_fn$module$size_hasher })+
		}
	};
}

hash! {
	secure "groestl" groestl
	"Grøstl-224" groestl224 Groestl224
	"Grøstl-256" groestl256 Groestl256
	"Grøstl-384" groestl384 Groestl384
	"Grøstl-512" groestl512 Groestl512
}

hash! {
	broken "md5" md5
	"MD5" md5 Md5
}

hash! {
	broken "sha1" sha1
	"SHA1" sha1 Sha1
}

hash! {
	secure "sha2" sha2
	"SHA-224" sha224 Sha224
	"SHA-256" sha256 Sha256
	"SHA-384" sha384 Sha384
	"SHA-512" sha512 Sha512
}

hash! {
	secure "sha3" sha3
	"SHA3-224 (Keccak)" sha3_224 Sha3_224
	"SHA3-256 (Keccak)" sha3_256 Sha3_256
	"SHA3-384 (Keccak)" sha3_384 Sha3_384
	"SHA3-512 (Keccak)" sha3_512 Sha3_512
}

hash! {
	secure "shabal" shabal
	"Shabal-192" shabal192 Shabal192
	"Shabal-224" shabal224 Shabal224
	"Shabal-256" shabal256 Shabal256
	"Shabal-384" shabal384 Shabal384
	"Shabal-512" shabal512 Shabal512
}

hash! {
	secure "whirlpool" whirlpool
	"Whirlpool" whirlpool Whirlpool
}

impl<'b> PartialEq<ByteStr<'b>> for ByteString {
	fn eq(&self, other: &ByteStr<'b>) -> bool {
		other == self.as_slice()
	}
}

impl<'b> From<&'b ByteString> for ByteStr<'b> {
	fn from(value: &'b ByteString) -> Self {
		Self::from(value.as_slice())
	}
}

impl<'b> From<&ByteStr<'b>> for ByteString {
	fn from(value: &ByteStr<'b>) -> Self {
		value.to_byte_string()
	}
}

impl From<Vec<u8>> for ByteString {
	fn from(data: Vec<u8>) -> Self {
		Self { data }
	}
}

impl From<&[u8]> for ByteString {
	fn from(value: &[u8]) -> Self {
		value.to_vec().into()
	}
}

impl FromIterator<u8> for ByteString {
	fn from_iter<T: IntoIterator<Item = u8>>(iter: T) -> Self {
		iter.into_iter()
			.collect::<Vec<_>>()
			.into()
	}
}

impl AsRef<[u8]> for ByteString {
	fn as_ref(&self) -> &[u8] { &self.data }
}

impl Extend<u8> for ByteString {
	fn extend<T: IntoIterator<Item = u8>>(&mut self, iter: T) {
		self.data.extend(iter)
	}
}

trait Encoder {
	fn encode_data(&self, input: &[u8], dst: &mut String);
}

impl Encoder for GeneralPurpose {
	fn encode_data(&self, data: &[u8], dst: &mut String) {
		self.encode_string(data, dst);
	}
}

struct Base16Encoder<const UPPERCASE: bool>;

impl Encoder for Base16Encoder<true> {
	fn encode_data(&self, input: &[u8], dst: &mut String) {
		dst.push_str(&base16ct::upper::encode_string(input))
	}
}

impl Encoder for Base16Encoder<false> {
	fn encode_data(&self, input: &[u8], dst: &mut String) {
		dst.push_str(&base16ct::lower::encode_string(input))
	}
}

/// Encodes slices as a multiple of `width`, rolling over the remainder into the
/// next slice. This ensures segmented data will be encoded the same as equivalent
/// contiguous data would be.
struct RollingEncoder<'b, E: Encoder> {
	width: usize,
	rem: Option<&'b [u8]>,
	enc: E,
}

impl<'b, E: Encoder> RollingEncoder<'b, E> {
	fn encode(&mut self, mut data: &'b [u8], dst: &mut String) {
		let width = self.width;

		if let Some(value) = self.rem.take() {
			let mut rem = Vec::with_capacity(width);
			rem.extend_from_slice(value);

			let len = min(width - value.len(), data.len());
			rem.extend_from_slice(&data[..len]);
			self.enc.encode_data(&rem, dst);
			data = &data[len..];
		}

		let clean_len = data.len() / width * width;
		if clean_len < data.len() {
			let _ = self.rem.insert(&data[clean_len..]);
		}

		self.enc.encode_data(&data[..clean_len], dst);
	}

	fn finish(self, dst: &mut String) {
		let Some(rem) = self.rem else { return };
		self.enc.encode_data(rem, dst)
	}
}

impl From<GeneralPurpose> for RollingEncoder<'_, GeneralPurpose> {
	fn from(value: GeneralPurpose) -> Self {
		Self {
			width: 3,
			rem: None,
			enc: value,
		}
	}
}

impl From<Base16Encoder<true>> for RollingEncoder<'_, Base16Encoder<true>> {
	fn from(value: Base16Encoder<true>) -> Self {
		Self {
			width: 2,
			rem: None,
			enc: value,
		}
	}
}

impl From<Base16Encoder<false>> for RollingEncoder<'_, Base16Encoder<false>> {
	fn from(value: Base16Encoder<false>) -> Self {
		Self {
			width: 2,
			rem: None,
			enc: value,
		}
	}
}

#[cfg(test)]
mod test {
	use base16ct::{lower, upper};
	use base64::Engine;
	use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
	use quickcheck::TestResult;
	use quickcheck_macros::quickcheck;
	use crate::{ByteStr, ByteString};

	#[quickcheck]
	fn same_size_eq(data: Vec<u8>) {
		let a1 = ByteStr::from(&*data);
		let b1 = ByteStr::from(&*data);
		assert_eq!(a1, b1, "ByteStr == ByteStr");
		let a2 = ByteString::from(data.clone());
		let b2 = ByteString::from(data.clone());
		assert_eq!(a2, b2, "ByteString == ByteString");
		assert_eq!(a1, a2, "ByteStr == ByteString");
		assert_eq!(a2, a1, "ByteString == ByteStr");
	}

	#[quickcheck]
	fn split_eq(data: Vec<u8>, split: usize) -> TestResult {
		if split >= data.len() {
			return TestResult::discard()
		}

		let (a, b) = data.split_at(split);
		let split_str = ByteStr::from(vec![a, b]);
		let full_str = ByteStr::from(&*data);
		let owned_str = ByteString::from(data.clone());

		assert_eq!(split_str, full_str, "ByteStr (split) == ByteStr (full)");
		assert_eq!(full_str, split_str, "ByteStr (full) == ByteStr (split)");
		assert_eq!(split_str, owned_str, "ByteStr (split) == ByteString");
		assert_eq!(owned_str, split_str,  "ByteString == ByteStr (split)");
		TestResult::passed()
	}

	#[quickcheck]
	fn encode_rolling(data: Vec<u8>, split: usize) -> TestResult {
		if split >= data.len() {
			return TestResult::discard()
		}

		let (a, b) = data.split_at(split);
		let bstr = ByteStr::from(a) + ByteStr::from(b);

		assert_eq!(bstr.base64(),     STANDARD_NO_PAD.encode(&data), "standard base64");
		assert_eq!(bstr.base64_url(), URL_SAFE_NO_PAD.encode(&data), "url-safe base64");
		assert_eq!(bstr.hex_lower(), lower::encode_string(&data), "lowercase hex");
		assert_eq!(bstr.hex_upper(), upper::encode_string(&data), "uppercase hex");
		TestResult::passed()
	}
}
