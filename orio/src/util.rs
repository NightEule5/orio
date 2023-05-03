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

// Todo: Move these to a separate crate.

/// A compiler-checked assertion:
/// https://nora.codes/post/its-time-to-get-hyped-about-const-generics-in-rust/
pub struct Assert<const CONDITION: bool>;

pub trait IsTrue { }

impl IsTrue for Assert<true> { }

#[macro_export]
macro_rules! expect {
    ($expr:expr,$($msg:tt)+) => {
		match $expr {
			Ok(v) => v,
			Err(_) => panic!($($msg)+)
		}
	};
}
