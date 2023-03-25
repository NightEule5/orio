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

use std::error;
use amplify_derive::Display;

#[derive(Copy, Clone, Debug, Display)]
pub enum ErrorKind {
	#[cfg(feature = "shared-pool")]
	#[display("could not get lock, mutex was poisoned")]
	Poison,
	#[display("could not borrow the pool, already in use")]
	PoolBorrow,
	#[display("invalid operation on closed stream")]
	Closed,
	#[display("could not clear the buffer")]
	BufClear,
	#[display("buffered write from source failed")]
	BufWrite,
	#[display("buffered read to sink failed")]
	BufRead,
	#[display("buffered sink could not be flushed to inner sink")]
	BufFlush,
	#[display("buffered stream could not be closed")]
	BufClose,
}

#[derive(Debug, Display)]
#[display("{kind}")]
pub struct Error {
	kind: ErrorKind,
	source: Option<Box<dyn error::Error>>,
}

impl error::Error for Error {
	fn source(&self) -> Option<&(dyn error::Error + 'static)> {
		self.source.as_deref()
	}
}

impl Error {
	fn new(
		kind: ErrorKind,
		source: impl error::Error + 'static
	) -> Self {
		Self {
			kind,
			source: Some(Box::new(source)),
		}
	}

	#[cfg(feature = "shared-pool")]
	pub(crate) fn poison(error: std::sync::PoisonError<&mut Vec<Segment>>) -> Self {
		Self {
			kind: ErrorKind::Poison,
			source: Some(Box::new(error)),
		}
	}

	pub(crate) fn borrow(error: impl error::Error + 'static) -> Self {
		Self::new(ErrorKind::PoolBorrow, error)
	}

	pub(crate) fn closed() -> Self {
		Self {
			kind: ErrorKind::Closed,
			source: None
		}
	}

	pub(crate) fn buf_clear(error: impl error::Error + 'static) -> Self {
		Self::new(ErrorKind::BufClear, error)
	}

	pub(crate) fn buf_write(error: impl error::Error + 'static) -> Self {
		Self::new(ErrorKind::BufWrite, error)
	}

	pub(crate) fn buf_read(error: impl error::Error + 'static) -> Self {
		Self::new(ErrorKind::BufRead, error)
	}

	pub(crate) fn buf_flush(error: impl error::Error + 'static) -> Self {
		Self::new(ErrorKind::BufFlush, error)
	}

	pub(crate) fn buf_close(error: impl error::Error + 'static) -> Self {
		Self::new(ErrorKind::BufClose, error)
	}
}
