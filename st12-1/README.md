# st12-1

[![Crates.io](https://img.shields.io/crates/v/st12-1.svg)](https://crates.io/crates/st12-1)
[![docs.rs](https://img.shields.io/docsrs/st12-1)](https://docs.rs/st12-1)

SMPTE ST 12-1:2014 Linear Time Code (LTC): parse and build the 80-bit logical
codeword (BCD time address, drop/color flags, binary groups, sync word),
`no_std`.

## What it does / what it does not

LTC is the timecode professional video and audio equipment exchanges as an
audio-rate signal (and, in ST 12-2, inside ancillary data). This crate takes the
**already-demodulated 10-byte (80-bit) codeword** and gives you typed fields
back, and builds the same 10 bytes from typed fields. Use it in tools that
read a timecode track from a file, an MXF/ANC payload or a demodulator's
output.

It does **not**:

- decode or generate the section 9.3 biphase-mark audio waveform (clock
  recovery, zero-crossing detection, audio I/O);
- implement VITC (section 10), the LTC/VITC relationship (section 11) or the
  higher-frame-rate extensions;
- implement ST 12-2 (timecode carried in ANC packets) or ST 12-3 (high frame
  rate timecode), although both are vendored for future work;
- detect the frame rate: the codeword carries no rate field, so rate-dependent
  flag meanings are resolved against a caller-supplied `FrameRate`.

## Standards supported

| Standard | Edition | Scope / clauses | Status | Notes |
|----------|---------|-----------------|--------|-------|
| SMPTE ST 12-1 - Time and Control Code | 2014 (vendored spec verified; revision of 12M-1:2008) | Sections 8.1-8.5 (numeric code, drop/color frame flags, binary group flags, Table 1 binary-group usage), section 9.2 (LTC codeword, Tables 2-4 bit positions, Table 3 rate-dependent flag bits, sync word `0xFCBF`) | Partial (LTC codeword only) | Section 9.3 biphase waveform, section 10 VITC, section 11 and 12 not implemented |
| SMPTE ST 12-2 - Transmission of Time Code in the Ancillary Data Space | 2014 (vendored) | none | Not implemented | Transport of timecode inside ANC; see `st291` for the ANC packet container |
| SMPTE ST 12-3 - Time Code for High Frame Rate Signals | 2016 (vendored) | none | Not implemented | |

## Features

| Feature   | Default | Description |
|-----------|---------|-------------|
| `std`     | yes     | Link the standard library. Without it the crate is `#![no_std]` (no heap allocation is needed at all). |
| `serde`   | no      | `serde::Serialize`/`Deserialize` derives on public types. |

## Quick start

```rust
use broadcast_common::{Parse, Serialize};
use st12_1::LtcFrame;

let frame = LtcFrame {
    hours: 1,
    minutes: 23,
    seconds: 45,
    frames: 13,
    drop_frame_flag: false,
    color_frame_flag: true,
    flag_bit_27: true,
    flag_bit_43: true,
    flag_bit_58: false,
    flag_bit_59: true,
    user_bits: [1, 2, 3, 4, 5, 6, 7, 8],
};
let mut bytes = [0u8; st12_1::FRAME_LEN];
frame.serialize_into(&mut bytes).unwrap();
assert_eq!(LtcFrame::parse(&bytes).unwrap(), frame);
```

More: `cargo run -p st12-1 --example build_frame`, `--example parse_frame`.

## What is implemented

- `LtcFrame`: hours (0-23), minutes/seconds (0-59), frames (0-29), the
  drop-frame (bit 10) and color-frame (bit 11) flags, the four rate-dependent
  flag bits kept at their wire positions (`flag_bit_27/43/58/59`), eight 4-bit
  `user_bits` binary groups, and the fixed sync word (`SYNC_WORD`, validated on
  parse, always written on serialize). `FRAME_LEN` is 10.
- `FrameRate` (`Fps30` / `Fps25` / `Fps24`): selects Table 3's column.
  `LtcFrame::polarity_correction(rate)` and `binary_group_flags(rate)` resolve
  the flag bits (25-frame swaps the meaning of bits 27 and 59 relative to the
  other two). `FrameRate::max_frame` and `LtcFrame::validate_for_rate` give the
  rate-aware frame bound (29 / 24 / 23).
- `BinaryGroupFlags` and `BinaryGroupUsage`: Table 1's eight-way classification
  of what the binary groups contain, `BinaryGroupUsage::from_flags`.

## Design notes

- `Parse` / `Serialize` are symmetric with byte-exact round-trips; the crate
  tests against spec vectors (`tests/spec_vectors.rs`).
- Parsing is strict: each BCD digit is range-checked (`Error::InvalidBcdDigit`),
  field maxima are enforced (`Error::InvalidValue`), the sync word must match
  (`Error::SyncWordMismatch`), so a malformed codeword cannot re-serialize to
  different bytes.
- Because the rate is not on the wire, `parse` validates frames against the
  widest bound (29) and leaves the tighter per-rate check to
  `validate_for_rate`.
- Public enums are `#[non_exhaustive]` and carry `name()` + `Display` labels.
- `no_std`, no allocation.

## Related crates

Depends only on `broadcast-common`. Timecode inside ANC packets would sit on
top of `st291`.

## Minimum Supported Rust Version

1.95.0

## License

MIT OR Apache-2.0
