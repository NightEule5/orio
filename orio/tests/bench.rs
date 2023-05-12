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

#![feature(test, try_blocks)]

extern crate test;

mod dataset;

mod std_read_slice_buf {
	use std::fs::{File, read};
	use std::io::{BufRead, BufReader, Read, Seek};
	use test::Bencher;
	use base16ct::lower::decode_vec as decode_base16_vec;
	use sha2::{Digest, Sha256};
	use crate::dataset::DATASET;

	#[bench]
	fn canterbury_cp(b: &mut Bencher) {
		let data = &DATASET.cp;
		let path = data.path();
		let hash = decode_base16_vec(data.sha2).unwrap();
		let contents = read(&path).unwrap();

		b.iter(|| {
			let mut reader = BufReader::new(&*contents);
			let mut hasher = Sha256::default();
			loop {
				let n = {
					let data = reader.fill_buf().unwrap();
					if data.is_empty() {
						break
					}

					hasher.update(data);
					data.len()
				};

				reader.consume(n);
			}

			let result = hasher.finalize();
			assert_eq!(&*result, &*hash, "invalid hash");
			result
		})
	}
}

mod orio_read_slice_buf {
	use std::fs::{File, read};
	use test::Bencher;
	use crate::dataset::DATASET;
	use base16ct::lower::decode_vec as decode_base16_vec;
	use orio::{Buffer, ReaderSource};
	use orio::streams::{BufSink, BufSource, BufStream, Error, SeekableExt, Sink, Source, SourceBuffer};

	#[bench]
	fn canterbury_cp(b: &mut Bencher) {
		let data = &DATASET.cp;
		let path = data.path();
		let size = data.size;
		let hash = decode_base16_vec(data.sha2).unwrap();
		let contents = read(&path).unwrap();

		b.iter(|| {
			let mut source = contents.buffer();
			let mut buffer = Buffer::default();
			source.read(&mut buffer, size).unwrap();
			assert_eq!(buffer.count(), contents.len());
			let result = buffer.as_byte_str().sha256();
			assert_eq!(result.as_slice(), &*hash, "invalid hash");
			result
		})
	}
}
