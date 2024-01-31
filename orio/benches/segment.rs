// SPDX-License-Identifier: Apache-2.0

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use pretty_assertions::assert_eq;
use orio::{Seg, SIZE};

const DATA: &[u8] = include_bytes!("../../test-data/cantrbry/fields_c");

fn alloc_segment(c: &mut Criterion) {
	c.bench_function("alloc_block", |b|
		b.iter(Seg::<SIZE>::new_block)
	);
}

fn write_segment(c: &mut Criterion) {
	let mut group = c.benchmark_group("write_segment");

	group.bench_function("contiguous block", |b| b.iter_batched(
		Seg::<SIZE>::default,
		|mut seg| assert_eq!(seg.write(DATA), Some(SIZE)),
		BatchSize::PerIteration
	));

	group.bench_function("discontiguous block", |b| b.iter_batched(
		|| {
			let mut seg: Seg = Seg::default();
			seg.write(DATA);
			seg.consume(4096);
			seg.truncate(1);
			seg
		},
		|mut seg| assert_eq!(seg.write(DATA), Some(SIZE - 1)),
		BatchSize::PerIteration
	));
	group.finish();
}

fn read_segment(c: &mut Criterion) {
	let seg: Seg = Seg::from_slice(&DATA[..SIZE]);
	let target = &mut [0; SIZE][..];

	let mut group = c.benchmark_group("read_segment");

	group.bench_function("contiguous block", |b| b.iter_batched_ref(
		|| {
			let mut seg = seg.clone();
			seg.fork();
			seg
		},
		|seg| assert_eq!(seg.read(target), SIZE),
		BatchSize::PerIteration
	));
	group.bench_function("discontiguous block", |b| b.iter_batched_ref(
		|| {
			let mut seg = seg.clone();
			seg.fork();
			seg.consume(4096);
			seg.write(&DATA[..4096]);
			seg
		},
		|seg| assert_eq!(seg.read(target), SIZE),
		BatchSize::PerIteration
	));
	group.bench_function("slice", |b| b.iter(||
		assert_eq!(seg.clone().read(target), SIZE)
	));

	let mut target: Seg = Seg::default();
	group.bench_with_input("slice into block", &seg, |b, seg| b.iter(|| {
		assert_eq!(target.write_from(&mut seg.clone()), Some(SIZE));
		target.clear();
	}));
	group.finish();
}

fn push(c: &mut Criterion) {
	let mut seg: Seg = Seg::default();
	c.bench_function("push", |b| b.iter(|| {
		for i in 0..SIZE {
			let _ = seg.push(DATA[i]);
		}
		seg.clear();
	}));
}

criterion_group!(benches, alloc_segment, write_segment, read_segment, push);
criterion_main!(benches);
