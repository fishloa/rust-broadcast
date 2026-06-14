# dvb-scte35

[![crates.io](https://img.shields.io/crates/v/dvb-scte35.svg)](https://crates.io/crates/dvb-scte35)
[![docs.rs](https://img.shields.io/docsrs/dvb-scte35)](https://docs.rs/dvb-scte35)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../LICENSE-MIT)

Spec-cited **ANSI/SCTE 35 2023r1** splice information (Digital Program
Insertion cueing) parser **and builder** in Rust, with the rust-dvb family's
symmetric `Parse`/`Serialize` round-trip discipline — every wire type
round-trips byte-for-byte.

> **Edition note.** This crate implements **ANSI/SCTE 35 2023r1**, the
> single-document edition of the standard. SCTE has since split the standard
> into **SCTE 35-1** (the message) and **SCTE 35-2** (XML/binary mappings); the
> binary `splice_info_section` syntax implemented here is unchanged.

## What is SCTE 35?

SCTE 35 is the cueing standard used across the cable / OTT industry to signal
ad-insertion (avail) and content-segmentation opportunities in an MPEG
transport stream. A `splice_info_section` (table_id `0xFC`) carries one splice
**command** (e.g. `splice_insert`, `time_signal`) plus a loop of splice
**descriptors** (e.g. `segmentation_descriptor`), trailed by an MPEG CRC-32.

## Quick start

```rust
use dvb_scte35::{SpliceInfoSection, commands::{AnyCommand, TimeSignal}};
use dvb_scte35::time::SpliceTime;
use dvb_common::{Parse, Serialize};

// Build a time_signal() section and emit it.
let ts = TimeSignal { splice_time: SpliceTime::with_pts(0x0_0012_3456) };
let section = SpliceInfoSection::new_clear(AnyCommand::TimeSignal(ts), &[]);
let bytes = section.to_bytes();
assert_eq!(bytes[0], 0xFC); // table_id

// ...and parse it straight back (CRC verified on parse).
let parsed = SpliceInfoSection::parse(&bytes).unwrap();
assert!(matches!(parsed.clear.unwrap().command, AnyCommand::TimeSignal(_)));
```

## What's implemented

### splice_info_section

Full §9.6 header: `table_id` (0xFC), `section_length`, `protocol_version`,
`encrypted_packet` flag + `encryption_algorithm` (encrypted region kept raw),
33-bit `pts_adjustment`, 12-bit `tier`, `splice_command_length`,
`splice_command_type`, descriptor_loop_length, CRC-32 (verified on parse).
Decoded accessor: `pts_adjustment_duration()` → `core::time::Duration`.

### Splice commands — 6 (§9.6.1 Table 7)

All 6 are in the `declare_commands!` list in `commands/any.rs`; each has `Parse`
+ `Serialize` impl and round-trip tests. Unknown/reserved command types fall
through to `AnyCommand::Unknown` with raw bytes preserved.

| Type | Command | Notes |
|------|---------|-------|
| 0x00 | `splice_null` | Empty; CRC-only keep-alive |
| 0x04 | `splice_schedule` | One or more timed splice events |
| 0x05 | `splice_insert` | Immediate or timed insert with `break_duration` |
| 0x06 | `time_signal` | `splice_time` with optional PTS |
| 0x07 | `bandwidth_reservation` | Empty payload |
| 0xFF | `private_command` | `identifier` (32-bit) + opaque body |

### Splice descriptors — 5 (§10.1 Table 16)

All 5 are in the `declare_splice_descriptors!` list in `descriptors/any.rs`;
unknown tags fall through to `AnySpliceDescriptor::Unknown` with raw body (lossless).

| Tag | Descriptor | Notes |
|-----|------------|-------|
| 0x00 | `avail_descriptor` | `provider_avail_id` |
| 0x01 | `DTMF_descriptor` | preroll + DTMF chars |
| 0x02 | `segmentation_descriptor` | `segmentation_upid`, typed `SegmentationTypeId`, `DeviceRestrictions`, `component_list` |
| 0x03 | `time_descriptor` | TAI seconds + nanoseconds + UTC offset |
| 0x04 | `audio_descriptor` | Multi-component audio coding info |

### Segmentation assignment tables

Typed enums so callers never re-implement the spec lookup tables (§10.3.3.1):

| Enum | Table | Values |
|------|-------|--------|
| `DeviceRestrictions` | Table 21 | 4 variants (2-bit field) |
| `SegmentationUpidType` | Table 22 | 18 named variants (0x00–0x11) + `Reserved(u8)` |
| `SegmentationTypeId` | Table 23 | 48 named variants (0x00–0x51) + `Reserved(u8)` |

All three enums round-trip via `from_*` / `to_u8`; unrecognised values are
carried through `Reserved(raw)` without data loss.

### Decoded time accessors

90 kHz `SpliceTime` and `BreakDuration` fields expose `pts()` / `duration()`
accessors returning `core::time::Duration`. The 33-bit `pts_adjustment` field
likewise has a `pts_adjustment_duration()` accessor (carry-ignored wrap per the
spec).

### Dispatch and drift tests

`AnyCommand` and `AnySpliceDescriptor` are each generated from a single
`declare_*!` macro invocation that is the single source of truth for the
dispatcher and a compile-time drift test. Adding a new command or descriptor
requires one line in the macro list.

## dvb-si integration

SCTE 35 sections ride on a PID the PMT labels with a registration descriptor
carrying the `"CUEI"` format_identifier — which [`dvb-si`](../dvb-si/) already
parses. Once you have the `0xFC` section bytes (e.g. from a dvb-si demux),
route them into `SpliceInfoSection::parse`.

## Feature flags

| Feature | Default | Description |
|---------|---------|-------------|
| `std` | **on** | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | **on** | Serialize-only (`serde::Serialize`) on all types; no Deserialize. |

## MSRV

Rust **1.81**.

## Spec grounding

The syntax tables and the assignment tables are hand-transcribed in
[`dvb-scte35/docs/scte_35.md`](docs/scte_35.md); every module doc cites the SCTE
35 section, table and tag/command_type it implements. SCTE 35 is published by
SCTE at no cost.

## License

Licensed under either of MIT or Apache-2.0 at your option.
