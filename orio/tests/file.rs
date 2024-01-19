// SPDX-License-Identifier: Apache-2.0

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use pretty_assertions::assert_str_eq;
use tempfile::tempfile;
use orio::streams::{BufSource, FileSource, SourceExt, Result, FileSink, SinkExt, BufSink, WriterSink};
use crate::dataset::{Data, DATASET};

mod dataset;

const DATA: Data = DATASET.fields_c;

#[test]
fn file_source() -> Result {
	let Data { path, size, text, .. } = DATA;
	let file = File::open(path).unwrap();
	let mut source = FileSource::from(file).buffered();
	let mut target = String::with_capacity(size);
	assert_str_eq!(
		source.read_utf8_to_end(&mut target)?,
		text
	);
	Ok(())
}

#[test]
fn file_sink() -> Result {
	let Data { size, text, .. } = DATA;
	let file = tempfile()?;
	let mut sink = FileSink::from(file).buffered();
	let mut source = DATA;
	assert_eq!(sink.write_all(&mut source)?, size);

	let mut file = sink.into_inner()
					   .into_inner()
					   .unwrap();
	file.seek(SeekFrom::Start(0))?;
	let mut target = String::with_capacity(size);
	file.read_to_string(&mut target)?;
	assert_str_eq!(target, text);
	Ok(())
}
