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

use std::{fmt, io, result};
use std::error::Error as StdError;
use std::fmt::{Debug, Display, Formatter};
use amplify_derive::Display;
use crate::{pool, streams};
use crate::pool::Error as PoolError;

pub type ErrorBox = Box<dyn StdError>;

pub(crate) trait OperationKind: Copy + Debug + Display {
	fn unknown() -> Self;
}

pub(crate) trait ErrorKind: Copy + Debug + Display {
	fn other(message: &'static str) -> Self;
}

#[derive(Debug)]
pub struct Error<O: OperationKind, E: ErrorKind> {
	op: O,
	kind: E,
	source: Option<ErrorBox>,
}

impl<O: OperationKind, E: ErrorKind> fmt::Display for Error<O, E> {
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
		self.source.as_deref()
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
	pub fn other<M: AsRef<str>>(
		op: O,
		message: M,
		source: Option<ErrorBox>
	) -> Self {
		Self::new(op, K::other(message.as_ref()), source)
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

impl From<&str> for Error {
	fn from(value: &str) -> Self {
		Self::other(OperationKind::Unknown, value, None)
	}
}

impl From<PoolError> for OperationKind {
	fn from(value: PoolError) -> Self {
		match value {
			PoolError::Claim   => OperationKind::SegClaim,
			PoolError::Recycle => OperationKind::SegRecycle
		}
	}
}

impl From<PoolError> for Error {
	fn from(value: PoolError) -> Self {
		Self::pool
	}
}
