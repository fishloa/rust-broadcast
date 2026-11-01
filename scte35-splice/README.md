# scte35-splice

[![crates.io](https://img.shields.io/crates/v/scte35-splice.svg)](https://crates.io/crates/scte35-splice)
[![docs.rs](https://img.shields.io/docsrs/scte35-splice)](https://docs.rs/scte35-splice)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../LICENSE-MIT)

Spec-cited **ANSI/SCTE 35 2023r1** splice information (Digital Program Insertion
cueing) parser **and builder**, with the rust-broadcast family's symmetric
`Parse`/`Serialize` round-trip discipline: every wire type round-trips byte for byte.

> **Formerly `dvb-scte35`.** Renamed from `dvb-scte35` (last released as 7.9.0 in the
> DVB lockstep) to `scte35-splice` (v1.0.0) and now independently versioned. The
> deprecated `dvb-scte35` crate re-exports this one.

```toml
[dependencies]
scte35-splice = "4"
```

## What it does / what it does not

SCTE 35 is the cueing standard used across cable and OTT to signal ad-insertion
(avail) and content-segmentation opportunities in an MPEG transport stream. A
`splice_info_section` (table_id `0xFC`) carries one splice **command**
(`splice_insert`, `time_signal`, ...) plus a loop of splice **descriptors**
(`segmentation_descriptor`, ...), trailed by an MPEG CRC-32.

**Does:** parse and build `splice_info_section`s byte-exactly, with typed commands,
typed splice descriptors, the segmentation UPID / type-id tables, decoded 90 kHz time
accessors, and the DVB Targeted Advertising (DVB-TA) SCTE 35 profile (compact forms,
`DVB_DAS_descriptor`, DSM-CC stream-event payload). Used by ad-insertion (SSAI),
playout and monitoring code.

**Does not:** transport anything (no TS demux or PID discovery: feed it the section
bytes, e.g. from `dvb-si`), decrypt encrypted sections (the encrypted region is kept
raw and round-trips), make ad decisions, implement SCTE 104 (see `scte104`), or
implement the SCTE 35-2 XML/JSON mappings.

## Standards supported

| Standard | Edition | Scope / clauses | Status | Notes |
|---|---|---|---|---|
| ANSI/SCTE 35 Digital Program Insertion Cueing Message | 2023r1 (checked against `private/specs-md/ansi_scte_35_2023r1_dpi_cueing.md`) | §9.6 `splice_info_section`; §9.6.1 Table 7 commands; §10.1 Table 16 splice descriptors; §10.3.3 segmentation tables 21-25 | Full for clear sections; encrypted payload Typed-opaque | The encrypted region and `E_CRC_32` are kept raw. SCTE has since split the standard into 35-1 / 35-2; the binary syntax is unchanged. |
| ETSI TS 103 752-1 DVB Targeted Advertising, part 1 | V1.2.1 (checked against `etsi_ts_103_752-1_v01.02.01_dvb_ta`) | §5.3.5.16 `DVB_DAS_descriptor` (tag `0xF0`); §8.3.3 compact `time_signal` / `splice_insert`; §6.3.1 `DSM-CC_stream_event_payload_binary`; §5.3.4-5.3.5 PPO/DPO profiling | Full, with one open ambiguity | See the nibble-order note below. |
| ISO/IEC 13818-1 / ITU-T H.222.0 CRC-32 | n/a | section `CRC_32` (`broadcast_common::crc32_mpeg2`) | Full | Verified on parse, recomputed on serialize. |

### Command and descriptor coverage matrix

| SCTE 35 item | Code | Status |
|---|---|---|
| `splice_null` 0x00, `splice_schedule` 0x04, `splice_insert` 0x05, `time_signal` 0x06, `bandwidth_reservation` 0x07, `private_command` 0xFF | `commands::*` (6 types, `declare_commands!`) | Full |
| Any other command type | `AnyCommand::Unknown` | Typed-opaque (raw bytes kept) |
| `avail_descriptor` 0x00, `DTMF_descriptor` 0x01, `segmentation_descriptor` 0x02, `time_descriptor` 0x03, `audio_descriptor` 0x04 | `descriptors::*` (5 types, `declare_splice_descriptors!`) | Full |
| Segmentation UPID MPU (Table 24) and MID (Table 25) | `descriptors::Mpu`, `descriptors::MidUpid` | Full (decoded on demand) |
| Other UPID payloads | `SegmentationDescriptor` | Typed-opaque (typed `SegmentationUpidType`, payload bytes) |
| Unknown / private descriptor tags | `AnySpliceDescriptor::Unknown` | Typed-opaque |
| `DVB_DAS_descriptor` tag `0xF0` | `dvb_ta::DvbDasDescriptor` | Full |
| DVB-TA compact `time_signal` / `splice_insert` | `dvb_ta::CompactTimeSignal` / `CompactSpliceInsert` | Full |

**Open question, not guessed (DasNibbleOrder).** ETSI TS 103 752-1 Table 1
(stand-alone `DVB_DAS_descriptor()`, `equivalent_segmentation_type` in the low
nibble) and Table 8 (`compact_splice_insert()`, type in the high nibble) disagree on
the nibble order of the same body, and no corrigendum, byte example or independent
implementation resolves it. `dvb_ta::DasNibbleOrder` (`TypeHigh` = Table 8,
`TypeLow` = Table 1) with `parse_with_order` / `serialize_into_with_order` on
`DvbDasDescriptor` and `CompactSpliceInsert` selects either reading. Default wire
behaviour is Table 1 for the descriptor and Table 8 for the compact form; both are
pinned by `tests/it/das_nibble_order.rs`. Independent-oracle compact vectors:
`tests/it/dvb_ta_compact_vectors.rs`. Evidence: `docs/dvb_ta/compact-scte35.md`.

## Feature flags

| Feature | Default | Description |
|---------|---------|-------------|
| `std` | on | Link the standard library. Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | on | Serialize-only (`serde::Serialize`) on all types; no Deserialize. |

## Quick start

```rust
use scte35_splice::{SpliceInfoSection, commands::AnyCommand};
use broadcast_common::{Parse, Serialize};

// A minimal time_signal() section with no descriptors, built and emitted.
let ts = scte35_splice::commands::TimeSignal {
    splice_time: scte35_splice::time::SpliceTime::with_pts(0x0_0012_3456),
};
let section = SpliceInfoSection::new_clear(AnyCommand::TimeSignal(ts), &[]);
let bytes = section.try_to_bytes().unwrap();
assert_eq!(bytes[0], 0xFC); // table_id

// ...and parsed straight back.
let parsed = SpliceInfoSection::parse(&bytes).unwrap();
assert!(matches!(parsed.clear.as_ref().unwrap().command, AnyCommand::TimeSignal(_)));
```

## What is implemented

- **`splice_info_section`** (§9.6): `table_id`, `section_length`, `protocol_version`,
  `encrypted_packet` + `encryption_algorithm` (encrypted region kept raw), 33-bit
  `pts_adjustment` (`pts_adjustment_duration()` accessor), `cw_index`, 12-bit `tier`,
  `splice_command_length` / type, descriptor loop, CRC-32 (verified on parse).
- **Segmentation tables** (§10.3.3.1): typed `DeviceRestrictions` (Table 21),
  `SegmentationUpidType` (Table 22) and `SegmentationTypeId` (Table 23), each with a
  `Reserved(u8)` catch-all so unrecognised values round-trip.
- **Time accessors**: `SpliceTime` and `BreakDuration` expose `pts()` / `duration()`
  as `core::time::Duration`; `time::ticks_to_duration` saturates rather than wraps.
- **Robustness**: a clear section whose `splice_command_length` is the deprecated
  `0xFFF` sentinel (SCTE 35 §9.6.1) is parsed by deriving the command extent from its
  structure (never guessed; an underivable layout is an error);
  `ClearPayload::legacy_command_length_sentinel` records it so it re-serializes
  byte-exactly (found with a real TSDuck capture, `tests/it/real_capture.rs`).
  Serializers reject `sap_type` > 3, `encryption_algorithm` > 63, audio
  `bit_stream_mode` > 7 and `num_channels` > 15 with `Error::FieldOverflow`.
- **DVB-TA** (`dvb_ta`): `DVB_DAS_descriptor`, compact SCTE 35 (Tables 6 and 8 as
  re-aligned from the PDF, typed preserved `reserved` fields), stream-event payload
  with an RFC 4648 base-64 helper, PPO/DPO profiling helpers.
- **Dispatch**: `AnyCommand` and `AnySpliceDescriptor` are generated from single
  `declare_*!` lists with compile-time drift tests; adding a type is one line.

## Design notes

- Zero-copy `Parse<'a>` / `Serialize`; parse, serialize, byte-identical round trip is a
  hard invariant, exercised on real captures (`fixtures/ts/scte35-*.ts`).
- Reserved bits are written as `1` on serialize and ignored on parse.
- `Error` is `#[non_exhaustive]`; the crate is `no_std` + `alloc` without `std`.
- `serde` output is Serialize-only (no Deserialize).

## Related crates

Depends on `broadcast-common` (traits, CRC-32, `clock33`). Built on top of it:
`timed-metadata` (SCTE 35 / HLS DATERANGE / emsg), `ssai-runtime`, `playout-runtime`.
`dvb-si` locates the SCTE 35 PID through the PMT `"CUEI"` registration descriptor;
`scte104` is the automation-side signalling that feeds a SCTE 35 encoder.

## Spec grounding and examples

Syntax and assignment tables are transcribed in [`docs/README.md`](docs/README.md);
every module doc cites the SCTE 35 section, table and tag. Examples
(`cargo run -p scte35-splice --example <name>`): `parse_splice_insert`,
`round_trip_and_descriptors`.

## MSRV and licence

Rust **1.95.0**. Licensed under MIT OR Apache-2.0.
