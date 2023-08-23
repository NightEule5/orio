// SPDX-License-Identifier: Apache-2.0

#[macro_use]
extern crate pretty_assertions;

use std::cell::{RefCell, RefMut};
use std::rc::Rc;
use quickcheck_macros::quickcheck;
use orio::{Buffer, pool, Segment};
use orio::pool::{Pool, SharedPool};
use orio::streams::{BufSink, BufSource};

mod dataset;

#[derive(Default)]
struct InnerMockPool {

	count: usize
}

#[derive(Clone, Default)]
struct MockPool {
	inner: Rc<RefCell<InnerMockPool>>
}

impl Pool for InnerMockPool {
	fn claim_one(&mut self) -> Segment {
		self.count += 1;
		Segment::default()
	}

	fn collect_one(&mut self, _: Segment) {
		self.count -= 1;
	}

	fn shed(&mut self) { }
}

impl SharedPool for MockPool {
	fn get() -> Self { unimplemented!() }

	fn lock(&self) -> pool::Result<RefMut<'_, InnerMockPool>> { Ok(self.inner.borrow_mut()) }
}

#[quickcheck]
fn count(data: Vec<u8>) {
	let buffer = Buffer::from_slice(&data).unwrap();
	assert_eq!(buffer.count(), data.len())
}

#[quickcheck]
fn clear(data: Vec<u8>) {
	let pool = MockPool::default();
	{
		let mut buffer = Buffer::new(pool.clone());
		buffer.write_from_slice(&data).unwrap();
		buffer.clear().unwrap();
	}
	assert_eq!(
		Rc::into_inner(pool.inner)
			.unwrap()
			.into_inner()
			.count,
		0
	);
}

#[quickcheck]
fn request(data: Vec<u8>) {
	let mut buffer = Buffer::from_slice(&data).unwrap();
	assert!(buffer.request(data.len()).unwrap());
}

mod write {
	use quickcheck_macros::quickcheck;
	use orio::{Buffer, ByteString};

	mod primitive {
		use quickcheck_macros::quickcheck;
		use orio::{Buffer, ByteString};

		macro_rules! gen {
	    	($($ty:ident)+) => {
				$(
				#[quickcheck]
				fn $ty(v: $ty) {
					let buffer = Buffer::from_encode(v).unwrap();
					assert_eq!(buffer.as_byte_str(), ByteString::from(&v.to_be_bytes()[..]));
				}
				)+
			};
		}

		gen! { u8 i8 u16 i16 u32 i32 u64 i64 usize isize }
	}

	#[quickcheck]
	fn vec(vec: Vec<u8>) {
		let buffer = Buffer::from_slice(&vec).unwrap();
		assert_eq!(buffer.as_byte_str(), ByteString::from(vec));
	}

	#[quickcheck]
	fn str(str: String) {
		let buffer = Buffer::from_utf8(&str).unwrap();
		assert_eq!(buffer.as_byte_str(), ByteString::from(str.as_bytes()));
	}
}

mod read {
	use quickcheck_macros::quickcheck;
	use orio::Buffer;
	use orio::streams::{BufSink, BufSource};

	mod primitive {
		use quickcheck_macros::quickcheck;
		use orio::Buffer;
		use orio::streams::BufSource;

		macro_rules! gen {
	    	($($ty:ident$name:ident),+) => {
				$(
				#[quickcheck]
				fn $ty(v: $ty) {
					let mut buffer = Buffer::from_encode(v).unwrap();
					assert_eq!(buffer.$name().unwrap(), v);
				}
				)+
			};
		}

		gen! {
			u8    read_u8,
			i8    read_i8,
			u16   read_u16,
			i16   read_i16,
			u32   read_u32,
			i32   read_i32,
			u64   read_u64,
			i64   read_i64,
			usize read_usize,
			isize read_isize
		}
	}

	#[quickcheck]
	fn vec(vec: Vec<u8>) {
		let mut buffer = Buffer::from_slice(&vec).unwrap();
		let mut slice = vec![0; vec.len()];
		buffer.read_into_slice_exact(&mut slice).unwrap();
		assert_eq!(slice, vec);
	}

	#[quickcheck]
	fn num_vec(vec: Vec<i32>) {
		let mut buffer = Buffer::default();
		for &n in &vec {
			buffer.write_i32(n).unwrap()
		}

		let read = {
			let mut dst = Vec::with_capacity(vec.len());
			for _ in 0..vec.len() {
				dst.push(buffer.read_i32().unwrap())
			}
			dst
		};

		assert_eq!(read, vec);
	}

	#[quickcheck]
	fn str(str: String) {
		let mut buffer = Buffer::from_utf8(&str).unwrap();
		let mut string = String::with_capacity(str.len());
		buffer.read_all_utf8(&mut string).unwrap();
		assert_eq!(string, str);
	}
}

mod corpus {
	use std::error::Error;
	use std::fs::read;
	use std::path::PathBuf;
	use orio::{Buffer, ByteString};
	use orio::streams::BufSink;
	use crate::corpus_test;

	corpus_test! { read_write }

	fn read_write(path: PathBuf, size: usize, _sha2: &str) -> Result<(), Box<dyn Error>> {
		let bytes = read(path)?;
		let mut buf = Buffer::default();
		buf.write_from_slice(&bytes)?;

		assert_eq!(buf.count(), size);
		assert_eq!(buf.as_byte_str(), ByteString::from(bytes));
		Ok(())
	}
}
