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

use digest::Digest;
use quickcheck_macros::quickcheck;
use orio::ByteString;

#[quickcheck]
fn hash(bytes: Vec<u8>) {
	let bytes: ByteString = bytes.into();
	let mut expected_hash: ByteString;

	macro_rules! hash {
		($(mod $mod:ident with $feature:literal {
			$($name:ident$hasher:ident)+
		})+) => {
			$(
			#[cfg(feature = $feature)]
			{
				$(
				expected_hash = $mod::$hasher::default()
					.chain_update(bytes.as_slice())
					.finalize()
					.to_vec()
					.into();
				assert_eq!(bytes.$name(), expected_hash, "{} hash didn't match", stringify!($name));
				)+
			}
			)+
		};
	}

	hash! {
		mod groestl with "groestl" {
			groestl224 Groestl224
			groestl256 Groestl256
			groestl384 Groestl384
			groestl512 Groestl512
		}
		mod md5 with "md5" {
			md5 Md5
		}
		mod sha1 with "sha1" {
			sha1 Sha1
		}
		mod sha2 with "sha2" {
			sha224 Sha224
			sha256 Sha256
			sha384 Sha384
			sha512 Sha512
		}
		mod sha3 with "sha3" {
			sha3_224 Sha3_224
			sha3_256 Sha3_256
			sha3_384 Sha3_384
			sha3_512 Sha3_512
		}
		mod shabal with "shabal" {
			shabal192 Shabal192
			shabal224 Shabal224
			shabal256 Shabal256
			shabal384 Shabal384
			shabal512 Shabal512
		}
		mod whirlpool with "whirlpool" {
			whirlpool Whirlpool
		}
	}
}
