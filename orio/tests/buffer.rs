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

use std::cell::{RefCell, RefMut};
use std::ops::DerefMut;
use std::rc::Rc;
use quickcheck_macros::quickcheck;
use orio::{Buffer, pool, Segment, SEGMENT_SIZE};
use orio::pool::{BasicPool, Pool, SharedPool};
use orio::streams::{BufSink, BufSource};

#[macro_use]
mod common;

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
	use orio::streams::BufSource;

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
	fn str(str: String) {
		let mut buffer = Buffer::from_utf8(&str).unwrap();
		let mut string = String::with_capacity(str.len());
		buffer.read_all_utf8(&mut string).unwrap();
		assert_eq!(string, str);
	}
}
