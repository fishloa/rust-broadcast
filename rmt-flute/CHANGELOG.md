# Changelog

All notable changes to `rmt-flute` (formerly `dvb-flute`) will be documented
in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0] - 2026-08-08

### Changed
- **Renamed `dvb-flute` → `rmt-flute`.** No code, API or behaviour change —
  this is a naming correction.

  The crate implements only **IETF RMT** standards: LCT (RFC 5651), ALC
  (RFC 5775), FLUTE (RFC 6726) and NORM (RFC 5740). It implements **no DVB
  standard**; its own description and keywords never claimed one — only the
  crate name did. DVB is one of several *consumers* of these formats,
  alongside 3GPP MBMS/eMBMS and ATSC 3.0 ROUTE.

  The name became actively misleading with ATSC 3.0 ROUTE work planned:
  A/331 Annex A is written as a profile-and-delta on RFC 5651/5775/6726, so
  an `atsc3-route` crate would have depended on a crate named `dvb-*` for
  something that is neither ATSC nor DVB.

  Version continues the existing line rather than restarting at 0.1.0 — the
  code is five releases in, audited and fuzzed, and a fresh `0.1.0` would
  misrepresent its maturity. This follows the workspace's own rename
  precedents (`smpte2038` → `st291` continued at 0.2.0; `ll-hls-runtime` →
  `hls-runtime` continued at 0.4.0). The minor bump reflects that a rename is
  breaking for consumers, and for a 0.x crate minor is the breaking axis.

  **All `dvb-flute` versions (0.1.0, 0.1.1, 0.2.0, 0.3.0, 0.3.1) are yanked.**
  No compatibility shim is published — the same approach taken for
  `smpte2038`/`dvb-smpte2038`. There were zero reverse dependencies on
  crates.io at the time of the rename.

### Fixed
- Crate-root docs and README reframed to lead with IETF RMT and to name DVB,
  3GPP MBMS and ATSC 3.0 ROUTE as consumers rather than owners.

## [0.3.1] - 2026-07-30

### Added
- `tests/non_exhaustive_coverage.rs` drift guard (issue #806). No public API
  or behaviour change.

## [0.3.0] - 2026-07-29

### Changed (BREAKING)
- **Requires `broadcast-common` 9** (issue #819). No functional or API change of
  this crate's own.

  Staying on `broadcast-common` 8 was not neutral: this crate's types implement
  `Parse`/`Serialize` from whichever major it links, so a consumer that used it
  alongside a 9-based crate (`transmux` 0.20, `dvb-si` 9, …) got **both majors
  in one graph**, and the trait methods resolved against the wrong one —
  surfacing as `no method named to_bytes found` / `no function named parse
  found` on types that plainly have them, with the compiler pointing at
  `broadcast-common-8.x/src/traits.rs`.

  The 9.0.0 wave originally shipped only the crates needed to publish
  `transmux`/`media-plane`/`multimux`, on the reasoning that everything else
  stayed coherent on its own 8 line. That reasoning was wrong: these crates
  exist to be composed, and the breakage only appears in a consumer that mixes
  them.

## [0.2.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

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
