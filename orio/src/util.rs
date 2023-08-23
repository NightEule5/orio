// SPDX-License-Identifier: Apache-2.0

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
