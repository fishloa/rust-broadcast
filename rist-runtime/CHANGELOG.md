# Changelog

All notable changes to this crate will be documented in this file.

## [Unreleased]

### Fixed

- `RistSenderCompound` and `RistReceiverCompound` now implement `Parse` (in
  addition to the `Serialize` they already had), so the "byte-exact
  `Parse`/`Serialize` round-trip fidelity for every wire type" claim in this
  file / `README.md` / `src/lib.rs` is now actually true instead of covering
  three of the five wire types (#938).
- The four `tests/round_trip.rs` tests that were named `*_round_trip` but only
  called `to_bytes()` (no parse-back) now actually parse the serialized bytes
  and assert byte-identical equality, plus one new test exercising every
  optional compound slot (multiple NACKs + Range NACKs + RTT Echo) together
  (#938).

### Added

- Initial release
- `GenericNack` — RFC 4585 §6.2.1 RTCP Transport-Layer Feedback (PT 205,
  FMT 1) bitmask-based retransmission request.
- `RangeNack` — RIST-specific RTCP APP (PT 204, subtype 0, name `"RIST"`)
  range-based retransmission request (VSF TR-06-1:2020 §5.3.2.2).
- `RttEcho` / `RttEchoKind` — RTCP APP (PT 204, name `"RIST"`, subtype 2/3)
  round-trip time measurement (VSF TR-06-1:2020 §5.2.6).
- `RistSenderCompound` / `RistReceiverCompound` — compound RTCP packet
  builders enforcing the RIST §5.2.1 structure.
- Byte-exact `Parse`/`Serialize` round-trip fidelity for every wire type,
  built on top of `rtcp-packet` (RFC 3550 §6 SR/RR/SDES/BYE/APP).
- `no_std` + `alloc` support (`std` feature on by default).
