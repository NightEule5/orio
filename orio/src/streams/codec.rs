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

//! Provides Encode and Decode traits for arbitrary types to interact with streams.

use std::cmp::min;
use std::mem;
use crate::{Buffer, Result};
use crate::pool::SharedPool;
use crate::streams::{BufSink, BufSource};

/// Defines encoding behavior for fixed-size types.
pub trait EncodeFixed: Sized {
	const SIZE: usize = mem::size_of::<Self>();

	/// Encodes to an array of bytes, in little-endian byte order if `le` is `true`.
	fn encode_to_bytes(self, le: bool) -> [u8; Self::SIZE];
}

/// Defines encoding behavior.
pub trait Encode {
	/// Encodes into `buf`, in little-endian byte order if `le` is `true`.
	fn encode<const N: usize>(self, buf: &mut Buffer<impl SharedPool>, le: bool) -> Result<usize>;
}

/// Defines decoding behavior for fixed-size types.
pub trait DecodeFixed: Sized {
	const SIZE: usize = mem::size_of::<Self>();

	/// Decodes from an array of bytes, in little-endian byte order if `le` is
	/// `true`.
	fn decode_from_bytes(bytes: [u8; Self::SIZE], le: bool) -> Result<Self>;
}

/// Defines decoding behavior.
pub trait Decode {
	/// Decodes at most `byte_count` bytes from `buf`, in little-endian byte order
	/// if `le` is `true`.
	fn decode<const N: usize>(&mut self, buf: &mut Buffer<impl SharedPool>, byte_count: usize, le: bool) -> Result<usize>;
}

default impl<E: EncodeFixed> Encode for E where [(); Self::SIZE]: {
	fn encode<const N: usize>(self, buf: &mut Buffer<impl SharedPool>, le: bool) -> Result<usize> {
		buf.write_from_slice(&self.encode_to_bytes(le))?;
		Ok(Self::SIZE)
	}
}

default impl<D: DecodeFixed> Decode for D where [(); Self::SIZE]: {
	fn decode<const N: usize>(&mut self, buf: &mut Buffer<impl SharedPool>, byte_count: usize, le: bool) -> Result<usize> {
		let mut arr = [0; Self::SIZE];
		let len = min(byte_count, Self::SIZE);
		buf.read_into_slice_exact(&mut arr[..len])?;

		*self = Self::decode_from_bytes(arr, le)?;
		Ok(len)
	}
}

// Bytes

impl Encode for &[u8] {
	fn encode<const N: usize>(self, buf: &mut Buffer<impl SharedPool>, _: bool) -> Result<usize> {
		let len = self.len();
		buf.write_from_slice(self)?;
		Ok(len)
	}
}

impl Decode for [u8] {
	fn decode<const N: usize>(&mut self, buf: &mut Buffer<impl SharedPool>, byte_count: usize, _: bool) -> Result<usize> {
		let len = min(byte_count, self.len());
		buf.read_into_slice(&mut self[..len])
	}
}

// Utf8

impl Encode for &str {
	fn encode<const N: usize>(self, buf: &mut Buffer<impl SharedPool>, _: bool) -> Result<usize> {
		let n = self.len();
		buf.write_utf8(self)?;
		Ok(n)
	}
}

impl Decode for str {
	fn decode<const N: usize>(&mut self, buf: &mut Buffer<impl SharedPool>, byte_count: usize, _: bool) -> Result<usize> {
		let len = min(byte_count, self.len());
		buf.read_utf8_into_slice(&mut self[..len])
	}
}

impl Encode for String {
	fn encode<const N: usize>(self, buf: &mut Buffer<impl SharedPool>, _: bool) -> Result<usize> {
		let n = self.len();
		buf.write_utf8(&*self)?;
		Ok(n)
	}
}

impl Decode for String {
	fn decode<const N: usize>(&mut self, buf: &mut Buffer<impl SharedPool>, byte_count: usize, _: bool) -> Result<usize> {
		buf.read_utf8(self, byte_count)
	}
}

// Numbers

macro_rules! gen_num_codec {
    ($($wfn:ident$rfn:ident$($wfn_le:ident$rfn_le:ident)?->$ty:ident,)+) => {
		$(gen_num_codec! { $wfn$rfn$($wfn_le$rfn_le)?$ty })+
	};
	($wfn:ident$rfn:ident$wfn_le:ident$rfn_le:ident$ty:ident) => {
		impl EncodeFixed for $ty {
			fn encode_to_bytes(self, le: bool) -> [u8; mem::size_of::<$ty>()] {
				if le { self.to_le_bytes() } else { self.to_be_bytes() }
			}
		}
		impl DecodeFixed for $ty {
			fn decode_from_bytes(bytes: [u8; mem::size_of::<$ty>()], le: bool) -> Result<$ty> {
				Ok(
					if le {
						$ty::from_le_bytes(bytes)
					} else {
						$ty::from_be_bytes(bytes)
					}
				)
			}
		}
		impl Encode for $ty {
			fn encode<const N: usize>(self, buf: &mut Buffer<impl SharedPool>, le: bool) -> Result<usize> {
				if le {
					buf.$wfn_le(self)?;
				} else {
					buf.$wfn(self)?;
				}
				Ok(mem::size_of::<$ty>())
			}
		}
		impl Decode for $ty {
			fn decode<const N: usize>(&mut self, buf: &mut Buffer<impl SharedPool>, byte_count: usize, le: bool) -> Result<usize> {
				if byte_count <= mem::size_of::<$ty>() {
					return Ok(0)
				}

				*self = if le {
					buf.$rfn_le()?
				} else {
					buf.$rfn()?
				};
				Ok(mem::size_of::<$ty>())
			}
		}
	};
	($wfn:ident$rfn:ident$ty:ident) => {
		impl EncodeFixed for $ty {
			fn encode_to_bytes(self, _: bool) -> [u8; 1] { [self as u8] }
		}
		impl DecodeFixed for $ty {
			fn decode_from_bytes([byte]: [u8; 1], _: bool) -> Result<$ty> { Ok(byte as $ty) }
		}
		impl Encode for $ty {
			fn encode<const N: usize>(self, buf: &mut Buffer<impl SharedPool>, _: bool) -> Result<usize> {
				buf.$wfn(self)?;
				Ok(1)
			}
		}
		impl Decode for $ty {
			fn decode<const N: usize>(&mut self, buf: &mut Buffer<impl SharedPool>, byte_count: usize, _: bool) -> Result<usize> {
				if byte_count == 0 { return Ok(0) }

				*self = buf.$rfn()?;
				Ok(1)
			}
		}
	};
}

gen_num_codec! {
	write_i8 read_i8 -> i8,
	write_u8 read_u8 -> u8,
	write_i16 read_i16 write_i16_le read_i16_le -> i16,
	write_u16 read_u16 write_u16_le read_u16_le -> u16,
	write_i32 read_i32 write_i32_le read_i32_le -> i32,
	write_u32 read_u32 write_u32_le read_u32_le -> u32,
	write_i64 read_i64 write_i64_le read_i64_le -> i64,
	write_u64 read_u64 write_u64_le read_u64_le -> u64,
	write_isize read_isize write_isize_le read_isize_le -> isize,
	write_usize read_usize write_usize_le read_usize_le -> usize,
}
