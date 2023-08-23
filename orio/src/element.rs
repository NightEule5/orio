// SPDX-License-Identifier: Apache-2.0

use std::mem;

/// A primitive stream element, like a byte or other integer. Defines behavior for
/// converting from and into raw bytes.
pub trait StreamElement: Copy + Default + Sized + Unpin {
	const SIZE: usize = mem::size_of::<Self>();

	/// Converts from big-endian bytes.
	fn from_bytes(value: [u8; Self::SIZE]) -> Self;
	/// Converts into big-endian bytes.
	fn into_bytes(self) -> [u8; Self::SIZE];
	/// Converts from little-endian bytes.
	fn from_le_bytes(mut value: [u8; Self::SIZE]) -> Self {
		value.reverse();
		Self::from_bytes(value)
	}
	/// Converts into little-endian bytes.
	fn into_le_bytes(self) -> [u8; Self::SIZE] {
		let mut value = self.into_bytes();
		value.reverse();
		value
	}
}

macro_rules! generate {
    ($($ty:ident)+) => {
		$(
		impl StreamElement for $ty {
			fn from_bytes([value]: [u8; Self::SIZE]) -> Self { value as $ty }
			fn into_bytes(self) -> [u8; Self::SIZE] { [self as u8] }
			fn from_le_bytes([value]: [u8; Self::SIZE]) -> Self { value as $ty }
			fn into_le_bytes(self) -> [u8; Self::SIZE] { [self as u8] }
		}
		)+
	};
}

generate! { u8 i8 }

macro_rules! generate {
    ($($ty:ident)+) => {
		$(
		impl StreamElement for $ty {
			fn from_bytes(bytes: [u8; Self::SIZE]) -> Self { $ty::from_be_bytes(bytes) }
			fn into_bytes(self) -> [u8; Self::SIZE] { self.to_be_bytes() }
			fn from_le_bytes(bytes: [u8; Self::SIZE]) -> Self { $ty::from_le_bytes(bytes) }
			fn into_le_bytes(self) -> [u8; Self::SIZE] { self.to_le_bytes() }
		}
		)+
	};
}

generate! { u16 i16 u32 i32 u64 i64 }
