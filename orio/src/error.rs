// SPDX-License-Identifier: Apache-2.0

mod utf8;

use std::{error, fmt, io};
use std::rc::Rc;
use amplify_derive::{Display, From};
use thiserror::Error;
use crate::streams::{EndOfStream, StreamClosed};
use crate::pool::PoolError;
pub use utf8::*;

mod sealed {
	use std::fmt::{Debug, Display};

	pub trait Context: Copy + Clone + Debug + Display + Default {}
	impl Context for super::StreamContext { }
	impl Context for super::BufferContext { }
}

pub(crate) trait Context: sealed::Context { }
impl<C: sealed::Context> Context for C { }

pub type BufferError = Error<BufferContext>;
pub type StreamError = Error<StreamContext>;
pub type BufferResult<T = ()> = Result<T, BufferError>;
pub type StreamResult<T = ()> = Result<T, StreamError>;

/// An IO error.
#[derive(Clone, Debug)]
pub struct Error<C: sealed::Context> {
	pub(crate) source: ErrorSource,
	pub(crate) context: C
}

/// Context of what a buffer was doing when the error occurred.
#[derive(Copy, Clone, Debug, Display, Default)]
pub enum BufferContext {
	#[default]
	#[display("<none>")]
	#[doc(hidden)]
	None,
	/// Reading from the buffer.
	#[display("reading")]
	Read,
	/// Writing to the buffer.
	#[display("writing")]
	Write,
	/// Copying the buffer.
	#[display("copying")]
	Copy,
	/// Filling the buffer.
	#[display("filling")]
	Fill,
	/// Draining the buffer.
	#[display("draining")]
	Drain,
	/// Clearing the buffer.
	#[display("clearing")]
	Clear,
	/// Reserving space in the buffer.
	#[display("reserving in")]
	Reserve,
	/// Resizing the buffer.
	#[display("resizing")]
	Resize,
	/// Compacting the buffer.
	#[display("compacting")]
	Compact,
}

/// Context of what a stream was doing when the error occurred.
#[derive(Copy, Clone, Debug, Display, Default, From)]
pub enum StreamContext {
	#[default]
	#[display("<none>")]
	#[doc(hidden)]
	None,
	/// A buffering operation.
	#[display("{_0} buffer")]
	Buffer(BufferContext),
	/// Reading from a source.
	#[display("reading from source")]
	Read,
	/// Writing to a sink.
	#[display("writing to sink")]
	Write,
	/// Flushing to a sink.
	#[display("flushing to sink")]
	Flush,
	/// Seeking in a stream.
	#[display("seeking in stream")]
	Seek,
	/// Other operation described by a string.
	#[display(inner)]
	Other(&'static str),
}

/// The error source.
#[derive(Clone, Debug, Error, From)]
#[error(transparent)]
pub enum ErrorSource {
	/// The underlying stream is closed.
	Closed(#[from(StreamClosed)] StreamClosed),
	/// End-of-stream was reached prematurely.
	Eos(#[from(EndOfStream)] EndOfStream),
	/// An IO error.
	Io(#[from(io::Error)] Rc<io::Error>), // Rc to get around io::Error not implementing Clone
	/// A UTF-8 decode error.
	Utf8(#[from(Utf8Error)] Utf8Error),
	/// A pool error.
	Pool(#[from(PoolError)] PoolError),
	/// A stream error.
	Stream(#[from(StreamError)] Box<StreamError>),
	/// A buffer error.
	Buffer(#[from(BufferError)] Box<BufferError>),
}

pub trait ResultContext<T, C: sealed::Context> {
	fn context(self, context: C) -> Result<T, Error<C>>;
}

pub trait ResultSetContext<T, C: sealed::Context> {
	fn set_context(self, context: C) -> Result<T, Error<C>>;
}

impl<C: sealed::Context> fmt::Display for Error<C> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match (self, f.alternate()) {
			(Self { source, context }, true ) => write!(f, "{source} while {context}"),
			(Self { source, ..      }, false) => fmt::Display::fmt(source, f)
		}
	}
}

impl<C: sealed::Context + Default> From<StreamClosed> for Error<C> {
	fn from(value: StreamClosed) -> Self {
		Self {
			source: value.into(),
			context: C::default(),
		}
	}
}

impl<C: sealed::Context + Default> From<EndOfStream> for Error<C> {
	fn from(value: EndOfStream) -> Self {
		Self {
			source: value.into(),
			context: C::default(),
		}
	}
}

impl<C: sealed::Context + Default> From<io::Error> for Error<C> {
	fn from(value: io::Error) -> Self {
		Self {
			source: value.into(),
			context: C::default(),
		}
	}
}

impl<C: sealed::Context + Default> From<Utf8Error> for Error<C> {
	fn from(value: Utf8Error) -> Self {
		Self {
			source: value.into(),
			context: C::default(),
		}
	}
}

impl<C: sealed::Context + Default> From<PoolError> for Error<C> {
	fn from(value: PoolError) -> Self {
		Self {
			source: value.into(),
			context: C::default(),
		}
	}
}

impl From<BufferError> for StreamError {
	fn from(value: BufferError) -> Self {
		Self {
			source: value.source,
			context: StreamContext::Buffer(value.context),
		}
	}
}

impl From<StreamError> for BufferError {
	fn from(value: StreamError) -> Self {
		Self {
			source: value.into(),
			context: BufferContext::None,
		}
	}
}

impl<C: sealed::Context> error::Error for Error<C> {
	fn source(&self) -> Option<&(dyn error::Error + 'static)> {
		self.source.source()
	}
}

impl<C: sealed::Context> Error<C> {
	pub fn closed(context: C) -> Self {
		Self {
			source: ErrorSource::Closed(StreamClosed),
			context,
		}
	}

	/// Gets the error context.
	pub fn context(&self) -> C {
		self.context
	}

	/// Returns true if the inner error is a "closed stream".
	pub fn is_closed(&self) -> bool {
		matches!(&self.source, ErrorSource::Closed(_))
	}

	/// Returns true if the inner error is an "end-of-stream".
	pub fn is_eos(&self) -> bool {
		matches!(&self.source, ErrorSource::Eos(_))
	}

	/// Returns true if the inner error is an IO error.
	pub fn is_io_error(&self) -> bool {
		self.as_io_error().is_some()
	}

	/// Returns true if the inner error is a UTF-8 decode error.
	pub fn is_utf8_error(&self) -> bool {
		self.as_utf8_error().is_some()
	}

	/// Returns true if the inner error is a pool error.
	pub fn is_pool_error(&self) -> bool {
		self.as_pool_error().is_some()
	}

	/// Returns true if the inner error is a stream error.
	pub fn is_stream_error(&self) -> bool {
		self.as_stream_error().is_some()
	}

	/// Returns true if the inner error is a buffer error.
	pub fn is_buffer_error(&self) -> bool {
		self.as_buffer_error().is_some()
	}

	/// Returns the inner error as a "stream closed" error.
	pub fn as_closed(&self) -> Option<&StreamClosed> {
		let ErrorSource::Closed(error) = &self.source else { return None };
		Some(error)
	}

	/// Returns the inner error as an "end-of-stream" error.
	pub fn as_eos(&self) -> Option<&EndOfStream> {
		let ErrorSource::Eos(error) = &self.source else { return None };
		Some(error)
	}

	/// Returns the inner error as an IO error.
	pub fn as_io_error(&self) -> Option<&io::Error> {
		let ErrorSource::Io(error) = &self.source else { return None };
		Some(error)
	}

	/// Returns the inner error as a UTF-8 decode error.
	pub fn as_utf8_error(&self) -> Option<&Utf8Error> {
		let ErrorSource::Utf8(error) = &self.source else { return None };
		Some(error)
	}

	/// Returns the inner error as a pool error.
	pub fn as_pool_error(&self) -> Option<&PoolError> {
		let ErrorSource::Pool(error) = &self.source else { return None };
		Some(error)
	}

	/// Returns the inner error as a stream error.
	pub fn as_stream_error(&self) -> Option<&StreamError> {
		let ErrorSource::Stream(error) = &self.source else { return None };
		Some(error)
	}

	/// Returns the inner error as a buffer error.
	pub fn as_buffer_error(&self) -> Option<&BufferError> {
		let ErrorSource::Buffer(error) = &self.source else { return None };
		Some(error)
	}
}

impl<T, C: sealed::Context, E: Into<ErrorSource>> ResultContext<T, C> for Result<T, E> {
	fn context(self, context: C) -> Result<T, Error<C>> {
		self.map_err(|err| Error { source: err.into(), context })
	}
}

impl<T, C: sealed::Context> ResultSetContext<T, C> for Result<T, Error<C>> {
	fn set_context(mut self, context: C) -> Self {
		if let Err(ref mut error) = self {
			error.context = context;
		}
		self
	}
}
