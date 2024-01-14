// SPDX-License-Identifier: Apache-2.0

use std::cmp::min;
use base64::prelude::{BASE64_STANDARD_NO_PAD, BASE64_URL_SAFE_NO_PAD};
use crate::{ByteStr, ByteString};

macro_rules! buffer {
    ($($method:ident).+($($params:expr),*)) => {{
		let mut buf = String::default();
		$($method).+($($params,)* &mut buf);
		buf
	}};
}

mod private {
	pub trait EncodeSpec {
		fn encode<'a>(
			&self,
			encoder: &impl Encoder,
			target: &'a mut String
		) -> &'a str;
	}

	pub trait Encoder {
		const WIDTH: usize;
		fn encode_data<'a>(&self, input: &[u8], dst: &'a mut String) -> &'a str;
	}
}

pub trait EncodeBytes: private::EncodeSpec {
	/// Returns a string containing the data encoded into base64.
	#[inline]
	fn base64_string(&self) -> String {
		buffer!(self.base64())
	}

	/// Returns a string containing the data encoded into URL-safe base64.
	#[inline]
	fn base64_url_string(&self) -> String {
		buffer!(self.base64_url())
	}

	/// Returns a string containing the data encoded into base64 with a custom
	/// encoder.
	#[inline]
	fn base64_string_with(&self, encoder: &impl base64::Engine) -> String {
		buffer!(self.base64_with(encoder))
	}

	/// Returns a string containing the data encoded into lowercase hex.
	#[inline]
	fn hex_lower_string(&self) -> String {
		buffer!(self.hex_lower())
	}

	/// Returns a string containing the data encoded into uppercase hex.
	#[inline]
	fn hex_upper_string(&self) -> String {
		buffer!(self.hex_upper())
	}

	/// Writes data encoded into base64 to `target`, returning a slice containing
	/// the written data.
	#[inline]
	fn base64<'a>(&self, target: &'a mut String) -> &'a str {
		self.base64_with(&BASE64_STANDARD_NO_PAD, target)
	}

	/// Writes data encoded into URL-safe base64 to `target`, returning a slice
	/// containing the written data.
	#[inline]
	fn base64_url<'a>(&self, target: &'a mut String) -> &'a str {
		self.base64_with(&BASE64_URL_SAFE_NO_PAD, target)
	}

	/// Writes data encoded into base64, with a custom `encoder`, to `target`,
	/// returning a slice containing the written data.
	#[inline]
	fn base64_with<'a>(&self, encoder: &impl base64::Engine, target: &'a mut String) -> &'a str {
		self.encode(encoder, target)
	}

	/// Writes data encoded into lowercase hex to `target`, returning a slice
	/// containing the written data.
	#[inline]
	fn hex_lower<'a>(&self, target: &'a mut String) -> &'a str {
		self.encode(&LowerHexEncoder, target)
	}

	/// Writes data encoded into uppercase hex to `target`, returning a slice
	/// containing the written data.
	#[inline]
	fn hex_upper<'a>(&self, target: &'a mut String) -> &'a str {
		self.encode(&UpperHexEncoder, target)
	}
}

impl<T: private::EncodeSpec> EncodeBytes for T { }

impl private::EncodeSpec for ByteStr<'_> {
	fn encode<'a>(&self, encoder: &impl private::Encoder, target: &'a mut String) -> &'a str {
		if self.data.len() == 1 {
			encoder.encode_data(self.data[0], target)
		} else {
			let mut enc = RollingEncoder::new(target, encoder);
			for data in self.data.iter() {
				enc.encode(data);
			}

			enc.finish()
		}
	}
}

impl private::EncodeSpec for ByteString {
	#[inline]
	fn encode<'a>(&self, encoder: &impl private::Encoder, dst: &'a mut String) -> &'a str {
		encoder.encode_data(&self.data, dst)
	}
}

impl<T: base64::Engine> private::Encoder for T {
	const WIDTH: usize = 3;
	fn encode_data<'a>(&self, data: &[u8], dst: &'a mut String) -> &'a str {
		let cur_len = dst.len();
		self.encode_string(data, dst);
		&dst[cur_len..]
	}
}

struct LowerHexEncoder;
struct UpperHexEncoder;

#[inline(always)]
fn encode_hex<'a, const UPPER: bool>(input: &[u8], buf: &'a mut String) -> &'a str {
	let encode = if UPPER {
		base16ct::upper::encode_str
	} else {
		base16ct::lower::encode_str
	};

	let cur_len = buf.len();
	let enc_len = base16ct::encoded_len(input);
	buf.reserve(enc_len);
	unsafe {
		// Safety: The UTF-8 constraint is held since only hex digits are written
		// to the string. set_len is safe since the added bytes are initialized by
		// encode immediately after. Finally, the unwrap can be unchecked because
		// we know the slice is large enough to fit the encoded digits.
		let buf = buf.as_mut_vec();
		buf.set_len(cur_len + enc_len);
		let buf = &mut buf.as_mut_slice()[cur_len..];
		encode(input, buf).unwrap_unchecked()
	}
}

impl private::Encoder for LowerHexEncoder {
	const WIDTH: usize = 2;
	fn encode_data<'a>(&self, input: &[u8], dst: &'a mut String) -> &'a str {
		encode_hex::<false>(input, dst)
	}
}

impl private::Encoder for UpperHexEncoder {
	const WIDTH: usize = 2;
	fn encode_data<'a>(&self, input: &[u8], dst: &'a mut String) -> &'a str {
		encode_hex::<true>(input, dst)
	}
}

/// Encodes slices as a multiple of `WIDTH`, rolling over the remainder into the
/// next slice. This ensures segmented data will be encoded the same as equivalent
/// contiguous data would be.
struct RollingEncoder<'a, 'b: 'a, E: private::Encoder> {
	buf: &'b mut String,
	len: usize,
	remainder: Option<&'a [u8]>,
	encoder: &'a E,
}

impl<'a, 'b: 'a, E: private::Encoder> RollingEncoder<'a, 'b, E> {
	fn new(buf: &'b mut String, encoder: &'a E) -> Self {
		let len = buf.len();
		Self {
			buf,
			len,
			remainder: None,
			encoder,
		}
	}

	fn encode(&mut self, mut data: &'a [u8]) {
		let width = E::WIDTH;

		if let Some(value) = self.remainder.take() {
			let mut rem = Vec::with_capacity(width);
			rem.extend_from_slice(value);

			let len = min(width - value.len(), data.len());
			rem.extend_from_slice(&data[..len]);
			self.encoder.encode_data(&rem, self.buf);
			data = &data[len..];
		}

		let clean_len = data.len() / width * width;
		if clean_len < data.len() {
			let _ = self.remainder.insert(&data[clean_len..]);
		}

		self.encoder.encode_data(&data[..clean_len], self.buf);
	}

	fn finish(self) -> &'b str {
		if let Some(rem) = self.remainder {
			self.encoder.encode_data(rem, self.buf);
		}
		&self.buf[self.len..]
	}
}
