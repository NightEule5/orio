// SPDX-License-Identifier: Apache-2.0

use orio::{Buffer, BufferResult, SIZE, StreamResult};
use orio::pool::Pool;
use orio::streams::{Source, Stream};

#[derive(Copy, Clone)]
pub struct Data<'a> {
	/// The name of the data file.
	pub name: &'a str,
	/// The path of the data file.
	pub path: &'a str,
	/// The size of the data file.
	pub size: usize,
	/// The text contents of the data file.
	pub text: &'a str,
	/// The SHA-256 hash of the data file.
	pub hash: &'a str,
}

pub struct Dataset<'a> {
	// Canterbury set
	pub fields_c: Data<'a>,
	// Canterbury "large" set
	pub e_coli: Data<'a>,
}

macro_rules! data_path {
    ($prefix:literal$group:literal/$name:literal) => {
		concat!($prefix, "test-data/", $group, "/", $name)
	};
	($group:literal/$name:literal) => {
		data_path!("" $group/$name)
	}
}

macro_rules! data {
    ($group:literal/$name:literal, $hash:literal) => {{
		let text = include_str!(data_path!("../../" $group/$name));
		Data {
			path: data_path!($group/$name),
			name: $name,
			size: text.len(),
			text,
			hash: $hash,
		}
	}};
}

pub const DATASET: Dataset = Dataset {
	fields_c: data!("cantrbry"/"fields.c", "85d73e354cc50cec76cb5a50537cf8dc035f8cbb8480f9e1cbe2f7d6c23393c7"),
	e_coli: data!("large"/"E.coli", "9125dfd87315961ef4286f3856098069e050cc3a2abe65735fe43e69d1996f40"),
};

impl<const N: usize> Stream<N> for Data<'_> {
	fn is_closed(&self) -> bool {
		false
	}

	fn close(&mut self) -> StreamResult {
		Ok(())
	}
}

impl<'d> Source<'d, SIZE> for Data<'d> {
	fn is_eos(&self) -> bool {
		self.text.is_empty()
	}

	fn fill(&mut self, sink: &mut Buffer<'d, SIZE, impl Pool<SIZE>>, count: usize) -> BufferResult<usize> {
		let mut pushed = 0;
		for mut chunk in self.text
						 .as_bytes()
						 .chunks(SIZE) {
			if pushed >= count {
				break
			}

			let remaining = count - pushed;
			if chunk.len() > remaining {
				chunk = &chunk[..remaining];
			}

			sink.push_slice(chunk);
			pushed += chunk.len();
		}

		self.text = &self.text[pushed..];
		Ok(pushed)
	}

	fn fill_all(&mut self, sink: &mut Buffer<'d, SIZE, impl Pool<SIZE>>) -> BufferResult<usize> {
		for chunk in self.text.as_bytes().chunks(SIZE) {
			sink.push_slice(chunk);
		}
		let len = self.text.len();
		self.text = "";
		Ok(len)
	}
}
