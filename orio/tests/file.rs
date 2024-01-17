// SPDX-License-Identifier: Apache-2.0

#![feature(round_char_boundary)]

use std::fs::File;
use pretty_assertions::{assert_eq, assert_str_eq};
use orio::{DefaultBuffer, SIZE};
use orio::streams::{BufSource, BufStream, FileSource, SourceExt};
use crate::dataset::{Data, Dataset, DATASET};

mod dataset;

#[test]
fn file_source() {
	let Data { path, size, text, .. } = DATASET.fields_c;
	let file = File::open(path).unwrap();
	let mut source = FileSource::from(file).buffered();
	let mut target = String::with_capacity(size);
	assert_eq!(
		source.read_utf8_to_end(&mut target).unwrap(),
		size
	);
	assert_str_eq!(target, text);
}
