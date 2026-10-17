# broadcast-common

[![crates.io](https://img.shields.io/crates/v/broadcast-common.svg)](https://crates.io/crates/broadcast-common)
[![docs.rs](https://img.shields.io/docsrs/broadcast-common)](https://docs.rs/broadcast-common)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#related-crates-and-license)

**The foundation crate of the rust-broadcast workspace: the symmetric `Parse`/`Serialize` traits, CRC-32/MPEG-2, BCD and MJD time codecs, bit-field I/O, the 33-bit/PCR clock arithmetic, checked field-width helpers, and the container-mux and streaming-stage traits every sibling crate builds on.**

## What it does / what it does not

Nearly every crate in the workspace depends on `broadcast-common`, so it holds
only the code that must be defined exactly once: the traits that make every
wire type parse/serialize symmetric, the small codecs the DVB/MPEG family
shares (CRC, BCD, MJD, odd parity, hex), the clock arithmetic that PTS/DTS,
PCR, SCTE-35 and SRT all need, the range-checking helpers serializers use
instead of silent masking, and the vocabulary traits of the muxing hub
(`Unpackage`/`Package`/`Encrypt`/`Decrypt`) and the incremental `Stage` driver.
Use it directly when you implement a new wire type in the workspace's style or
need one of those primitives.

It deliberately has no shared error enum (each crate owns a domain `Error`,
reached through `type Error` on the traits), no CRC-8 (only `dvb-bbframe` needs
it), no concrete media or codec types, and no heavy dependencies (`chrono`,
`serde` are optional; `hex` is the only unconditional one).

## Standards supported

This crate implements primitives rather than a wire format, so the table lists
the specification each primitive follows.

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ITU-T H.222.0 / ISO/IEC 13818-1 | H.222.0 v9 (08/2023), verified against the vendored copy | Annex A CRC decoder model, the CRC-32 (`crc32_mpeg2`); §2.4.3.2 PUSI unit delimiting (`pusi`); §2.4.3.3 legal duplicate packets (`ts_dup`); §2.4.3.4/§2.4.3.7 PCR (27 MHz) and 33-bit PTS/DTS clocks and the 5-byte marker-bit timestamp field (`clock33`) | Full for the primitives listed | Higher layers (TS, PES, PSI) live in `mpeg-ts`, `mpeg-pes`, `dvb-si`. |
| ETSI EN 300 468 | V1.20.1 (vendored; the 2038 MJD wrap rule is from this edition) | Annex A CRC-32 (same polynomial as above); Annex C MJD + BCD UTC time and BCD durations (`time`, `bcd`) | Full for the codecs | MJD range 1948-08-05 ..= 2128-01-09. Integer proleptic-Gregorian arithmetic (no closed-form Annex C formula limit). |
| ETSI TS 102 773 | V1.4.1 | Annex A CRC-32 for T2-MI packets (same `crc32_mpeg2`) | Full for the CRC | The packet format is in `dvb-t2mi`. |
| ETSI EN 300 706 | V1.2.1 | §8.1 odd-parity protected byte (`parity`) | Partial: parity primitive only | Also used for CEA-608 line-21 bytes. Detect-only by design. |
| ISO/IEC 23001-7 (CENC) | not pinned (not vendored; unverified) | §4 `schm.scheme_type` identity: `cenc`, `cbcs` (`cenc::CencScheme`) | Scheme identity only | Crypto boxes live in `transmux`; `#EXT-X-KEY` signalling in `broadcast-hls`. |
| RFC 1982 serial-number arithmetic | n/a | `seq::SeqSpace` wrap-safe sequence comparison (RTP 16-bit, SRT 31-bit) | Comparable, not spec-cited | Implementation policy, not defined by RTP/SRT. |

## Feature flags

| Feature | Default | What it enables |
|---|---|---|
| `std` | yes | Links `std`: `std::error::Error` glue, `Timestamp::from_instant`, and `chrono`'s clock when `chrono` is on. Off gives `#![no_std]` + `alloc`. |
| `chrono` | no | `chrono::DateTime<Utc>` conversions in `time` (`decode_mjd_bcd_utc`, `encode_mjd_bcd_utc`, ...). Without it the plain `MjdBcdDateTime` and duration helpers stay available. |
| `serde` | no | `serde::Serialize` on the shared spec types (currently `cenc::CencScheme`). |

```toml
[dependencies]
broadcast-common = "10"
# embedded (no_std + alloc):
# broadcast-common = { version = "9", default-features = false }
```

## Quick start

```rust
use broadcast_common::{bcd, clock33, crc32_mpeg2, Parse, Serialize};

// CRC-32/MPEG-2 over a real PAT section (from the committed m6-single.ts fixture):
// the CRC of the body equals the trailing CRC_32 field.
let section = [
    0x00, 0xB0, 0x0D, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x04, 0x01, 0xE0, 0x64,
    0xF9, 0xB4, 0x63, 0xEF,
];
let crc = crc32_mpeg2::compute(&section[..section.len() - 4]);
assert_eq!(crc, 0xF9B4_63EF);

// BCD and 33-bit clock arithmetic.
assert_eq!(bcd::from_bcd_byte(0x42), Some(42));
assert_eq!(clock33::add(clock33::WRAP_33BIT - 1, 2), 1);
assert_eq!(clock33::signed_distance(clock33::WRAP_33BIT - 10, 5), 15);

// Implement the symmetric contract for your own wire type.
#[derive(Debug, PartialEq)]
struct Toy { tag: u8, value: u16 }

impl<'a> Parse<'a> for Toy {
    type Error = &'static str;
    fn parse(b: &'a [u8]) -> Result<Self, Self::Error> {
        if b.len() < 3 { return Err("short"); }
        Ok(Toy { tag: b[0], value: u16::from_be_bytes([b[1], b[2]]) })
    }
}
impl Serialize for Toy {
    type Error = &'static str;
    fn serialized_len(&self) -> usize { 3 }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if buf.len() < 3 { return Err("short"); }
        buf[0] = self.tag;
        buf[1..3].copy_from_slice(&self.value.to_be_bytes());
        Ok(3)
    }
}
let wire = [0x42, 0x12, 0x34];
let toy = Toy::parse(&wire)?;
assert_eq!(toy.try_to_bytes()?, wire); // the workspace round-trip invariant
# Ok::<(), &'static str>(())
```

Runnable examples: `cargo run -p broadcast-common --example crc_and_bcd` and
`--example implement_parse_serialize`.

## What is implemented

| Module | Public API |
|---|---|
| `traits` | `Parse<'a>` (`parse`), `Serialize` (`serialized_len`, `serialize_into`, `try_to_bytes`). `to_bytes()` no longer exists: it panicked on failure; `try_to_bytes` is the only allocate-and-serialize entry point. |
| `crc32_mpeg2` | `compute` (poly `0x04C1_1DB7`, init `0xFFFF_FFFF`, MSB-first, no reflection, no final XOR; table built at compile time) |
| `bcd` | `from_bcd_byte`, `to_bcd_byte` and wider helpers; `None` on out-of-range input, never garbage |
| `time` | `MjdBcdDateTime`, `decode_mjd_bcd`/`encode_mjd_bcd`, `decode_bcd_duration`/`encode_bcd_duration`, `mjd_to_ymd`/`ymd_to_mjd`, `decode_seconds_since_2000_utc`/`encode_seconds_since_2000_utc`; with `chrono`: `decode_mjd_bcd_utc`/`encode_mjd_bcd_utc`. The encoder rejects impossible days (31 Feb) |
| `bits` | `BitReader`, `BitWriter` (MSB-first, up to 64 bits per field, spans bytes), `BitError` |
| `clock33` | `WRAP_33BIT`, `unwrap_delta`, `wrapping_forward_distance`, `add`, `add_signed`, `signed_distance`; PCR: `PTS_HZ`, `PCR_HZ`, `PCR_EXT_MODULUS`, `PCR_WRAP_27MHZ`, `pcr_from_parts`, `pcr_split`, `pcr_forward_distance`, `pcr_signed_distance`; marker-bit timestamp field: `Marked`, `MARKERS_ALL_SET`, `unpack_marked`, `pack_marked`, `pack_marked_with` |
| `len` | `FieldOverflow`, `fit_bits`, `fit_mask`, `fit_u8`, `fit_u16`, `fit_u24`, `fit_u32`: checked width narrowing for serializers |
| `pusi` | `PusiAccumulator` (size-capped, `DEFAULT_MAX_UNIT_SIZE` 16 MiB): the unit splitter wrapped by `mpeg-ts` and `mpeg-pes` |
| `ts_dup` | `is_legal_duplicate_pair`, `check_duplicate`, `DuplicateVerdict` (`NotDuplicate`, `Legal`, `IllegalThirdRepeat`): one reading of H.222.0 §2.4.3.3 shared by `dvb-conformance`, `media-doctor` and `ts-fix` |
| `parity` | `has_odd_parity`, `set_odd_parity`, `strip_parity` |
| `hex` | `hex_encode` (decoders stay with callers, whose validation policies differ) |
| `seq` | `SeqSpace` (`add`, `diff`, `lt`/`leq`/`gt`/`geq`, `in_closed_range`), `MAX_SEQ_BITS` |
| `cenc` | `CencScheme` (`to_four_cc`, `from_four_cc`, `name`), `SCHEME_CENC`, `SCHEME_CBCS` |
| `mux` | `Unpackage`, `Package` (batch container demux/mux), `Decrypt`, `Encrypt` (in-place sample protection); inverse pairs, generic over `Input`/`Output`/`Media`/`Config`/`Keys`/`Error` |
| `stage` | `Stage` (`feed`/`poll`/`finish`, `next_deadline`/`on_deadline`, `demand`), `Timestamp` (monotonic nanoseconds, saturating arithmetic), `Demand` |
| root | `impl_spec_display!` macro (generates `Display` from an inherent `name()`) |

## Design notes

- **Reserved-bit and trailing-byte policy** (workspace-wide, issue #1139):
  parse is tolerant; serialize preserves what was parsed in typed, spec-named
  fields (no raw passthrough blobs); hand-built values default to the edition
  value (`1` for `reserved_future_use`, `0` for `reserved_zero_future_use`);
  reserved bytes are always written explicitly; trailing bytes are never
  dropped without a documented decision.
- **Round-trip invariant.** Every wire type parses to a borrowed struct and
  serializes back byte-identically; this crate supplies the traits, the helpers
  and the rule, and each crate carries the round-trip tests.
- **Checked widths, not masks.** `len::fit_*` returns `FieldOverflow` when a
  value does not fit its field; the `& MASK` idiom is banned in serializers.
- **One clock implementation.** PCR/PTS/DTS/`pts_time` wrap arithmetic lives in
  `clock33`; the 2^33 modulus must not be re-derived elsewhere.
- **Label convention.** `impl_spec_display!` keeps `Display` in lockstep with
  the hand-written `name()` of every public spec enum.
- `#![forbid(unsafe_code)]`, `#![warn(missing_docs)]`, `no_std` + `alloc`
  without `std`.

## Related crates and license

Versioned independently of the DVB lockstep crates (it ships from its own
`broadcast-common-v*` tag). Every protocol crate depends on it; it depends on
none of them. See the [workspace README](https://github.com/fishloa/rust-broadcast#readme)
for the crate map.

MSRV 1.95.0. Licensed under MIT OR Apache-2.0, at your option.
