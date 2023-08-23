// SPDX-License-Identifier: Apache-2.0

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
