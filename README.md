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
use orio::{Buffer, Source, BufSource};

fn main() {
	let mut buf_a = Buffer::try_from("Hello world!").unwrap();
	let mut buf_b = Buffer::new();
	
	let count: usize = buf_b.read(buf_a, 5).unwrap();
	assert_eq!(count, 5);
	
	let str_a: String = buf_a.read_all().unwrap();
	let str_b: String = buf_b.read_all().unwrap();
	assert_eq!(str_a, " world!");
	assert_eq!(str_b, "Hello");
}
```

When dropped, the `Buffer` returns its segments to the pool for reuse.

### `Source`s and `Sink`s

`Source`s provide data to be read into `Buffer`s. `Sink`s consume data written from `Buffer`s. For
use cases, these won't be used directly. The buffered versions, `BufSource` and `BufSink`, provide
more useful operations, and can be obtained by calling `buffer` on a `Source` or `Sink`.

Sources implement `read`:

```rust
use orio::{Buffer, Source};

struct MySource {
	data: String
}

impl Source for MySource {
	type Error = orio::Error;
	fn read(&mut self, buffer: Buffer, count: usize) -> orio::Result {
		buffer.write_from(&mut self.data, count)
	}
}
```

Sinks implement `write`:

```rust
use orio::{Buffer, Sink};

struct MySink {
	data: String
}

impl Sink for MySink {
	type Error = orio::Error;
	fn write(&mut self, buffer: Buffer, count: usize) -> orio::Result {
		buffer.read_from(&mut self.data, count)
	}
}
```

[Okio]: https://github.com/square/okio