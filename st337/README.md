# st337

[![Crates.io](https://img.shields.io/crates/v/st337.svg)](https://crates.io/crates/st337)
[![docs.rs](https://img.shields.io/docsrs/st337)](https://docs.rs/st337)

SMPTE ST 337:2015 burst-preamble framing for non-PCM audio and data carried in
an AES3 serial digital audio interface: typed parse/serialize of the
`Pa`/`Pb`/`Pc`/`Pd` (and extended `Pe`/`Pf`) preamble plus the opaque payload,
`no_std` + `alloc`.

## What it does / what it does not

ST 337 defines how compressed or non-PCM data (for example AC-3, E-AC-3 or
DTS audio) rides inside the sample words of an AES3 stream: a repeating **data
burst** made of a preamble (sync words, an info word, a length word) and a
payload. Use this crate to find, read or build those bursts once you have the
word sequence as bytes (for example from an SDI audio embedder, a `.wav`
holding an S/PDIF stream, or a test generator).

It does **not**:

- implement the AES3 physical layer (biphase-mark line coding, subframe and
  time-slot bit placement, channel-status words);
- map `data_type` to a codec: that registry is SMPTE ST 338, which is not
  vendored, so `data_type` is a validated `u8` (0 to 31) rather than an enum
  with invented names;
- support the 20-bit and 24-bit `data_mode` values: `DataMode::Mode20/Mode24`
  parse as enum values but `BurstPreamble` rejects them with
  `Error::UnsupportedDataMode` (their words are not a whole number of bytes);
- decode or re-encode the payload (compressed audio stays opaque bytes);
- implement IEC 61937 (the consumer S/PDIF sibling). `length_code` is read as
  **bits** per ST 337's own text; a real IEC 61937 E-AC-3 capture writes bytes
  for the same data type (documented in `docs/st337-PROVENANCE.md`).

## Standards supported

| Standard | Edition | Scope / clauses | Status | Notes |
|----------|---------|-----------------|--------|-------|
| SMPTE ST 337 - Format for Non-PCM Audio and Data in an AES3 Serial Digital Audio Interface | 2015 (vendored; revision of 337-2008) | Section 7.1/7.2 burst and preamble: `Pa`/`Pb` sync, `Pc` (data_type, data_mode, error_flag, data_type_dependent, data_stream_number), `Pd` length code, extended-type `Pe`/`Pf` (7.2.4), Table 8 `data_mode` | Partial | 16-bit mode only; word sequence as little-endian 16-bit words in a byte stream |
| SMPTE ST 338 - Non-PCM data_type registry | not vendored | none | Not implemented | `data_type` is an opaque validated number |
| AES3-3 / IEC 61937 | n/a | none | Not implemented | Cross-checked against a real E-AC-3 capture (`ffmpeg -f spdif`) for sync constants and `Pc` layout |

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `std`   | yes     | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | no      | `serde::Serialize`/`Deserialize` derives on public types. |

## Quick start

```rust
use broadcast_common::{Parse, Serialize};
use st337::{Burst, DataMode};

let payload = [0xDE, 0xAD, 0xBE, 0xEF];
let burst = Burst::new(1, DataMode::Mode16, false, 0, 0, None, &payload).unwrap();

let mut bytes = vec![0u8; burst.serialized_len()];
burst.serialize_into(&mut bytes).unwrap();
assert_eq!(Burst::parse(&bytes).unwrap(), burst);
```

More: `cargo run -p st337 --example build_burst`, `--example parse_burst`.

## What is implemented

- `Burst`: a preamble plus the opaque `burst_payload` slice (zero-copy
  borrowed on parse), built with `Burst::new` (which computes `length_code`
  for you).
- `BurstPreamble`: `data_type`, `data_mode`, `error_flag`,
  `data_type_dependent`, `data_stream_number`, `length_code`, and an optional
  `ExtendedPreamble` (`Pe`/`Pf`) used when `data_type` equals
  `EXTENDED_DATA_TYPE_MARKER`.
- `DataMode`: the Table 8 values (`Mode16`, `Mode20`, `Mode24`, `Reserved`).
- Constants: `SYNC_WORD_PA`, `SYNC_WORD_PB`, `MAX_5_BIT_FIELD`,
  `MAX_DATA_STREAM_NUMBER`, `MAX_LENGTH_CODE_BITS`.
- A real committed E-AC-3 burst (`tests/fixtures/eac3_frame0.bin`) round-trips
  byte-exact in `tests/it/fixture_eac3.rs`.

## Design notes

- Symmetric `Parse` / `Serialize` with round-trip tests; the preamble is
  validated (`Error::InvalidSync`, `InvalidValue`, `ExtendedPreambleMismatch`,
  `PayloadTooLarge`) so a corrupt burst cannot re-serialize differently.
- Words are modelled as 2 bytes each, little-endian per 16-bit word (explained
  in the `burst` module docs), matching the `ffmpeg -f spdif` oracle.
- Public enums are `#[non_exhaustive]` with `name()` + `Display` labels.
- `no_std` + `alloc`; one `thiserror` error enum.

## Related crates

Depends only on `broadcast-common`. The payloads it carries are compressed
audio or data (the committed fixture is an E-AC-3 burst); this crate never
parses them.

## Minimum Supported Rust Version

1.95.0

## License

MIT OR Apache-2.0
