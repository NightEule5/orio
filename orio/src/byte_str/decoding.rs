// SPDX-License-Identifier: Apache-2.0

use base64::prelude::{BASE64_STANDARD_NO_PAD, BASE64_URL_SAFE_NO_PAD};
use super::{ByteString, Data};

impl ByteString {
	/// Decodes base64-encoded bytes into the byte string.
	pub fn decode_base64<T: AsRef<[u8]>>(&mut self, input: T) -> Result<(), base64::DecodeError> {
		self.decode_base64_with(input, &BASE64_STANDARD_NO_PAD)
	}

	/// Decodes URL-safe base64-encoded bytes into the byte string.
	pub fn decode_base64_url<T: AsRef<[u8]>>(&mut self, input: T) -> Result<(), base64::DecodeError> {
		self.decode_base64_with(input, &BASE64_URL_SAFE_NO_PAD)
	}

	/// Decodes base64-encoded bytes into the byte string with a custom `decoder`.
	pub fn decode_base64_with<T: AsRef<[u8]>>(&mut self, input: T, decoder: &impl base64::Engine) -> Result<(), base64::DecodeError> {
		let mut buf = self.data.take_bytes();
		decoder.decode_vec(input, &mut buf)?;
		self.data = Data::Bytes(buf);
		Ok(())
	}

	/// Decodes hex bytes into the byte string.
	pub fn decode_hex<T: AsRef<[u8]>>(&mut self, input: T) -> Result<(), base16ct::Error> {
		self.extend_from_slice(
			Self::from_hex(input)?.as_slice()
		);
		Ok(())
	}

	/// Decodes base64-encoded bytes to a new byte string.
	pub fn from_base64<T: AsRef<[u8]>>(input: T) -> Result<Self, base64::DecodeError> {
		Self::from_base64_with(input, &BASE64_STANDARD_NO_PAD)
	}

	/// Decodes URL-safe base64-encoded bytes to a new byte string.
	pub fn from_base64_url<T: AsRef<[u8]>>(input: T) -> Result<Self, base64::DecodeError> {
		Self::from_base64_with(input, &BASE64_URL_SAFE_NO_PAD)
	}

	/// Decodes base64-encoded bytes to a new byte string with a custom `decoder`.
	pub fn from_base64_with<T: AsRef<[u8]>>(input: T, decoder: &impl base64::Engine) -> Result<Self, base64::DecodeError> {
		decoder.decode(input).map(Into::into)
	}

	/// Decodes hex bytes into a new byte string.
	pub fn from_hex<T: AsRef<[u8]>>(input: T) -> Result<Self, base16ct::Error> {
		base16ct::mixed::decode_vec(input).map(Into::into)
	}
}
