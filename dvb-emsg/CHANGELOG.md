# Changelog

All notable changes to `dvb-emsg` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- `EmsgVersion` and `PresentationTime` are now `#[non_exhaustive]` — forward-compat signal for downstream matchers if new spec versions are ever defined.
- `PresentationTime` now implements `name()` + `Display` via `dvb_common::impl_spec_display!` (issue #204 label convention): `Delta` renders as `"presentation_time_delta(0x…)"` and `Absolute` as `"presentation_time(0x…)"`.
- `examples/parse_emsg.rs`: guard `message_data[0]` access with `.first()` to avoid a panic when `is_scte35()` is true but `message_data` is empty (legal per spec).
- Added `tests/label_coverage.rs` drift-guard: fails CI if any public spec/field enum in `dvb-emsg` lacks a `Display` impl (SKIP: `Error`).

## [0.1.0]

### Added

- `EmsgBox` — parser+serializer for the MPEG-DASH Event Message Box (`emsg`):
  the `'emsg'` ISOBMFF `FullBox` header (`size` u32 / `'emsg'` / `version` u8 /
  `flags` u24) plus both version bodies — the two null-terminated UTF-8 strings
  (`scheme_id_uri`, `value`), the integer fields (`timescale`, `event_duration`,
  `id`), the version-discriminated presentation-time field, and the opaque
  `message_data[]` (box-size-derived length). `size` is **recomputed** and
  `version` is **derived** from the typed fields on serialize (no raw
  passthrough); `parse` validates the box type, size, and version.
- `PresentationTime` — the version-discriminated timing field:
  `Delta(u32)` (version 0, segment-relative `presentation_time_delta`) vs
  `Absolute(u64)` (version 1, representation-relative `presentation_time`).
  Selecting a variant selects the box version. The **v0/v1 field ordering
  differs** (strings-first in v0; integers-first / strings-last in v1) and both
  orderings are handled.
- `EmsgVersion` — the `version` byte (`SegmentRelative` 0 / `RepresentationRelative`
  1) with `name()` + `Display`. Constants `VERSION_0`, `VERSION_1`.
- `EmsgBox::is_scte35` + `SCTE35_SCHEME_PREFIX` (`urn:scte:scte35`) — recognises
  the SCTE 35 scheme, in which case `message_data` carries a SCTE 35
  `splice_info_section`.
- Constants `EMSG_BOX_TYPE` (`b"emsg"`), `FULLBOX_HEADER_LEN` (12), `EMSG_FLAGS`
  (0), `STRING_TERMINATOR` (0x00).
- A committed fixture `tests/fixtures/scte35_emsg_v0.bin` (a v0 `emsg` carrying
  the canonical SCTE 35 `splice_insert()` example as `message_data`), exercised
  by `tests/fixture_scte35.rs` (decoded fields + byte-exact round-trip + size
  recompute + message_data mutation bite).
- Two runnable examples: `build_emsg` (construct + serialize a v0 and a v1 box,
  showing the field-order difference) and `parse_emsg` (parse the SCTE 35
  fixture + byte-exact round-trip).
- `#![no_std]` + `alloc`; builds with `--no-default-features`.
- `serde` support behind the `serde` feature.

### Source footing

- The `emsg` **field semantics/types** are render-verified from the **free**
  DASH-IF IOP Part 10 V5.0.0 §6.1 / Table 6-2 (`docs/emsg.md`). The normative
  ISOBMFF box syntax (`FullBox('emsg', …)` ordering, the `version`-gated branch,
  the null-terminated-string layout) lives in **ISO/IEC 23009-1 §5.10.3.3**
  (**paid, not vendored**). The box layout is implemented from the well-known
  public `emsg` structure + DASH-IF semantics, with ISO 23009-1 cited as the
  formal source — **softer footing** than the fully-free crates, flagged per
  project policy.
