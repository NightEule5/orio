# Architecture

## Overview

Orio is an IO library, much like `std::io`, that facilitates the flow and manipulation of bytes through
the program. Unlike `std::io`, **Orio is lazy**; special consideration is taken to do as little work
as possible for data flow. It does its job efficiently and ergonomically, minimizing expensive operations
such as copying and memory allocation.

More specifically, data is buffered in *segments*, large, fixed-size chunks of memory. These can be shared and
moved between buffers instead of copying memory. A buffer consists of a chain of these segments, claimed from
and returned to a central *pool*. The pool allows segments to be reused instead of allocating more memory for
subsequent writes. The buffer grows as data is written and shrinks as data is read, by claiming or returning
segments to the pool. This allows buffers to grow without copying data over to a new array as a vector would.

Data flows through *streams*; it can be produced by sources and consumed by sinks. Sources and sinks are usually
buffered, consolidating smaller reads and writes via the internal buffer. This also allows data written from
source to sink to avoid costly moves and copies; segments simply change hands between the buffers without actually
reading and writing their contents.

## Code Map

This section lays out a rough map of important files and data structures.

### `Buffer`

The `Buffer` struct is the core of Orio: a growable, segmented byte buffer. It acts as both source and sink,
implementing all streaming functionality. Stream functions will, almost always, use a buffer function at some
point in their implementation.

### `Source` and `BufSource`

The `Source` trait defines how bytes are read from a source, say a file or TCP socket, into a sink.
Its buffered variant, `BufSource`, defines an internal buffer which acts as an intermediate sink,
from which a wider range of reading methods are defined (e.g. `read_u32`, `read_utf8`, etc.).

Sources are compatible with the standard library's `Read` trait.

### `Sink` and `BufSink`

The `Sink` trait defines how bytes are written to a sink from a source. Its buffered variant, `BufSink`,
defines an internal buffer which acts as an intermediate sink, from which a wider range of writing
methods are defined (e.g. `write_u32`, `write_utf8`, etc.).

Sinks are compatible with the standard library's `Write` trait.

### `Pool`

The `Pool` trait defines a global container which segments are claimed from and returned to, managing
memory allocation and de-allocation.
