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

use std::{fmt, io};
use std::error::Error as StdError;
use std::fmt::{Debug, Display, Formatter};

pub type ErrorBox = Box<dyn StdError + Send + Sync>;

pub trait OperationKind: Copy + Debug + Display {
	fn unknown() -> Self;
}

pub trait ErrorKind: Copy + Debug + Display {
	fn other(message: &'static str) -> Self;
}

#[derive(Debug)]
pub struct Error<O: OperationKind, E: ErrorKind> {
	op: O,
	pub(crate) kind: E,
	source: Option<ErrorBox>,
}

impl<O: OperationKind, E: ErrorKind> Display for Error<O, E> {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		let Self { op, kind, source } = self;
		if let Some(source) = source {
			write!(f, "{op} failed; {kind} ({source})")
		} else {
			write!(f, "{op} failed; {kind}")
		}
	}
}

impl<O: OperationKind, E: ErrorKind> StdError for Error<O, E> {
	fn source(&self) -> Option<&(dyn StdError + 'static)> {
		if let Some(ref source) = self.source {
			Some(source.as_ref())
		} else {
			None
		}
	}
}

impl<O: OperationKind, K: ErrorKind> Error<O, K> {
	pub(crate) fn new(
		op: O,
		kind: K,
		source: Option<ErrorBox>
	) -> Self {
		Self { op, kind, source: source.map(Into::into) }
	}

	/// Creates a new error with a custom message.
	pub fn other(
		op: O,
		message: &'static str,
		source: Option<ErrorBox>
	) -> Self {
		Self::new(op, K::other(message), source)
	}

	/// Returns the operation kind.
	pub fn operation(&self) -> O { self.op }

	/// Sets the operation kind.
	pub fn with_operation(mut self, op: O) -> Self {
		self.op = op;
		self
	}

	/// Returns the error kind.
	pub fn kind(&self) -> K { self.kind }

	/// Sets the error kind.
	pub fn with_kind(mut self, kind: K) -> Self {
		self.kind = kind;
		self
	}
}

impl<O: OperationKind, K: ErrorKind> From<&'static str> for Error<O, K> {
	fn from(value: &'static str) -> Self {
		Self::other(O::unknown(), value, None)
	}
}
