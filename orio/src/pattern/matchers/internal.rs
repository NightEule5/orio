// SPDX-License-Identifier: Apache-2.0

use simdutf8::compat::from_utf8;
use crate::Utf8Error;

pub fn decode_valid(bytes: &[u8]) -> (Option<&str>, usize) {
	match from_utf8(bytes).map_err(Utf8Error::from) {
		Ok(str) => (Some(str), str.len()),
		Err(err) => {
			let (valid, invalid) = unsafe { err.split_valid(bytes) };
			if valid.is_empty() {
				(None, valid.len())
			} else {
				(Some(valid), valid.len() + invalid.len())
			}
		}
	}
}
