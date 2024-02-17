// SPDX-License-Identifier: Apache-2.0

use std::mem;
use criterion::{BatchSize, Bencher, black_box, Criterion, criterion_group, criterion_main};
use orio::{Buffer, DefaultBuffer, SIZE};
use orio::streams::{BufSink, BufSource};

const DATA: &[u8] = include_bytes!("../../test-data/cantrbry/fields_c");

fn write_slice(c: &mut Criterion) {
	c.bench_function("write_slice", |b| b.iter(|| {
		let mut buf = DefaultBuffer::default();
		buf.write_slice(DATA).unwrap();
		buf
	}));
}

fn write_numbers(c: &mut Criterion) {
	let mut group = c.benchmark_group("write_numbers");
	let mut buffer = DefaultBuffer::default();

	macro_rules! gen {
		($($fn:ident $ty:ident),+) => {
			$(
			group.bench_function(stringify!($fn), |b| b.iter(|| {
				for _ in 0..SIZE / mem::size_of::<$ty>() {
					let _ = black_box(buffer.$fn($ty::MAX));
				}
				buffer.clear();
			}));
			)+
		};
	}

	gen!(
		write_u8 u8,
		write_u16 u16,
		write_u16_le u16,
		write_u32 u32,
		write_u32_le u32,
		write_u64 u64,
		write_u64_le u64,
		write_u128 u128,
		write_u128_le u128
	);
}

#[inline(always)]
fn read_loop<R>(b: &mut Bencher, buf: &Buffer, read: impl FnMut(&mut Buffer) -> R) {
	b.iter_batched_ref(|| buf.clone(), read, BatchSize::SmallInput)
}

fn read_slice(c: &mut Criterion) {
	let mut buffer = Buffer::default();
	buffer.write_slice(DATA).unwrap();
	let target = &mut [0; DATA.len()][..];
	c.bench_function("read_slice", |b|
		read_loop(b, &buffer, |buf| buf.read_slice_exact(target).map(<[u8]>::len))
	);
}

fn read_numbers(c: &mut Criterion) {
	let mut group = c.benchmark_group("read_numbers");
	let mut buffer = DefaultBuffer::default();
	// Fill a segment with ones.
	for _ in 0..SIZE / 8 {
		let _ = buffer.write_u128(u128::MAX);
	}

	macro_rules! gen {
		($($fn:ident $ty:ident),+) => {
			$(
			group.bench_function(stringify!($fn), |b|
				read_loop(b, &buffer, |buf|
					for _ in 0..SIZE / mem::size_of::<$ty>() {
						let _ = black_box(buf.$fn());
					}
				)
			);
			)+
		};
	}

	gen!(
		read_u8 u8,
		read_u16 u16,
		read_u16_le u16,
		read_u32 u32,
		read_u32_le u32,
		read_u64 u64,
		read_u64_le u64,
		read_u128 u128,
		read_u128_le u128
	);
}

fn skip(c: &mut Criterion) {
	let mut group = c.benchmark_group("skip");
	let mut buffer = Buffer::default();
	buffer.write_slice(DATA).unwrap();

	group.bench_function("skip all", |b|
		read_loop(b, &buffer, |buf| buf.skip(DATA.len()))
	);
	group.bench_function("skip complete", |b|
		read_loop(b, &buffer, |buf| buf.skip(SIZE))
	);
	group.bench_function("skip partial", |b|
		read_loop(b, &buffer, |buf| buf.skip(4096))
	);
	group.finish();
}

fn find(c: &mut Criterion) {
	let mut group = c.benchmark_group("find");
	let mut buffer = DefaultBuffer::default();
	buffer.write_slice(DATA).unwrap();

	group.bench_function("find byte", |b| b.iter(|| buffer.find(b'<')));
	group.bench_function("find char", |b| b.iter(|| buffer.find('<')));
	group.bench_function("find str",  |b| b.iter(|| buffer.find("case")));
	group.bench_function("find predicate", |b| b.iter(||
		buffer.find(u8::is_ascii_digit as fn(&u8) -> _))
	);
	group.bench_function("find list", |b| b.iter(|| buffer.find(['<', '>', '='])));
	group.finish();
}

fn hash(c: &mut Criterion) {
	use digest::Digest;
	let mut buffer = DefaultBuffer::default();
	buffer.write_slice(DATA).unwrap();
	c.bench_function("hash", |b| b.iter(|| {
		let mut hasher = sha2::Sha256::default();
		buffer.hash(black_box(&mut hasher));
		hasher.finalize()
	}));
}

criterion_group!(write, write_slice, write_numbers);
criterion_group!(read, read_slice, read_numbers, skip, find, hash);
criterion_main!(write, read);
