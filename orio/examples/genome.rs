// SPDX-License-Identifier: Apache-2.0

//! Reads a large ~4.6MB file, the E. coli genome, decodes it into a sequence of
//! amino acids, writes the result into another file, and prints the first few codons
//! in the sequence. Orio is compared with std::io.

use std::fs::{create_dir_all, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, Write};
use std::time::Instant;
use orio::SIZE;
use orio::streams::{Result, BufSource, SourceExt, FileSource, FileSink, SinkExt, BufSink};

const IN_PATH: &str = "test-data/large/E.coli";
const OUT_PATH: &str = "test-data/out/large/E.coli";
const OUT_DIR: &str = "test-data/out/large/";

fn main() -> Result {
	let ref mut triplet = [0; 3];
	create_dir_all(OUT_DIR)?;
	let mut inp_file = File::open(IN_PATH)?;
	let mut out_file = OpenOptions::new()
		.write(true)
		.create(true)
		.truncate(true)
		.open(OUT_PATH)?;

	let mut seq = String::with_capacity(32);
	let mut count = 0;
	let mut reader = BufReader::new(&inp_file);
	let mut writer = BufWriter::new(&out_file);
	let now = Instant::now();
	while reader.read_exact(triplet).is_ok() {
		let codon = decode_triplet(triplet) as u8;
		if seq.len() < 32 {
			seq.push(codon as char);
		}

		writer.write(&[codon])?;
		count += 3;
	}
	let time = now.elapsed().as_millis();

	println!("Decoded sequence of {count} bytes via std::io in {time}ms: {seq}...");

	writer.flush()?;
	drop(writer);

	inp_file.rewind()?;
	out_file.rewind()?;
	seq.clear();
	count = 0;
	let mut source = FileSource::from(inp_file)
		.buffered_with_capacity(SIZE);
	let mut sink = FileSink::from(out_file)
		.buffered_with_capacity(SIZE);
	let now = Instant::now();
	while source.read_slice_exact(triplet).is_ok() {
		let codon = decode_triplet(triplet) as u8;
		if seq.len() < 32 {
			seq.push(codon as char);
		}

		sink.write_u8(codon)?;
		count += 3;
	}
	let time = now.elapsed().as_millis();

	println!("Decoded sequence of {count} bytes via Orio in {time}ms: {seq}...");
	Ok(())
}

#[derive(Debug)]
#[repr(u8)]
enum Amino {
	Phe = b'F', Leu = b'L', Tyr = b'Y',
	His = b'H', Gln = b'Q', Ile = b'I',
	Met = b'M', Asn = b'N', Lys = b'K',
	Val = b'V', Asp = b'D', Glu = b'E',
	Ser = b'S', Cys = b'C', Trp = b'W',
	Pro = b'P', Arg = b'R', Thr = b'T',
	Ala = b'A', Gly = b'G', Stop = b'*'
}

fn decode_triplet(triplet: &[u8; 3]) -> Amino {
	// https://www.genscript.com/tools/codon-frequency-table
	match triplet {
		b"ttt" | b"ttc" => Amino::Phe,
		b"tta" | b"ttg" | [ b'c', b't', _ ] => Amino::Leu,
		b"tat" | b"tac" => Amino::Tyr,
		b"cat" | b"cac" => Amino::His,
		b"caa" | b"cag" => Amino::Gln,
		b"att" | b"atc" | b"ata" => Amino::Ile,
		b"atg" => Amino::Met,
		b"aat" | b"aac" => Amino::Asn,
		b"aaa" | b"aag" => Amino::Lys,
		[ b'g', b't', _ ] => Amino::Val,
		b"gat" | b"gac" => Amino::Asp,
		b"gaa" | b"gag" => Amino::Glu,
		[ b't', b'c', _ ] | b"agt" | b"agc" => Amino::Ser,
		b"tgt" | b"tgc" => Amino::Cys,
		b"tgg" => Amino::Trp,
		[ b'c', b'c', _ ] => Amino::Pro,
		[ b'c', b'g', _ ] | b"aga" | b"agg" => Amino::Arg,
		[ b'a', b'c', _ ] => Amino::Thr,
		[ b'g', b'c', _ ] => Amino::Ala,
		[ b'g', b'g', _ ] => Amino::Gly,
		b"taa" | b"tag" | b"tga" => Amino::Stop,
		_ => unreachable!()
	}
}
