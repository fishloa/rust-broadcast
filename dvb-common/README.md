# dvb-common

[![crates.io](https://img.shields.io/crates/v/dvb-common.svg)](https://crates.io/crates/dvb-common)
[![docs.rs](https://img.shields.io/docsrs/dvb-common)](https://docs.rs/dvb-common)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

Shared primitives for the DVB crate family. Every sibling crate (`dvb-si`,
`dvb-t2mi`, `dvb-bbframe`, …) depends on this crate and nothing else.

## `no_std` + `alloc` support

`dvb-common` is `#![no_std]` when the `std` feature is disabled. Only `alloc`
is required — suitable for embedded/RTOS targets with a heap. The `std` feature
(on by default) re-enables `std::error::Error` integration and `chrono`'s
wall-clock support. `core::error::Error` is used for the error trait bound in
`no_std` builds (stabilised in Rust 1.81). The MJD↔calendar float arithmetic
uses `libm::floor` uniformly in both `std` and `no_std` builds, so the output
is bit-identical on all targets.

## What's in here

### `Parse<'a>` and `Serialize` traits (`traits`)

The symmetric contract every wire type across the family implements:

```rust
pub trait Parse<'a>: Sized {
    type Error;
    fn parse(bytes: &'a [u8]) -> Result<Self, Self::Error>;
}

pub trait Serialize {
    type Error;
    fn serialized_len(&self) -> usize;
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, Self::Error>;
    fn to_bytes(&self) -> Vec<u8> where Self::Error: core::fmt::Debug;
}
```

`Parse` is lifetime-parametric so parsed structs can borrow directly from the
input buffer (zero-copy). `Serialize` is split from `Parse` so owned types can
implement it without carrying a lifetime. `to_bytes()` is a convenience
allocator; `serialize_into` is the low-level building block.

### CRC-32 MPEG-2 (`crc32_mpeg2`)

Polynomial `0x04C1_1DB7`, initial value `0xFFFF_FFFF`, MSB-first, no
reflection, no final XOR — the CRC used by every PSI/SI section and every T2-MI
packet. A precomputed 256-entry table is built at compile time (zero runtime
initialisation cost).

```rust
let crc = dvb_common::crc32_mpeg2::compute(&section_bytes[..section_bytes.len() - 4]);
```

### BCD codec (`bcd`)

Binary-coded decimal helpers for the packed nibble fields throughout DVB
(frequencies, symbol rates, HHMMSS time fields, HHMM offsets). Every encode has
a symmetric decode; both return `None` rather than producing garbage on
out-of-range input.

```rust
use dvb_common::bcd;
assert_eq!(bcd::from_bcd_byte(0x42), Some(42));
assert_eq!(bcd::to_bcd_byte(42),     Some(0x42));
```

### MJD+BCD time codec (`time`)

Decodes the 5-byte DVB UTC wire format (16-bit Modified Julian Date + 24-bit
BCD HHMMSS) to a plain `MjdBcdDateTime` struct (no dependency required) or, if
the `chrono` feature is enabled, to a `chrono::DateTime<Utc>`. The
`Duration` / HHMMSS helpers for event durations are always available without
`chrono`.

### Bit-field codec (`bits`)

`BitReader` and `BitWriter` for big-endian MSB-first sub-byte fields — the bit
order used throughout DVB/MPEG physical-layer signalling (L1-pre, L1-post,
MATYPE, …). Fields can span byte boundaries; up to 64 bits per field. Both sides
are symmetric.

```rust
use dvb_common::bits::{BitReader, BitWriter};

let mut buf = [0u8; 2];
let mut w = BitWriter::new(&mut buf);
w.write_bits(0b101, 3).unwrap();
w.write_bits(0x1FF, 9).unwrap();

let mut r = BitReader::new(&buf);
assert_eq!(r.read_bits(3).unwrap(), 0b101);
assert_eq!(r.read_bits(9).unwrap(), 0x1FF);
```

### `impl_spec_display!` macro

Project-wide helper that generates a `Display` impl for spec/field enums that
delegate to an inherent `fn name(&self) -> &'static str`. Keeps spec token
labels next to variant docs and greppable; removes identical boilerplate.

## Feature flags

| Feature  | Default | Description |
|----------|---------|-------------|
| `std`    | on      | Link `std`; enables `std::error::Error` and `chrono`'s clock/timezone. Without it the crate is `#![no_std]` + `alloc`. |
| `chrono` | off     | MJD↔`chrono::DateTime<Utc>` conversion in the `time` module. |

## MSRV

**1.81** — required for `core::error::Error` (used as the error trait bound in
`no_std` builds).

## Non-goals

- A shared error enum. Each DVB crate owns a domain-specific `Error`; the
  shared traits keep it that way via `type Error`.
- CRC-8 (used only by `dvb-bbframe`). Lives in the consumer.
- Anything with a non-trivial dependency. If a helper needs `bytes` / `serde`,
  it belongs in a consumer with the matching feature flag.

## License

MIT OR Apache-2.0, at your option.
