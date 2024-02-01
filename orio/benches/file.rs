// SPDX-License-Identifier: Apache-2.0

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::time::Duration;
use criterion::{Criterion, criterion_group, criterion_main};
use tempfile::tempfile;
use orio::streams::{BufSource, FileSink, FileSource, Sink, SinkExt, SourceExt};

const PATH: &str = "../test-data/cantrbry/fields_c";

fn file_read_write(c: &mut Criterion) {
	c.bench_function("file_read_write", |b| b.iter(|| {
		let mut source = FileSource::open(PATH).unwrap().buffered();
		let mut sink = FileSink::from(tempfile().unwrap()).buffered();
		source.read_all(&mut sink).unwrap();
		sink.flush().unwrap();
	}));
}

fn file_read_write_with_std(c: &mut Criterion) {
	c.bench_function("file_read_write_with_std", |b| b.iter(|| {
		let mut reader = BufReader::new(File::open(PATH).unwrap());
		let mut writer = BufWriter::new(tempfile().unwrap());
		loop {
			let data = reader.fill_buf().unwrap();
			if data.is_empty() {
				break
			}

			writer.write_all(data).unwrap();
			let written = data.len();
			reader.consume(written);
		}
		writer.flush().unwrap();
	}));
}

// https://github.com/bheisler/criterion.rs/issues/162
criterion_group! {
	name = benches;
	config = Criterion::default()
		.sample_size(10)
		.warm_up_time(Duration::from_millis(5))
		.measurement_time(Duration::from_millis(50));
	targets = file_read_write, file_read_write_with_std
}
criterion_main!(benches);
