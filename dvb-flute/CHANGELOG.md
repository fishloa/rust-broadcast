# Changelog

All notable changes to `dvb-flute` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1] — 2026-06-29

### Changed
- Dependency `broadcast-common` bump (renamed from `dvb-common`); no API change.

## [0.1.0] — 2026-06-21

### Added

- `NormInfo` — NORM_INFO (type = 1) parser+serializer (RFC 5740 §4.2.2, Figure 8):
  common header + sender word + `flags | fec_id | object_transport_id` + optional
  header extensions + payload. Unlike NORM_DATA there is **no** `fec_payload_id`
  field; base `hdr_len` is 4 words (16 bytes). `NORM_INFO_FIXED_LEN` constant
  exported. Three round-trip tests: basic construct-from-fields + byte-exact
  check, mutated-field bite test, and EXT_FTI extension chain test.
- `tests/label_coverage.rs` — drift-guard for the `name()`+`Display` label
  convention (#204): scans `src/` for `pub enum`s and fails CI if any lacks a
  `Display` impl.

### Changed

- `lct.rs`: flag-bit literals `0x0002` (A = close_session) and `0x0001`
  (B = close_object) replaced with named private constants `FLAG_A` / `FLAG_B`.
- `lct.rs`: `h_flag()` corrected from `||` to `&&` — the RFC 5651 §5.1 H-bit
  constraint requires TSI **and** TOI to agree on half-word parity.
- `lct_ext.rs`: bare Use-field sub-masks `0x00FF` and `0x0F00` replaced with
  named private constants `USE_PI_SPECIFIC_MASK` / `USE_RESERVED_MASK`.


### Added

- `LctHeader` — parser+serializer for the Layered Coding Transport header
  (RFC 5651 §5): the fixed first word (`V`/`C`/`PSI`/`S`/`O`/`H`/`A`/`B`,
  `HDR_LEN`, Codepoint) plus the flag-driven variable fields **CCI**
  (`4*(C+1)` bytes), **TSI** (`4*S+2*H` bytes) and **TOI** (`4*O+2*H` bytes).
  The shared `H` half-word feeds both TSI and TOI; the `C`/`S`/`O`/`H` flag bits
  and `HDR_LEN` are recomputed on serialize from the typed field lengths (no raw
  passthrough). Mismatched-`H` and out-of-range widths are rejected.
- `HeaderExtension` + `parse_chain`/`serialize_chain` — the LCT/NORM
  header-extension chain (RFC 5651 §5.2 / RFC 5740 §4.1): variable-length
  (`HET` 0..=127, carries `HEL`) and fixed-length (`HET` 128..=255, one 32-bit
  word) forms; `HEL` recomputed on serialize.
- `LctExtType` registry (EXT_NOP 0 / EXT_AUTH 1 / EXT_TIME 2) and the `ExtTime`
  typed EXT_TIME extension (RFC 5651 §5.2.2) with the SCT-High/SCT-Low/ERT/SLC
  `Use` bit field and ordered 32-bit time values.
- `AlcPacket` — an Asynchronous Layered Coding packet (RFC 5775 §4): LCT header
  + an opaque (FEC-scheme-dependent, caller-sized) FEC Payload ID + the
  encoding-symbol payload, with the SPI PSI bit and `EXT_FTI` (HET 64).
  Data-less control packets (LCT header only) round-trip with empty
  `fec_payload_id`/`payload`.
- `FecPayloadId128` — the Small-Block-Systematic (`fec_id` 128/129) FEC Payload
  ID (32-bit source_block_number + 16-bit source_block_length + 16-bit
  encoding_symbol_id), reproduced from RFC 5445 as one concrete layout.
- FLUTE (RFC 6726): `ExtFdt` (EXT_FDT, HET 192 — FLUTE version + 20-bit FDT
  Instance ID), `ExtCenc` (EXT_CENC, HET 193 — `CencAlgorithm`
  null/ZLIB/DEFLATE/GZIP), and the `TOI_FDT` = 0 FDT-Instance convention. The
  FDT Instance body is XML and is **out of scope** — exposed as the opaque
  packet payload.
- NORM (RFC 5740): `NormCommonHeader` (version/type/hdr_len/sequence/source_id),
  the `NormMessageType` registry (INFO/DATA/CMD/NACK/ACK/REPORT), `SenderWord`
  (instance_id/grtt/backoff/gsize), and the message types `NormData`,
  `NormCmd` (with the `NormCmdType` sub-type registry FLUSH/EOT/SQUELCH/CC/
  REPAIR_ADV/ACK_REQ/APPLICATION) and `NormFeedback` (NORM_NACK / NORM_ACK with
  the `NormAckType` registry). `hdr_len` recomputed on serialize; FEC Payload
  IDs and length-inferred trailing regions kept opaque.
- A committed FLUTE FDT-packet fixture (`tests/fixtures/flute_fdt.bin`, built to
  the RFC 6726 §3.4 shape: TOI = 0 + EXT_FDT + 8-byte FEC Payload ID + XML body)
  with a fixture test exercising the flag-driven LCT widths, EXT_FDT decode, and
  a byte-exact round-trip.
- Two runnable examples: `build_lct` (construct + serialize a FLUTE/ALC packet
  from typed fields) and `parse_flute` (parse the committed fixture).
- `#![no_std]` + `alloc`; builds with `--no-default-features`.
- `serde` support behind the `serde` feature.
