# Changelog

All notable changes to `ule` will be documented in this file.
This crate was previously published as `dvb-ule` (0.1.0). The rename
reflects that ULE (RFC 4326) is an IETF standard, not a DVB standard.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1] — 2026-06-29

### Changed
- Dependency `broadcast-common` bump (renamed from `dvb-common`); no API change.

## [0.1.0] — 2026-06-21

### Fixed

- **Reassembly PP consistency check (RFC 4326 §7.2.1):** `UleReceiver` now
  validates the Payload Pointer on a PUSI=1 packet arriving mid-reassembly. If
  `PP ≠ remaining_bytes` the partial SNDU is discarded per §7.2.1 instead of
  being silently contaminated with bytes from the wrong offset. A regression
  test (`pusi_mid_reassembly_wrong_pp_discards_partial`) verifies this bites
  against the pre-fix code and passes after the fix.
- **`Sndu::type_field` field removed:** the former `pub type_field: TypeField`
  field was ignored by `serialize_into` (which always derived the type from the
  `payload` chain), meaning a manually-constructed `Sndu` with a divergent
  `type_field` serialized incorrectly. The field is replaced by a
  `type_field()` accessor method that delegates to `payload.base_type()`,
  eliminating the divergence entirely.

### Added

- `MandatoryHType` and `OptionalHType` enums — typed H-Type values for the two
  separate IANA H-Type registries (RFC 4326 §5 / RFC 5163 §3). Both carry
  `name()` + `impl_spec_display!` and `#[non_exhaustive]`, and are accessible
  via `ExtensionHeader::mandatory_h_type()` / `ExtensionHeader::optional_h_type()`.
- `tests/label_coverage.rs` drift-guard: fails CI if any `pub enum` in `src/`
  is missing a `Display` impl (issue #204 convention).

### Changed

- `TypeField` and `ExtensionHeader` are now `#[non_exhaustive]`.
- Private named constants `D_BIT_MASK` (`0x8000`) and `LENGTH_MASK` (`0x7FFF`)
  replace all raw hex literals for the D-bit and Length field masks across
  `sndu.rs` and `ts.rs`.
- `is_end_indicator` rewritten to use the existing `PADDING_BYTE` constant
  instead of bare `0xFF` literals.


### Added

- `Sndu` — parser+serializer for the ULE SubNetwork Data Unit (RFC 4326 §4,
  Figure 1): the `D` bit + 15-bit `Length` + 16-bit `Type`, an optional 6-byte
  Destination NPA address (present iff `D = 0`), the PDU, and the 4-byte CRC-32
  trailer. `Length` and the CRC are **recomputed on serialize** from the typed
  fields (no raw passthrough); `parse` re-validates the CRC and rejects a
  mismatch.
- `TypeField` — the RFC 4326 §4.4 Type-field interpretation, split at `0x0600`:
  a Next-Header (`H-LEN` 3 bits + `H-Type` 8 bits) below the boundary, an
  EtherType at or above. Constants `ETHERTYPE_IPV4` (`0x0800`), `ETHERTYPE_IPV6`
  (`0x86DD`), `ETHERTYPE_BOUNDARY` (`0x0600`).
- `ExtensionHeader` / `PayloadChain` — the chained extension-header model
  (RFC 4326 §5, RFC 5163 §3): Optional headers (`H-LEN = 1..=5`, total `2·H-LEN`
  bytes including the 2-byte Type field) and a terminating EtherType or
  Mandatory header. H-Type registry constants for Test-SNDU (`0x00`),
  Bridged-Frame (`0x01`), TS-Concat (`0x02`), PDU-Concat (`0x03`), TimeStamp
  (`0x01`/H-LEN 3), and Extension-Padding (`0x00`/H-LEN 1–5).
- `UleReceiver` — a de-fragmenting/reassembling depacketizer (RFC 4326 §6, §7):
  feed it each TS packet payload + PUSI flag; it handles the 1-byte Payload
  Pointer, SNDU fragmentation across packets, packing of multiple SNDUs per
  packet, and the End-Indicator (`0xFFFF`) / `0xFF` padding, yielding complete
  SNDU byte vectors. `TS_PAYLOAD_LEN` constant (`184`).
- CRC-32 reused from `dvb-common::crc32_mpeg2` — verified byte-exact against
  RFC 4326 Appendix B's worked example (an ICMPv6-over-IPv6 SNDU with CRC
  `0x7C171763`), committed as the `tests/fixtures/appendix_b.bin` fixture and
  exercised by `tests/fixture_appendix_b.rs` (decoded fields + byte-exact
  round-trip + CRC-match + corrupt-CRC rejection).
- Two runnable examples: `build_sndu` (construct + serialize from typed fields)
  and `receive_sndu` (fragment the Appendix B fixture across two TS packets and
  reassemble it).
- `#![no_std]` + `alloc`; builds with `--no-default-features`.
- `serde` support behind the `serde` feature.
