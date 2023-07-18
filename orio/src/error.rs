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
use crate::streams::OffsetUtf8Error;
use crate::pool::Error as PoolError;
use crate::ErrorBox;

/// The error type Orio `Buffer`s, `Source`s and `Sink`s, etc.
#[derive(Debug, thiserror::Error)]
#[error("{source}")]
pub struct Error {
	pub context: Context,
	pub source: SourceError,
}

/// The source error encountered.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
#[non_exhaustive]
pub enum SourceError {
	/// The underlying stream is closed.
	#[error("stream closed")]
	Closed,
	/// End-of-stream was reached prematurely.
	#[error("premature end-of-stream")]
	Eos,
	/// An IO error.
	Io(io::Error),
	/// A segment pool error.
	Pool(#[from] PoolError),
	/// Invalid UTF-8 was encountered.
	Utf8(#[from] OffsetUtf8Error),
	/// An unknown error.
	Unknown(#[from] ErrorBox),
}

/// The operation attempted when the error was encountered.
#[derive(Copy, Clone, Debug, Default, strum::EnumIs)]
#[non_exhaustive]
pub enum Context {
	/// Unknown operation.
	#[default]
	Unknown,
	/// Reading from the buffer.
	BufRead,
	/// Writing to the buffer.
	BufWrite,
	/// Copying the buffer.
	BufCopy,
	/// Clearing the buffer.
	BufClear,
	/// Flushing the buffer.
	BufFlush,
	/// Compacting the buffer.
	BufCompact,
	/// Seeking the underlying stream.
	StreamSeek,
	/// Other operation described with a string.
	Other(&'static str),
}

pub(crate) trait ResultExt<T> {
	fn context(self, context: Context) -> crate::Result<T>;
}

impl Error {
	pub fn new(context: Context, source: SourceError) -> Self {
		Self { context, source }
	}

	pub fn closed(context: Context) -> Self {
		Self::new(context, SourceError::Closed)
	}

	pub fn eos(context: Context) -> Self {
		Self::new(context, SourceError::Eos)
	}
}

impl From<SourceError> for Error {
	fn from(value: SourceError) -> Self {
		Self::new(Context::Unknown, value)
	}
}

impl From<io::Error> for Error {
	fn from(value: io::Error) -> Self {
		<Self as From<SourceError>>::from(value.into())
	}
}

impl From<PoolError> for Error {
	fn from(value: PoolError) -> Self {
		<Self as From<SourceError>>::from(value.into())
	}
}

impl From<OffsetUtf8Error> for Error {
	fn from(value: OffsetUtf8Error) -> Self {
		<Self as From<SourceError>>::from(value.into())
	}
}

impl From<ErrorBox> for Error {
	fn from(value: ErrorBox) -> Self {
		<Self as From<SourceError>>::from(value.into())
	}
}

impl From<io::Error> for SourceError {
	fn from(value: io::Error) -> Self {
		if let io::ErrorKind::UnexpectedEof = value.kind() {
			Self::Eos
		} else {
			Self::Io(value)
		}
	}
}

use simdutf8::compat::Utf8Error;

impl From<Utf8Error> for SourceError {
	fn from(value: Utf8Error) -> Self {
		<Self as From<OffsetUtf8Error>>::from(value.into())
	}
}

impl Context {
	pub fn as_str(&self) -> &'static str {
		match self {
			Context::Unknown    => "unknown operation",
			Context::BufRead    => "read from buffer",
			Context::BufWrite   => "write to buffer",
			Context::BufCopy    => "copy buffer",
			Context::BufClear   => "clear buffer",
			Context::BufFlush   => "flush buffer",
			Context::BufCompact => "compact buffer",
			Context::StreamSeek => "seek stream",
			Context::Other(ctx) => ctx,
		}
	}
}

impl fmt::Display for Context {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.as_str())
	}
}

impl<T, E: Into<SourceError>> ResultExt<T> for Result<T, E> {
	fn context(self, context: Context) -> crate::Result<T> {
		self.map_err(|err| Error::new(context, err.into()))
	}
}

impl<T> ResultExt<T> for crate::Result<T> {
	fn context(mut self, context: Context) -> Self {
		if let Err(ref mut error) = self {
			error.context = context;
		}
		self
	}
}
