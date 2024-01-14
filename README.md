# Orio

Orio is a memory-recycling IO library written in Rust, inspired by Square's [Okio][Okio] for
Kotlin. It speeds up buffered IO operations using a pool of previously allocated segments, which are
returned to the pool (recycled) when finished.

## Usage

Add `orio` to your dependencies with `cargo add orio`, or manually in  your `Cargo.toml`:

```toml
[dependencies]
orio = "0.1"
```

### `Buffer`s

The `Buffer` struct is a mutable byte buffer, containing a vector of segments. It implements the
`Source` and `Sink` traits, as well as the `BufSource` and `BufSink` traits.

```rust
extern crate orio;
use orio::{Buffer, DefaultBuffer, Source, BufSource};

fn main() -> orio::Result {
	let mut buf_a = Buffer::from_utf8("Hello world!")?;
	let mut buf_b = DefaultBuffer::default();
	
	let count: usize = buf_b.fill(buf_a, 5)?;
	assert_eq!(count, 5);
	
	let mut str_a = String::default();
	let mut str_b = String::default();
	buf_a.read_utf8_to_end(&mut str_a)?;
	buf_b.read_utf8_to_end(&mut str_b)?;
	assert_eq!(str_a, " world!");
	assert_eq!(str_b, "Hello");
}
```

When dropped, the `Buffer` returns its segments to the pool for reuse.

### `Source`s and `Sink`s

`Source`s read data into `Buffer`s. `Sink`s write data from `Buffer`s. For most use cases, these
won't be used directly. The buffered versions, `BufSource` and `BufSink`, provide more useful
operations, and can be obtained by calling `buffered` on a source or sink.

```rust
use orio::streams::{BufSource, Result};

fn read<'a>(source: &mut impl BufSource<'a>) -> Result<(u64, String)> {
    let key: u64 = source.read_u64()?;
    let len: u16 = source.read_u16()?;
    let mut value = String::default();
    source.read_utf8(&mut value, len as usize)?;
    Ok((key, value))
}

fn write<'a>(sink: &mut impl BufSink<'a>, key: u64, value: &str) -> Result {
    sink.write_u64(key)?;
    sink.write_utf8(value)?;
    sink.flush()?;
    Ok(())
}
```

[Okio]: https://github.com/square/okio