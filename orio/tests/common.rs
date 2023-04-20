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

use std::fmt::{Arguments, Debug};

macro_rules! qc_assert_eq {
	($left:expr,$right:expr) => {{
		let left = $left;
		let right = $right;
		if left == right {
			TestResult::passed()
		} else {
			TestResult::error(
				common::format_qc_assert_error(&left, &right, None)
			)
		}
	}};
    ($left:expr,$right:expr,$($arg:tt)+) => {{
		let left = $left;
		let right = $right;
		if left == right {
			TestResult::passed()
		} else {
			TestResult::error(
				common::format_qc_assert_error(&left, &right, Some(format_args!($($arg)+)))
			)
		}
	}};
}

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
