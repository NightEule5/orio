// SPDX-License-Identifier: Apache-2.0

use std::borrow::Cow;
use std::marker::PhantomData;
use crate::util::partial_utf8::{CharBuf, from_partial_utf8};

pub struct AlignedUtf8Iter<'a, I: Iterator> {
	char_buf: CharBuf,
	iter: I,
	fragment: Option<I::Item>,
	invalidated: bool,
	__lt: PhantomData<&'a I::Item>
}

impl<'a, I: IntoIterator> From<I> for AlignedUtf8Iter<'a, I::IntoIter> {
	fn from(iter: I) -> Self {
		Self {
			char_buf: CharBuf::default(),
			iter: iter.into_iter(),
			fragment: None,
			invalidated: false,
			__lt: PhantomData
		}
	}
}

impl<'a, I: Iterator<Item = &'a [u8]>> Iterator for AlignedUtf8Iter<'a, I> {
	type Item = Cow<'a, str>;

	fn next(&mut self) -> Option<Self::Item> {
		if self.invalidated {
			return None
		}

		loop {
			let mut fragment = self.fragment
								   .take()
								   .or_else(|| self.iter.next())?;
			match from_partial_utf8(&mut fragment, &mut self.char_buf) {
				Ok(next) if next.is_empty() && self.char_buf.char_width().is_some() => { }
				Ok(next) => {
					self.fragment = (!fragment.is_empty()).then_some(fragment);
					break Some(next)
				}
				Err(err) => {
					// Safety: we are passing in the same bytes we decoded. See the
					// safety not on Utf8Error::valid_in.
					let valid = unsafe { err.valid_in(fragment) };

					if err.kind.is_invalid_sequence() {
						self.invalidated = true;
						break (!valid.is_empty()).then_some(valid.into())
					}

					break Some(valid.into())
				}
			}
		}
	}
}