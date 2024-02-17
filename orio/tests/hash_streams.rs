// SPDX-License-Identifier: Apache-2.0

#![cfg(feature = "sha2")]

mod dataset;

use pretty_assertions::{assert_eq, assert_str_eq};
use orio::{DefaultBuffer, EncodeBytes, SIZE};
use orio::streams::{BufSink, BufSource, BufStream, HashSink, HashSource, HashStream, Sink, Source, void_sink};
use crate::dataset::{Data, DATASET};

#[test]
fn hash_source() {
	let mut source = HashSource::sha256(DATASET.fields_c);
	let mut buffer = DefaultBuffer::default();

	let mut read = 0;
	while !source.is_eos() {
		read += source.fill(&mut buffer, SIZE).unwrap();
		buffer.clear();
	}

	let src_hash = source.take_hash().hex_lower_string();
	let Data { hash, size, .. } = DATASET.fields_c;
	assert_eq!(read, size, "should read to end");
	assert_str_eq!(hash, src_hash, "hashes should match");
}

#[test]
fn buf_hash_source() {
	let mut source = HashSource::sha256(
		DefaultBuffer::from_utf8(DATASET.fields_c.text)
	);
	let mut buffer = vec![0; SIZE];

	let mut read = 0;
	while !source.is_eos() {
		read += source.read_slice(&mut buffer).unwrap().len();
	}

	let src_hash = source.take_hash().hex_lower_string();
	let Data { hash, size, .. } = DATASET.fields_c;
	assert_eq!(read, size, "should read to end");
	assert_str_eq!(hash, src_hash, "hashes should match");
}

#[test]
fn hash_sink() {
	let mut source = DATASET.fields_c;
	let mut sink = HashSink::sha256(void_sink());
	let mut buffer = DefaultBuffer::default();

	let mut written = 0;
	while !source.is_eos() {
		source.fill(&mut buffer, SIZE).unwrap();
		written += sink.drain_all(&mut buffer).unwrap();
	}

	let sink_hash = sink.take_hash().hex_lower_string();
	let Data { hash, size, .. } = DATASET.fields_c;
	assert_eq!(written, size, "should write to end");
	assert_str_eq!(hash, sink_hash, "hashes should match");
}

#[test]
fn buf_hash_sink() {
	let mut source = DATASET.fields_c;
	let mut sink = HashSink::sha256(DefaultBuffer::default());

	let mut written = 0;
	while !source.is_eos() {
		written += sink.write(&mut source, SIZE).unwrap();
		sink.buf_mut().clear();
	}

	let sink_hash = sink.take_hash().hex_lower_string();
	let Data { hash, size, .. } = DATASET.fields_c;
	assert_eq!(written, size, "should write to end");
	assert_str_eq!(hash, sink_hash, "hashes should match");
}
