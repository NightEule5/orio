// SPDX-License-Identifier: Apache-2.0

mod dataset;

macro_rules! qc_assert_ok {
    ($expr:expr) => {
		if let Err(err) = $expr {
			return TestResult::error(err.to_string())
		}
	};
	($a:expr, $b:expr) => {
		match (&$a, &$b) {
			(Ok(a), b) => {
				if !(*a == *b) {
					use pretty_assertions::private::CreateComparison;
					let error = format!("assertion failed: `(left == right)`\
                       \n\
                       \n{}\
                       \n",
                       (a, b).create_comparison()
					);
					return TestResult::error(error)
				}
			}
			(Err(err), _) => return TestResult::error(err.to_string())
		}
	};
}

macro_rules! qc_assert_str_eq {
    ($a:expr, $b:expr) => {
		match (&$a, &$b) {
			(a, b) => {
				if !(*a == *b) {
					let error = format!("assertion failed: `(left == right)`\
                       \n\
                       \n{}\
                       \n",
                       pretty_assertions::StrComparison::new(a, b)
                    );
					return TestResult::error(error)
				}
			}
		}
	};
}

mod write {
	use pretty_assertions::assert_eq;
	use quickcheck_macros::quickcheck;
	use orio::{Buffer, DefaultBuffer};
	use orio::streams::BufSink;

	macro_rules! gen_single {
		(u8 le) => { };
		(i8 le) => { };
		($ty:ident le) => {
			paste::paste! {
				#[quickcheck]
				fn [<write_ $ty _le>](v: $ty) {
					let mut buffer = DefaultBuffer::default();
					buffer.[<write_ $ty _le>](v).unwrap();
					assert_eq!(buffer, &v.to_le_bytes());
				}
			}
		};
		($ty:ident be) => {
			paste::paste! {
				#[quickcheck]
				fn [<write_ $ty _be>](v: $ty) {
					let mut buffer = DefaultBuffer::default();
					buffer.[<write_ $ty>](v).unwrap();
					assert_eq!(buffer, &v.to_be_bytes());
				}
			}
		}
	}

	macro_rules! gen {
		($($ty:ident),+) => {
			mod primitive {
				use super::*;
				use pretty_assertions::assert_eq;
				$(
				gen_single! { $ty be }
				gen_single! { $ty le }
				)+
			}
		};
	}

	gen! { u8, i8, u16, i16, u32, i32, u64, i64, u128, i128, usize, isize }

	#[quickcheck]
	fn vec(vec: Vec<u8>) {
		let buffer = Buffer::from_slice(&vec).detached();
		assert_eq!(buffer, &vec);
	}

	#[quickcheck]
	fn str(str: String) {
		let buffer = Buffer::from_utf8(&str).detached();
		assert_eq!(buffer, str.as_bytes());
	}
}

mod read {
	use pretty_assertions::{assert_eq, assert_str_eq};
	use std::{i128, vec};
	use std::mem::size_of;
	use quickcheck::{Arbitrary, Gen, TestResult};
	use quickcheck_macros::quickcheck;
	use orio::{Buffer, BufferOptions, DefaultBuffer, StreamResult};
	use orio::streams::{BufSink, BufSource};

	macro_rules! gen_single {
		(u8 le) => { };
		(i8 le) => { };
		($ty:ident le) => {
			paste::paste! {
				#[quickcheck]
				fn [<read_ $ty _le>](v: $ty) {
					let mut buffer = Buffer::from_int_le(v).unwrap();
					assert_eq!(buffer.[<read_ $ty _le>]().unwrap(), v);
				}
			}
		};
		($ty:ident be) => {
			paste::paste! {
				#[quickcheck]
				fn [<read_ $ty _be>](v: $ty) {
					let mut buffer = Buffer::from_int(v).unwrap();
					assert_eq!(buffer.[<read_ $ty>]().unwrap(), v);
				}
			}
		}
	}

	macro_rules! gen {
		($($ty:ident),+) => {
			mod primitive {
				use super::*;
				use pretty_assertions::assert_eq;
				$(
				gen_single! { $ty be }
				gen_single! { $ty le }
				)+
			}
		};
	}

	gen! { u8, i8, u16, i16, u32, i32, u64, i64, u128, i128, usize, isize }

	#[quickcheck]
	fn vec(vec: Vec<u8>) {
		let mut buffer = DefaultBuffer::default();
		buffer.write_from_slice(&vec).unwrap();
		let mut slice = vec![0; vec.len()];
		buffer.read_slice_exact(&mut slice).unwrap();
		assert_eq!(slice, vec);
	}

	#[quickcheck]
	fn num_vec(vec: Vec<i32>) {
		let mut buffer = DefaultBuffer::default();
		for &n in &vec {
			buffer.write_i32(n).unwrap()
		}

		let read = {
			let mut dst = Vec::with_capacity(vec.len());
			for _ in 0..vec.len() {
				dst.push(buffer.read_i32().unwrap())
			}
			dst
		};

		assert_eq!(read, vec);
	}

	#[quickcheck]
	fn str(str: String) {
		let mut buffer = DefaultBuffer::default();
		buffer.write_utf8(&str).unwrap();
		let mut string = String::with_capacity(str.len());
		buffer.read_utf8_to_end(&mut string).unwrap();
		assert_str_eq!(string, str);
	}

	#[derive(Clone, Debug)]
	enum Value {
		U8(u8),
		I8(i8),
		U16(u16, bool),
		I16(i16, bool),
		U32(u32, bool),
		I32(i32, bool),
		U64(u64, bool),
		I64(i64, bool),
		U128(u128, bool),
		I128(i128, bool),
		USize(usize, bool),
		ISize(isize, bool),
		Str(String),
	}

	impl Arbitrary for Value {
		fn arbitrary(g: &mut Gen) -> Self {
			match u8::arbitrary(g) % 13 {
				0  => Self::U8(u8::arbitrary(g)),
				1  => Self::I8(i8::arbitrary(g)),
				2  => Self::U16  (u16  ::arbitrary(g), bool::arbitrary(g)),
				3  => Self::I16  (i16  ::arbitrary(g), bool::arbitrary(g)),
				4  => Self::U32  (u32  ::arbitrary(g), bool::arbitrary(g)),
				5  => Self::I32  (i32  ::arbitrary(g), bool::arbitrary(g)),
				6  => Self::U64  (u64  ::arbitrary(g), bool::arbitrary(g)),
				7  => Self::I64  (i64  ::arbitrary(g), bool::arbitrary(g)),
				8  => Self::U128 (u128 ::arbitrary(g), bool::arbitrary(g)),
				9  => Self::I128 (i128 ::arbitrary(g), bool::arbitrary(g)),
				10 => Self::USize(usize::arbitrary(g), bool::arbitrary(g)),
				11 => Self::ISize(isize::arbitrary(g), bool::arbitrary(g)),
				_  => Self::Str(String::arbitrary(g)),
			}
		}

		fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
			match self {
				Self::U8(v) => Box::new(v.shrink().map(Self::U8)),
				Self::I8(v) => Box::new(v.shrink().map(Self::I8)),
				&Self::U16  (v, le) => Box::new(v.shrink().map(move |v| Self::U16  (v, le))),
				&Self::I16  (v, le) => Box::new(v.shrink().map(move |v| Self::I16  (v, le))),
				&Self::U32  (v, le) => Box::new(v.shrink().map(move |v| Self::U32  (v, le))),
				&Self::I32  (v, le) => Box::new(v.shrink().map(move |v| Self::I32  (v, le))),
				&Self::U64  (v, le) => Box::new(v.shrink().map(move |v| Self::U64  (v, le))),
				&Self::I64  (v, le) => Box::new(v.shrink().map(move |v| Self::I64  (v, le))),
				&Self::U128 (v, le) => Box::new(v.shrink().map(move |v| Self::U128 (v, le))),
				&Self::I128 (v, le) => Box::new(v.shrink().map(move |v| Self::I128 (v, le))),
				&Self::USize(v, le) => Box::new(v.shrink().map(move |v| Self::USize(v, le))),
				&Self::ISize(v, le) => Box::new(v.shrink().map(move |v| Self::ISize(v, le))),
				Self::Str(v) => Box::new(v.shrink().map(Self::Str)),
			}
		}
	}

	#[derive(amplify_derive::From)]
	enum Bytes<'s> {
		_1 (#[from] [u8; 1]),
		_2 (#[from] [u8; 2]),
		_4 (#[from] [u8; 4]),
		_8 (#[from] [u8; 8]),
		_16(#[from] [u8; 16]),
		Str(#[from] &'s [u8])
	}

	impl AsRef<[u8]> for Bytes<'_> {
		fn as_ref(&self) -> &[u8] {
			match self {
				Bytes::_1(v) => v,
				Bytes::_2(v) => v,
				Bytes::_4(v) => v,
				Bytes::_8(v) => v,
				Bytes::_16(v) => v,
				Bytes::Str(v) => v
			}
		}
	}

	impl Value {
		fn size_of(values: &[Self]) -> usize {
			values.iter().map(|value|
				match value {
					Self::U8   (_   ) | Self::I8   (_   ) => size_of::<u8>(),
					Self::U16  (_, _) | Self::I16  (_, _) => size_of::<u16>(),
					Self::U32  (_, _) | Self::I32  (_, _) => size_of::<u32>(),
					Self::U64  (_, _) | Self::I64  (_, _) |
					Self::USize(_, _) | Self::ISize(_, _) => size_of::<u64>(),
					Self::U128 (_, _) | Self::I128 (_, _) => size_of::<u128>(),
					Self::Str(str) => str.len()
				}
			).sum()
		}

		fn bytes_of(values: &[Self]) -> Vec<u8> {
			let mut vec = Vec::with_capacity(Self::size_of(values));
			for bytes in values.iter().map(Self::bytes) {
				vec.extend_from_slice(bytes.as_ref());
			}
			vec
		}
		
		fn bytes(&self) -> Bytes<'_> {
			match self {
				&Self::U8(v) => [v].into(),
				&Self::I8(v) => [v as u8].into(),
				Self::U16(v, false) => v.to_be_bytes().into(),
				Self::U16(v, true ) => v.to_le_bytes().into(),
				Self::I16(v, false) => v.to_be_bytes().into(),
				Self::I16(v, true ) => v.to_le_bytes().into(),
				Self::U32(v, false) => v.to_be_bytes().into(),
				Self::U32(v, true ) => v.to_le_bytes().into(),
				Self::I32(v, false) => v.to_be_bytes().into(),
				Self::I32(v, true ) => v.to_le_bytes().into(),
				Self::U64(v, false) => v.to_be_bytes().into(),
				Self::U64(v, true ) => v.to_le_bytes().into(),
				Self::I64(v, false) => v.to_be_bytes().into(),
				Self::I64(v, true ) => v.to_le_bytes().into(),
				Self::U128(v, false) => v.to_be_bytes().into(),
				Self::U128(v, true ) => v.to_le_bytes().into(),
				Self::I128(v, false) => v.to_be_bytes().into(),
				Self::I128(v, true ) => v.to_le_bytes().into(),
				&Self::USize(v, false) => (v as u64).to_be_bytes().into(),
				&Self::USize(v, true ) => (v as u64).to_le_bytes().into(),
				&Self::ISize(v, false) => (v as i64).to_be_bytes().into(),
				&Self::ISize(v, true ) => (v as i64).to_le_bytes().into(),
				Self::Str(str) => str.as_bytes().into()
			}
		}
		
		fn write(&self, buf: &mut Buffer) -> StreamResult {
			match self {
				&Value::U8(v) => buf.write_u8(v),
				&Value::I8(v) => buf.write_i8(v),
				&Value::U16(v, false) => buf.write_u16   (v),
				&Value::U16(v, true ) => buf.write_u16_le(v),
				&Value::I16(v, false) => buf.write_i16   (v),
				&Value::I16(v, true ) => buf.write_i16_le(v),
				&Value::U32(v, false) => buf.write_u32   (v),
				&Value::U32(v, true ) => buf.write_u32_le(v),
				&Value::I32(v, false) => buf.write_i32   (v),
				&Value::I32(v, true ) => buf.write_i32_le(v),
				&Value::U64(v, false) => buf.write_u64   (v),
				&Value::U64(v, true ) => buf.write_u64_le(v),
				&Value::I64(v, false) => buf.write_i64   (v),
				&Value::I64(v, true ) => buf.write_i64_le(v),
				&Value::USize(v, false) => buf.write_usize   (v),
				&Value::USize(v, true ) => buf.write_usize_le(v),
				&Value::ISize(v, false) => buf.write_isize   (v),
				&Value::ISize(v, true ) => buf.write_isize_le(v),
				&Value::U128(v, false) => buf.write_u128   (v),
				&Value::U128(v, true ) => buf.write_u128_le(v),
				&Value::I128(v, false) => buf.write_i128   (v),
				&Value::I128(v, true ) => buf.write_i128_le(v),
				Value::Str(v) => {
					buf.write_utf8(v)?;
					Ok(())
				}
			}
		}

		fn compare_with_read(&self, buf: &mut Buffer) -> TestResult {
			match self {
				&Value::U8(v) => qc_assert_ok!(buf.read_u8(), v),
				&Value::I8(v) => qc_assert_ok!(buf.read_i8(), v),
				&Value::U16(v, false) => qc_assert_ok!(buf.read_u16   (), v),
				&Value::U16(v, true ) => qc_assert_ok!(buf.read_u16_le(), v),
				&Value::I16(v, false) => qc_assert_ok!(buf.read_i16   (), v),
				&Value::I16(v, true ) => qc_assert_ok!(buf.read_i16_le(), v),
				&Value::U32(v, false) => qc_assert_ok!(buf.read_u32   (), v),
				&Value::U32(v, true ) => qc_assert_ok!(buf.read_u32_le(), v),
				&Value::I32(v, false) => qc_assert_ok!(buf.read_i32   (), v),
				&Value::I32(v, true ) => qc_assert_ok!(buf.read_i32_le(), v),
				&Value::U64(v, false) => qc_assert_ok!(buf.read_u64   (), v),
				&Value::U64(v, true ) => qc_assert_ok!(buf.read_u64_le(), v),
				&Value::I64(v, false) => qc_assert_ok!(buf.read_i64   (), v),
				&Value::I64(v, true ) => qc_assert_ok!(buf.read_i64_le(), v),
				&Value::USize(v, false) => qc_assert_ok!(buf.read_usize   (), v),
				&Value::USize(v, true ) => qc_assert_ok!(buf.read_usize_le(), v),
				&Value::ISize(v, false) => qc_assert_ok!(buf.read_isize   (), v),
				&Value::ISize(v, true ) => qc_assert_ok!(buf.read_isize_le(), v),
				&Value::U128(v, false) => qc_assert_ok!(buf.read_u128   (), v),
				&Value::U128(v, true ) => qc_assert_ok!(buf.read_u128_le(), v),
				&Value::I128(v, false) => qc_assert_ok!(buf.read_i128   (), v),
				&Value::I128(v, true ) => qc_assert_ok!(buf.read_i128_le(), v),
				Value::Str(v) => {
					let ref mut str = String::with_capacity(v.len());
					qc_assert_ok!(buf.read_utf8_count(str, v.len()));
					qc_assert_str_eq!(str, v);
				}
			}
			TestResult::passed()
		}
	}

	#[quickcheck]
	fn mixed_vec(vec: Vec<Value>) -> TestResult {
		let ref mut buffer: DefaultBuffer = BufferOptions::default().always_allocate().into();
		buffer.write_from_slice(&Value::bytes_of(&vec)).unwrap();

		for value in vec {
			match value.compare_with_read(buffer) {
				tr if tr.is_error() => return tr,
				_ => { }
			}
		}

		TestResult::passed()
	}
}

use pretty_assertions::assert_str_eq;
use quickcheck::{Arbitrary, Gen, TestResult};
use quickcheck_macros::quickcheck;
use orio::DefaultBuffer;
use orio::streams::{BufSource, BufSink};
use crate::dataset::DATASET;

#[derive(Copy, Clone, Debug)]
struct Span<const LEN: usize> {
	offset: usize,
	length: usize
}

impl<const LEN: usize> Arbitrary for Span<LEN> {
	fn arbitrary(g: &mut Gen) -> Self {
		let offset = usize::arbitrary(g) % (LEN - 1);
		let length = usize::arbitrary(g).saturating_add(1) % 32.min(LEN - offset);
		Self { offset, length }
	}

	fn shrink(&self) -> Box<dyn Iterator<Item = Self>> {
		let loc = *self;
		Box::new((1..self.length).rev().map(move |length| Self { length, ..loc }))
	}
}

/// Test reading random spans of up to 32 bytes from fields.c.
#[quickcheck]
fn corpus(Span { offset, length }: Span<{DATASET.fields_c.size}>) -> TestResult {
	let source = &DATASET.fields_c.text[offset..][..length];
	let mut buffer = DefaultBuffer::default();

	qc_assert_ok!(buffer.write_utf8(source));
	let ref mut str = String::with_capacity(length);
	qc_assert_ok!(buffer.read_utf8_to_end(str));
	assert_str_eq!(str, source);
	TestResult::passed()
}
