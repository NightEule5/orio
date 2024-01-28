// SPDX-License-Identifier: Apache-2.0

use std::fmt::Debug;
use std::mem;
use num_traits::{NumCast, PrimInt, Signed, zero};
use super::{ByteString, ByteStr};

mod sealed {
	pub trait ParseBytes { }
	impl ParseBytes for super::ByteStr<'_> { }
	impl ParseBytes for super::ByteString { }
}

/// Parses a value from a byte string.
pub trait FromByteStr: Sized {
	type Error;

	/// Parses a value from a reference to an owned, contiguous byte string.
	fn from_contiguous_bytes(bytes: &ByteString) -> Result<Self, Self::Error> {
		Self::from_segmented_bytes(&bytes.as_byte_str())
	}
	/// Parses a value from a borrowed, segmented byte string.
	fn from_segmented_bytes(bytes: &ByteStr) -> Result<Self, Self::Error>;
}

#[derive(Copy, Clone, Debug, thiserror::Error)]
pub enum ParseIntError {
	#[error("empty byte string")]
	Empty,
	#[error("invalid digit {digit:X} found in byte string")]
	InvalidDigit {
		digit: u8
	},
	#[error("number too large to fit in target type")]
	PosOverflow,
	#[error("number too small to fit in target type")]
	NegOverflow,
	#[error("number is zero in non-zero target type")]
	Zero,
}

/// A number containing a valid radix in range `[2, 36]`.
#[derive(Copy, Clone, Debug)]
pub struct Radix(u32);
#[derive(Copy, Clone, Debug, thiserror::Error)]
#[error("radix must be in range [2, 36], but it was {0}")]
pub struct RadixError(u32);

pub trait ParseBytes: sealed::ParseBytes {
	/// Parses a value from  the byte string.
	fn parse<T: FromByteStr>(&self) -> Result<T, T::Error>;
	/// Parses an integer with a `radix` from the byte string. The radix is checked
	/// within the range `[2, 36]`, representing digits from `0-9` and `A-Z`.
	fn parse_int<N: PrimInt>(&self, radix: Radix) -> Result<N, ParseIntError>;

	/// Parses the byte string into an integer from decimal digits `0-9`.
	#[inline]
	fn parse_decimal_int<N: PrimInt>(&self) -> Result<N, ParseIntError> {
		self.parse_int(Radix::DEC)
	}

	/// Parses the byte string into an integer from hexadecimal digits `0-9` and
	/// `A-F` (uppercase or lowercase).
	#[inline]
	fn parse_hex_int<N: PrimInt>(&self) -> Result<N, ParseIntError> {
		self.parse_int(Radix::HEX)
	}

	/// Parses the byte string into an integer from binary digits `0` and `1`.
	#[inline]
	fn parse_binary_int<N: PrimInt>(&self) -> Result<N, ParseIntError> {
		self.parse_int(Radix::BIN)
	}
}

impl From<u8> for ParseIntError {
	#[inline]
	fn from(digit: u8) -> Self {
		Self::InvalidDigit { digit }
	}
}

impl TryFrom<u32> for Radix {
	type Error = RadixError;

	/// Creates a radix from an integer in the range `[2, 36]`, returning an error
	/// if the value is outside that range.
	fn try_from(value: u32) -> Result<Self, Self::Error> {
		match value {
			2..=36 => Ok(Self(value)),
			_ => Err(RadixError(value))
		}
	}
}

impl Radix {
	/// Binary.
	pub const BIN: Self = Self(2);
	/// Octal.
	pub const OCT: Self = Self(8);
	/// Decimal.
	pub const DEC: Self = Self(10);
	/// Hexadecimal.
	pub const HEX: Self = Self(16);
}

impl Radix {
	fn as_num<N: NumCast>(&self) -> N {
		let num = N::from(self.0);
		debug_assert!(
			num.is_some(),
			"maximum radix value should be small enough to fit in any integer type"
		);
		unsafe { num.unwrap_unchecked() }
	}
}

impl ParseBytes for ByteStr<'_> {
	#[inline]
	fn parse<T: FromByteStr>(&self) -> Result<T, T::Error> {
		T::from_segmented_bytes(self)
	}

	#[inline]
	fn parse_int<N: PrimInt>(&self, radix: Radix) -> Result<N, ParseIntError> {
		parse_num(self.as_ref(), self.len, radix)
	}
}

impl ParseBytes for ByteString {
	#[inline]
	fn parse<T: FromByteStr>(&self) -> Result<T, T::Error> {
		T::from_contiguous_bytes(self)
	}

	#[inline]
	fn parse_int<N: PrimInt>(&self, radix: Radix) -> Result<N, ParseIntError> {
		if self.is_empty() { return Err(ParseIntError::Empty) }
		let mut data = &self.data[..];
		let sign = Sign::parse::<N>(data[0], data.len())?;
		if sign.is_explicit() {
			data = &data[1..];
		}
		parse_num_slice(zero(), data, radix, &sign, no_overflow::<N>(radix.0, data.len()))
	}
}

#[inline(always)]
fn to_digit<N: PrimInt>(d: u8, Radix(r): Radix) -> Result<N, u8> {
	let digit = (d as char).to_digit(r).ok_or(d).map(N::from)?;
	debug_assert!(
		digit.is_some(),
		"to_digit should return a value small enough to fit in any integer type"
	);
	unsafe {
		Ok(digit.unwrap_unchecked())
	}
}

trait IsSigned {
	const IS_SIGNED: bool;
}

impl<N: Signed> IsSigned for N {
	const IS_SIGNED: bool = true;
}

impl<N> IsSigned for N {
	default const IS_SIGNED: bool = false;
}

#[derive(Copy, Clone)]
enum Sign {
	ExplicitPositive,
	ImplicitPositive,
	Negative
}

impl Sign {
	fn is_positive(&self) -> bool {
		matches!(
			self,
			Self::ExplicitPositive |
			Self::ImplicitPositive)
	}

	fn is_explicit(&self) -> bool {
		matches!(
			self,
			Self::ExplicitPositive |
			Self::Negative
		)
	}

	fn parse<N: IsSigned>(digits: u8, len: usize) -> Result<Self, ParseIntError> {
		match digits {
			b'+' if len > 1 => Ok(Self::ExplicitPositive),
			b'-' if N::IS_SIGNED && len > 1 => Ok(Self::Negative),
			d @ (b'-' | b'+') => Err(d.into()),
			_ => Ok(Self::ImplicitPositive),
		}
	}
}

#[inline(always)]
fn no_overflow<N>(radix: u32, digits: usize) -> bool {
	radix <= 16 && digits <= mem::size_of::<N>() * 2 - N::IS_SIGNED as usize
}

/// A version of [`core::num::from_str_radix`] which takes multiple byte slices
/// instead of a single `str`. Optimizations from there should also apply here,
/// i.e. bit-shifting when the radix can be expressed as a sum of powers of 2,
/// out-of-order multiplication otherwise.
fn parse_num<N: PrimInt>(data: &[&[u8]], len: usize, radix: Radix) -> Result<N, ParseIntError> {
	let first = data.iter()
					.find_map(|s| s.first())
					.ok_or(ParseIntError::Empty)?;
	let sign = Sign::parse::<N>(*first, len)?;
	let mut skip = sign.is_explicit();
	let no_overflow = no_overflow::<N>(radix.0, len - skip as usize);
	data.iter().copied().try_fold(N::zero(), |num, mut digits| {
		if skip && !digits.is_empty() {
			skip = false;
			digits = &digits[1..];
		}
		parse_num_slice(num, digits, radix, &sign, no_overflow)
	})
}

fn parse_num_slice<N: PrimInt>(
	mut num: N,
	digits: &[u8],
	radix: Radix,
	sign: &Sign,
	no_overflow: bool,
) -> Result<N, ParseIntError> {
	if no_overflow {
		if sign.is_positive() {
			for &d in digits {
				num = num * radix.as_num() + to_digit(d, radix)?;
			}
		} else {
			for &d in digits {
				num = num * radix.as_num() - to_digit(d, radix)?;
			}
		}
	} else if sign.is_positive() {
		for &d in digits {
			let mul = num.checked_mul(&radix.as_num());
			let d = to_digit(d, radix)?;
			num = mul.and_then(|v| v.checked_add(&d))
					 .ok_or(ParseIntError::PosOverflow)?;
		}
	} else {
		for &d in digits {
			let mul = num.checked_mul(&radix.as_num());
			let d = to_digit(d, radix)?;
			num = mul.and_then(|v| v.checked_sub(&d))
					 .ok_or(ParseIntError::NegOverflow)?;
		}
	}
	Ok(num)
}

#[cfg(test)]
mod test {
	use std::fmt::Debug;
	use num_traits::{AsPrimitive, NumAssign, PrimInt};
	use pretty_assertions::assert_eq;
	use quickcheck::{Arbitrary, Gen};
	use quickcheck_macros::quickcheck;
	use crate::{ByteStr, ByteString, ParseBytes, Radix};

	impl Arbitrary for Radix {
		fn arbitrary(g: &mut Gen) -> Self {
			let r = u32::arbitrary(g) % 34 + 2;
			r.try_into().unwrap()
		}

		fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
			Box::new(
				(2..self.0).rev().map(|r| r.try_into().unwrap())
			)
		}
	}

	fn to_str_radix<N: PrimInt + NumAssign + AsPrimitive<u32>>(mut i: N, r: u32) -> ByteString
	where i32: AsPrimitive<N>,
		  u32: AsPrimitive<N> {
		let mut str = Vec::new();
		let is_neg = i < N::zero();
		let rn = {
			let mut rs = r as i32;
			if is_neg {
				rs = -rs;
			}
			rs.as_()
		};
		loop {
			let d = (i % rn).to_i128().unwrap().abs() as u32;
			i /= r.as_();
			str.push(char::from_digit(d, r).unwrap() as u8);

			if i == N::zero() {
				break
			}
		}

		if is_neg {
			str.push(b'-');
		}
		str.reverse();
		str.into()
	}

	macro_rules! contiguous {
		($($ty:ident)+) => {
			$(
			paste::paste! {
				#[quickcheck]
				fn [<parse_contiguous_ $ty>](radix: Radix, value: $ty) {
					parse_contiguous(value, radix);
				}
			}
			)+
		};
	}

	contiguous! { u8 u16 u32 u64 usize i8 i16 i32 i64 isize }

	fn parse_contiguous<N: Debug + PrimInt + NumAssign + AsPrimitive<u32>>(value: N, radix: Radix)
	where i32: AsPrimitive<N>,
		  u32: AsPrimitive<N> {
		let bytes = to_str_radix(value, radix.0);
		assert_eq!(
			bytes.parse_int::<N>(radix)
				 .unwrap(),
			value
		);
	}

	#[derive(Debug, Clone)]
	struct SplitNum<N> {
		value: N,
		radix: Radix,
		bytes: [Vec<u8>; 2],
	}

	impl<N: Arbitrary + PrimInt + NumAssign + AsPrimitive<u32>> Arbitrary for SplitNum<N>
	where u32: AsPrimitive<N>,
		  i32: AsPrimitive<N> {
		fn arbitrary(g: &mut Gen) -> Self {
			let value = N::arbitrary(g);
			let radix = Radix::arbitrary(g);
			let bytes = {
				let mut bytes = to_str_radix(value, radix.0).data.take_bytes();
				let split = usize::arbitrary(g) % bytes.len();
				let bytes_b = bytes.split_off(split);
				[bytes, bytes_b]
			};
			Self { value, radix, bytes }
		}
	}

	macro_rules! split {
		($($ty:ident)+) => {
			$(
			paste::paste! {
				#[quickcheck]
				fn [<parse_split_ $ty>](value: SplitNum<$ty>) {
					parse_split(value);
				}
			}
			)+
		};
	}

	split! { u8 u16 u32 u64 usize i8 i16 i32 i64 isize }

	fn parse_split<N: Debug + PrimInt>(
		SplitNum { value, radix, bytes }: SplitNum<N>
	) {
		let bytes = bytes.iter().map(Vec::as_slice).collect::<ByteStr>();
		assert_eq!(bytes.parse_int::<N>(radix).unwrap(), value);
	}
}
