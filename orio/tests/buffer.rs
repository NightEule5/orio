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

#![feature(trait_alias)]

#[macro_use]
mod common;

use std::fmt::{Arguments, Debug};
use std::result;
use quickcheck::TestResult;
use quickcheck_macros::quickcheck;
use orio::Buffer;
use orio::streams::{BufSink, BufSource, Error};
use orio::streams::codec::{Encode, Decode};

pub fn format_qc_assert_error<L: Debug, R: Debug>(left: &L, right: &R, msg: Option<Arguments>) -> String {
	if let Some(msg) = msg {
		format!(
			"assertion failed `(left == right)`: {msg}\n \
			left: `{left:?}`,\nright: `{right:?}`",
		)
	} else {
		format!(
			"assertion failed `(left == right)`:\n \
			left: `{left:?}`,\nright: `{right:?}`",
		)
	}
}

#[quickcheck] fn    byte(b: u8) -> TestResult { read_write(b) }
#[quickcheck] fn  s_byte(b: i8) -> TestResult { read_write(b) }
#[quickcheck] fn   short(b: u16) -> TestResult { read_write(b) }
#[quickcheck] fn s_short(b: i16) -> TestResult { read_write(b) }
#[quickcheck] fn     int(b: u32) -> TestResult { read_write(b) }
#[quickcheck] fn   s_int(b: i32) -> TestResult { read_write(b) }
#[quickcheck] fn    long(b: u64) -> TestResult { read_write(b) }
#[quickcheck] fn  s_long(b: i64) -> TestResult { read_write(b) }
#[quickcheck] fn    size(b: usize) -> TestResult { read_write(b) }
#[quickcheck] fn  s_size(b: isize) -> TestResult { read_write(b) }

#[quickcheck]
fn str(str: String) -> TestResult {
	read_write(str)
}

fn read_write<T>(value: T) -> TestResult where T: Clone +
												  Encode +
												  Decode +
												  Debug +
												  Default +
												  PartialEq {
	fn to_tr(error: Error) -> TestResult {
		TestResult::error(error.to_string())
	}

	let mut read_value = T::default();
	let mut buf: Buffer = Buffer::default();
	if let Err(error) = buf.write_from(value.clone()) { return to_tr(error) }
	if let Err(error) = buf.read_into(&mut read_value, usize::MAX) {
		return to_tr(error)
	}

	qc_assert_eq!(value, read_value)
}
