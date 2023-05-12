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

use std::error::Error;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use ctor::ctor;
use fetch_data::FetchData;
use zip::ZipArchive;

//noinspection SpellCheckingInspection
#[ctor]
static CANTERBURY_ZIP: FetchData = FetchData::new(
	"cantrbry.zip c44b686dfc137e74aba4db0540e5d6568cb09e270ba8f8411d2f9df24f39a1a6",
	"http://corpus.canterbury.ac.nz/resources/",
	"ORIO_TEST_DATA_DIR",
	"dev",
	"strixpyrr",
	"orio"
);

/// A set of text files from [The Canterbury Corpus][].
///
/// [The Canterbury Corpus]: https://corpus.canterbury.ac.nz/descriptions/
//noinspection SpellCheckingInspection
pub struct Dataset {
		root_dir: PathBuf,
	pub alice29	: Data,
	pub asyoulik: Data,
	pub cp		: Data,
	pub fields	: Data,
	pub grammar	: Data,
	pub kennedy	: Data,
	pub lcet10	: Data,
	pub plrabn12: Data,
	pub ptt5	: Data,
	pub sum		: Data,
	pub xargs	: Data,
}

//noinspection SpellCheckingInspection
impl Dataset {
	const  ALICE29_HASH: &'static str = "7467306ee0feed4971260f3c87421154a05be571d944e9cb021a5713700c38f0";
	const ASYOULIK_HASH: &'static str = "eaa3526fe53859f34ecdf255712f9ecf0b2c903451d4755b2edaa2e2599cb0fc";
	const       CP_HASH: &'static str = "e0cd21cef5b6c4069461e949be100080c3ce887de6f1dd8626c480528efaaf61";
	const   FIELDS_HASH: &'static str = "85d73e354cc50cec76cb5a50537cf8dc035f8cbb8480f9e1cbe2f7d6c23393c7";
	const  GRAMMAR_HASH: &'static str = "1b0805dfc0ae706b35aac2bb4e15f02485efd24dda5dbd29de7b2f84d1a88c15";
	const  KENNEDY_HASH: &'static str = "9af47239ca29dfe20e633f80bbbb9a4cc9783d0803d7b2b5626f42e4c3790420";
	const   LCET10_HASH: &'static str = "5314ba1dbb03f471df88bec6cd120a938ef60d0fd3511c5c1dce61bf7463245f";
	const PLRABN12_HASH: &'static str = "07e2e0b461af78c7c647cb53dab39de560198e16f799b4516eccf0fbd69f764c";
	const     PTT5_HASH: &'static str = "0ec3a75089bb52342813496b17e51377bc9eba3cb519a444d67025354841d650";
	const      SUM_HASH: &'static str = "ee5733cd76ecc2f9d8ff156adc3c02a7a851051dcf43a2d56ff4ee4ff606bdb3";
	const    XARGS_HASH: &'static str = "c58aeb5d2d1e12751d47e7412b45784405fc30a5671b03d480fa05776e183619";

	fn extract() -> Result<PathBuf, Box<dyn Error>> {
		let zip_path = CANTERBURY_ZIP.fetch_file("cantrbry.zip")?;
		let out_dir = zip_path.parent().unwrap().join("cantrbry");

		if !out_dir.exists() {
			let mut zip = {
				let file = File::open(zip_path)?;
				let reader = BufReader::new(file);
				ZipArchive::new(reader)?
			};
			zip.extract(out_dir.clone())?;
		}

		return Ok(out_dir)
	}
}

#[derive(Copy, Clone)]
pub struct Data {
	pub name: &'static str,
	pub size: usize,
	pub sha2: &'static str
}

impl Data {
	pub fn path(&self) -> PathBuf {
		DATASET.root_dir.join(self.name)
	}
}

impl Data {
	const fn new(
		name: &'static str,
		size: usize,
		sha2: &'static str
	) -> Self {
		Self { name, size, sha2 }
	}
}

//noinspection SpellCheckingInspection
#[ctor]
pub static DATASET: Dataset = Dataset {
	root_dir: Dataset::extract().unwrap(),
	alice29	: Data::new("alice29.txt",	152_089,	Dataset:: ALICE29_HASH),
	asyoulik: Data::new("asyoulik.txt",	125_179,	Dataset::ASYOULIK_HASH),
	cp		: Data::new("cp.html",		24_603,		Dataset::      CP_HASH),
	fields	: Data::new("fields.c",		11_150,		Dataset::  FIELDS_HASH),
	grammar	: Data::new("grammar.lsp",	3_721,		Dataset:: GRAMMAR_HASH),
	kennedy	: Data::new("kennedy.xls",	1_029_744,	Dataset:: KENNEDY_HASH),
	lcet10	: Data::new("lcet10.txt",	426_754,	Dataset::  LCET10_HASH),
	plrabn12: Data::new("plrabn12.txt",	481_861,	Dataset::PLRABN12_HASH),
	ptt5	: Data::new("ptt5",			513_216,	Dataset::    PTT5_HASH),
	sum		: Data::new("sum",			38_240,		Dataset::     SUM_HASH),
	xargs	: Data::new("xargs.1",		4_227,		Dataset::   XARGS_HASH),
};
