// SPDX-License-Identifier: Apache-2.0

#![feature(maybe_uninit_slice)]

mod dataset;

use std::mem::MaybeUninit;
use pretty_assertions::{assert_eq, assert_str_eq};
use orio::{Buffer, BufferResult, DefaultBuffer, SIZE};
use orio::pool::Pool;
use orio::streams::{BufSource, Result, Sink, SourceExt, SinkExt, Stream, BufSink};
use crate::dataset::{Data, DATASET};

#[test]
fn read_all() -> Result {
	const DATA: Data = DATASET.fields_c;
	let mut source = DATA.buffered();
	let mut buffer = DefaultBuffer::default();
	let mut string = String::with_capacity(DATA.size);
	assert_eq!(source.read_all(&mut buffer)?, DATA.size);
	assert_str_eq!(buffer.read_utf8_to_end(&mut string)?, DATA.text);
	Ok(())
}

#[test]
fn read() -> Result {
	const DATA: Data = DATASET.fields_c;
	let mut source = DATA.buffered();
	let mut string = String::with_capacity(32);
	assert_eq!(source.skip(1024)?, 1024);
	assert_str_eq!(source.read_utf8(&mut string, 32)?, &DATA.text[1024..][..32]);
	Ok(())
}

#[derive(Default)]
struct VecSink {
	vec: Vec<u8>
}

impl Stream<SIZE> for VecSink {
	fn is_closed(&self) -> bool {
		false
	}

	fn close(&mut self) -> Result {
		Ok(())
	}
}

impl Sink<'_, SIZE> for VecSink {
	fn drain(&mut self, source: &mut Buffer<'_, SIZE, impl Pool<SIZE>>, count: usize) -> BufferResult<usize> {
		let len = self.vec.len();
		self.vec.reserve(count);
		let count = source.read_slice(unsafe {
			MaybeUninit::slice_assume_init_mut(
				self.vec.spare_capacity_mut()
			)
		})?;
		unsafe {
			self.vec.set_len(len + count);
		}
		Ok(count)
	}
}

#[test]
fn write_all() -> Result {
	let mut data = DATASET.fields_c;
	let contents = data.text;
	let mut sink = VecSink::default().buffered();
	assert_eq!(sink.write_all(&mut data)?, data.size);
	let string = String::from_utf8(
		sink.into_inner().vec
	).unwrap();
	assert_str_eq!(&string, contents);
	Ok(())
}

#[test]
fn write() -> Result {
	let mut data = DATASET.fields_c;
	let contents = data.text;
	let mut sink = VecSink::default().buffered();
	assert_eq!(sink.write(&mut data, 32)?, 32);
	let string = String::from_utf8(
		sink.into_inner().vec
	).unwrap();
	assert_str_eq!(&string, &contents[..32]);
	Ok(())
}
