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

use crate::{Buffer, Result};
use crate::pool::SharedPool;
use super::{Sink, Source};

/// Returns a [`Sink`] that writes to nowhere, dropping any data written to it.
pub fn void_sink() -> VoidSink { VoidSink }

/// Returns a [`Source`] that reads from nowhere, producing no data.
pub fn void_source() -> VoidSource { VoidSource }

/// A [`Sink`] that writes to nowhere, dropping any data written to it.
#[derive(Copy, Clone, Debug, Default)]
pub struct VoidSink;

impl Sink for VoidSink {
	/// Skips `count` bytes at `source`.
	fn write(&mut self, source: &mut Buffer<impl SharedPool>, count: usize) -> Result<usize> {
		source.skip(count)
	}

	/// Skips all bytes at `source`.
	fn write_all(&mut self, source: &mut Buffer<impl SharedPool>) -> Result<usize> {
		source.skip_all()
	}
}

/// A [`Source`] that reads from nowhere, producing no data.
#[derive(Copy, Clone, Debug, Default)]
pub struct VoidSource;

impl Source for VoidSource {
	/// Reads nothing, returning `0`.
	fn read(&mut self, _sink: &mut Buffer<impl SharedPool>, _count: usize) -> Result<usize> {
		Ok(0)
	}

	/// Reads nothing, returning `0`.
	fn read_all(&mut self, _: &mut Buffer<impl SharedPool>) -> Result<usize> {
		Ok(0)
	}
}
