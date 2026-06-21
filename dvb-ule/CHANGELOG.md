# Changelog

All notable changes to `dvb-ule` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0]

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
