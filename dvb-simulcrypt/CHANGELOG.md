# Changelog

All notable changes to `dvb-simulcrypt` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [0.2.1] — 2026-06-29

### Changed
- Dependency `broadcast-common` bump (renamed from `dvb-common`); no API change.

## [0.2.0] — 2026-06-22

### Added

- C(P)SIG ⇔ (P)SIG interface (TS 103 197 clause 8): `Interface::CpSigPSig`,
  `CpSigMessageType` (22 channel/stream/trigger/table/descriptor/PID-provision
  message types `0x0301`–`0x0321`), and `CpSigParameterType` (46 parameter
  types from Table 36, `0x000D`/`0x0100`–`0x012C`/`0x7000`–`0x7001`), all in
  `registry.rs`. Wired into the unified `MessageType`/`ParameterType` dispatch
  enums. Round-trip drift and message-round-trip tests in `tests/framing.rs`.

## [0.1.0] — 2026-06-21

### Fixed

- Added `tests/label_coverage.rs` drift-guard (issue #204 convention): scans
  every `pub enum` in `src/` and fails if any lacks a `Display` / `name()` impl.
  SKIP = `["Error"]`. Enforces the convention for all 11 public spec enums
  (`Interface`, `EcmgScsMessageType`, `EmmgMuxMessageType`, `MessageType`,
  `EcmgScsParameterType`, `EmmgMuxParameterType`, `ParameterType`, `DataType`,
  `SectionTspktFlag`, `EcmgErrorStatus`, `EmmgErrorStatus`).
- Expanded `error_status_values` test in `tests/framing.rs` into exhaustive
  loop-tables (`ecmg_error_status_values` / `emmg_error_status_values`) covering
  all 23 `EcmgErrorStatus` and 22 `EmmgErrorStatus` named variants with
  `to_u16`/`from_u16` round-trips and `Reserved(v)` pass-through assertions.
- Added `emmg_message_type_reserved_passthrough` test for `EmmgMuxMessageType`
  (mirrors the existing `EcmgScsMessageType` reserved pass-through check).
- Guarded `cpcw.value[0..2]` in `examples/parse_cw_provision.rs` against short
  values — uses `get(0..2)` and prints a graceful message instead of panicking.
- Added `#[cfg(feature = "serde")]` `serde_cw_provision_smoke` test serialising
  a `CW_provision` message to JSON and asserting the expected field names
  (`message_type`, `parameters`, `CwProvision`) are present; puts the existing
  `serde_json` dev-dependency to use.


### Added

- `SimulcryptMessage` — parser+serializer for the DVB SimulCrypt generic
  message (ETSI TS 103 197 §4.4.1, Table 1b): a 5-byte header
  (`protocol_version` + `message_type` + `message_length`, big-endian) followed
  by an ordered list of TLV `Parameter`s (`parameter_type` + `parameter_length`
  + value). `message_length` and every `parameter_length` are **recomputed on
  serialize** from the typed fields (no raw passthrough). `parse_on` validates
  the header, the `message_length` bound, and each `parameter_length`.
- `Parameter` — one TLV: a typed `ParameterType` + a borrowed, **opaque**
  `parameter_value` (CW/ECM/EMM/datagram bytes are carried, never interpreted).
- `Interface` — scopes the interface-dependent `message_type`/`parameter_type`
  numeric spaces (the interface is fixed by the TCP connection, supplied as a
  hint to `parse_on`). `EcmgScs` and `EmmgPdgMux` are modelled.
- ECMG⇔SCS (clause 5): `EcmgScsMessageType` (channel/stream
  setup/test/status/close/error, `CW_provision` `0x0201`, `ECM_response`
  `0x0202`), the Table 5 `EcmgScsParameterType` registry (`Super_CAS_id`
  `0x0001` … `ECM_id` `0x0019`, `error_status` `0x7000`, `error_information`
  `0x7001`), and the Table 6 `EcmgErrorStatus` codes.
- EMMG/PDG⇔MUX (clause 6): `EmmgMuxMessageType` (channel/stream messages,
  `stream_BW_request`/`allocation`, `data_provision` `0x0211`), the Table 7
  `EmmgMuxParameterType` registry, the Table 8 `EmmgErrorStatus` codes, and the
  `DataType` (§6.2.3) and `SectionTspktFlag` value tables.
- Interface-tagged `MessageType` / `ParameterType` enums + per-interface
  `name()` + `impl_spec_display!` labels on every public spec enum.
- A hand-built `CW_provision` fixture (`tests/fixtures/cw_provision.bin`,
  TS 103 197 §5.5.7) with an integration test parsing its fields and byte-exact
  round-tripping it; the CW inside `CP_CW_combination` stays opaque.
- Two runnable examples: `build_channel_setup` (construct + serialize from
  typed fields) and `parse_cw_provision` (read + walk + round-trip the fixture).
- `#![no_std]` + `alloc`; builds with `--no-default-features`.
- `serde` support behind the `serde` feature.
