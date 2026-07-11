# Changelog

All notable changes to `rtp-packet` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `RtpPacket` — parser+serializer for the RFC 3550 §5.1 RTP fixed header:
  version (validated `== 2` on parse, always written `2` on serialize),
  padding (trailing pad-count octet stripped/re-appended, stored as the raw
  padding region so byte content is preserved exactly), the CSRC identifier
  list (0–15 entries, `CC` always derived from `csrc.len()` — never trusted as
  an independent field), marker, payload type (7 bits), sequence number,
  timestamp, SSRC, an optional `HeaderExtension`, and the payload.
- `HeaderExtension` — the RFC 3550 §5.3.1 generic header extension: a 16-bit
  profile-specific identifier + opaque profile-specific data (the `length`
  field, counted in 32-bit words, is always derived from the data length, and
  a zero-length extension is accepted per §5.3.1: "therefore zero is a valid
  length").
- `docs/rtp-header.md` — curated transcription of RFC 3550 §5.1/§5.3.1 (fetched
  directly from the RFC), the implementation/audit oracle for this crate.
- Real-fixture test (`tests/fixture_simple.rs`) over a committed 324-byte RTP
  packet (`tests/fixtures/rtp_simple.bin`) captured by running this
  workspace's own `transmux::RtpPacketizer` over the real broadcast capture
  `fixtures/ts/h264_aac.ts` — real, spec-compliant wire bytes, not hand-typed.
  Padding/CSRC/header-extension cases (not exercised by any real stream in
  this workspace today) use spec-Table-derived vectors instead
  (`tests/round_trip.rs`), documented as such.
- Two runnable examples: `build_packet` (construct + serialize from typed
  fields, including CSRC list + header extension) and `parse_packet` (parse
  the committed fixture at runtime + byte-exact round-trip).
- `#![no_std]` (via `#![cfg_attr(not(feature = "std"), no_std)]`) + `alloc`;
  builds standalone with `--no-default-features` and on a bare-metal
  (`thumbv7em-none-eabi`) target.
- `serde` support behind the `serde` feature.
- `tests/label_coverage.rs` — the workspace's issue #204 label-convention
  drift-guard (passes trivially today: this crate defines no spec/field
  enums, only structs).
