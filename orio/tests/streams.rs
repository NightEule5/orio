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

mod dataset;

mod hash_canterbury {
	use std::error::Error;
	use std::fs::File;
	use std::path::PathBuf;
	use orio::{BufferOptions, ByteString, ReaderSource};
	use orio::streams::{BufSource, SourceBuffer};
	use crate::corpus_test;

	corpus_test! { hash }

	fn hash(path: PathBuf, size: usize, sha2: &str) -> Result<(), Box<dyn Error>> {
		let hash = ByteString::from(base16ct::lower::decode_vec(sha2)?);
		let mut source = {
			let file = File::open(path)?;
			ReaderSource::from(file).buffer_with(
				BufferOptions::default()
					.set_compact_threshold(usize::MAX)
					.set_fork_reluctant(false)
					.set_retention_ratio(0)
			)
		};

		let bytes = source.read_byte_str(size)?;

		assert_eq!(bytes.sha256(), hash, "invalid hash");
		Ok(())
	}
}
