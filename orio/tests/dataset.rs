// SPDX-License-Identifier: Apache-2.0

#[derive(Copy, Clone)]
pub struct Data<'a> {
	/// The name of the data file.
	pub name: &'a str,
	/// The size of the data file.
	pub size: usize,
	/// The text contents of the data file.
	pub text: &'a str,
	/// The SHA-256 hash of the data file.
	pub hash: &'a str,
}

pub struct Dataset<'a> {
	// Canterbury set
	pub fields_c: Data<'a>,
	// Canterbury "large" set
	pub e_coli: Data<'a>,
}

macro_rules! data {
    ($group:literal/$name:literal, $hash:literal) => {{
		let text = include_str!(concat!("../../test-data/", $group, "/", $name));
		Data {
			name: $name,
			size: text.len(),
			text,
			hash: $hash,
		}
	}};
}

pub const DATASET: Dataset = Dataset {
	fields_c: data!("cantrbry"/"fields.c", "85d73e354cc50cec76cb5a50537cf8dc035f8cbb8480f9e1cbe2f7d6c23393c7"),
	e_coli: data!("large"/"E.coli", "9125dfd87315961ef4286f3856098069e050cc3a2abe65735fe43e69d1996f40"),
};
