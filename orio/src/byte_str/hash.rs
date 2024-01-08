// SPDX-License-Identifier: Apache-2.0

#![cfg(feature = "hash")]

use digest::Digest;
use crate::{ByteStr, ByteString};

mod sealed {
	pub trait HashBytes { }
	impl HashBytes for super::ByteStr<'_> { }
	impl HashBytes for super::ByteString { }
}

macro_rules! hash {
    ($sec:tt$feature:literal$module:ident
	$($size_name:literal$size_fn:ident$size_hasher:ident)+
	) => {
		$(
		hash! {
			$sec
			$size_name
			#[cfg(feature = $feature)]
			fn $size_fn(&self) -> ByteString {
				self.hash::<$module::$size_hasher>()
			}
		}
		)+
	};
    (secure $name:literal$method:item) => {
		/// Computes a
		#[doc = $name]
		/// hash of the byte string. There are no known attacks on this hash
		/// function; it can be considered suitable for cryptography.
		$method
	};
    (broken $name:literal$method:item) => {
		/// Computes a
		#[doc = $name]
		/// hash of the byte string. This hash function has been broken; its use in
		/// cryptography is ***not*** secure. Use for checksums only.
		$method
	};
}

pub trait HashBytes: sealed::HashBytes {
	/// Hashes the byte string with `digest`, returning the hash.
	fn hash<D: Digest>(&self) -> ByteString;

	hash! {
		secure "groestl" groestl
		"Grøstl-224" groestl224 Groestl224
		"Grøstl-256" groestl256 Groestl256
		"Grøstl-384" groestl384 Groestl384
		"Grøstl-512" groestl512 Groestl512
	}

	hash! {
		broken "md5" md5
		"MD5" md5 Md5
	}

	hash! {
		broken "sha1" sha1
		"SHA1" sha1 Sha1
	}

	hash! {
		secure "sha2" sha2
		"SHA-224" sha224 Sha224
		"SHA-256" sha256 Sha256
		"SHA-384" sha384 Sha384
		"SHA-512" sha512 Sha512
	}

	hash! {
		secure "sha3" sha3
		"SHA3-224 (Keccak)" sha3_224 Sha3_224
		"SHA3-256 (Keccak)" sha3_256 Sha3_256
		"SHA3-384 (Keccak)" sha3_384 Sha3_384
		"SHA3-512 (Keccak)" sha3_512 Sha3_512
	}

	hash! {
		secure "shabal" shabal
		"Shabal-192" shabal192 Shabal192
		"Shabal-224" shabal224 Shabal224
		"Shabal-256" shabal256 Shabal256
		"Shabal-384" shabal384 Shabal384
		"Shabal-512" shabal512 Shabal512
	}

	hash! {
		secure "whirlpool" whirlpool
		"Whirlpool" whirlpool Whirlpool
	}
}

impl HashBytes for ByteStr<'_> {
	fn hash<D: Digest>(&self) -> ByteString {
		let mut digest = D::new();
		for data in self.data.iter() {
			digest.update(data)
		}
		digest.finalize()
			  .as_slice()
			  .into()
	}
}

impl HashBytes for ByteString {
	fn hash<D: Digest>(&self) -> Self {
		D::new()
			.chain_update(&self.data)
			.finalize()
			.as_slice()
			.into()
	}
}
