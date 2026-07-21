# Changelog

All notable changes to `rtp-packet` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-07-11

## [0.2.0] - 2026-07-11

### Added

- `rfc8285` feature (additive, off by default): decodes the RFC 8285
  one-byte/two-byte multiplexed extension elements a profile may pack into
  the RFC 3550 §5.3.1 `HeaderExtension`'s opaque `data`.
  - `parse_extensions(&HeaderExtension) -> Result<ExtensionElements>` —
    dispatches on `profile_id`: `0xBEDE` -> one-byte form, `profile_id &
    0xFFF0 == 0x1000` -> two-byte form, anything else ->
    `Error::NotRfc8285Extension` (a distinct, non-malformed-packet result,
    since RFC 8285 interpretation is opt-in/profile-scoped).
  - `rfc8285::OneByteElement { id: OneByteId, data: &[u8] }` /
    `OneByteElements` — §4.2: `OneByteId` validates the local identifier to
    `1..=14` (0=padding, 15=reserved "stop" marker are not constructible);
    `data` is `1..=16` bytes (`len` nibble = `data.len() - 1`). Parsing
    correctly halts at a reserved ID-15 byte and at the malformed "ID 0 with
    a nonzero length nibble" case (RFC 8285 §4.1.2), keeping only the
    elements seen before it.
  - `rfc8285::TwoByteElement { id: TwoByteId, data: &[u8] }` /
    `TwoByteElements` — §4.3: `TwoByteId` validates the local identifier to
    `1..=255` (0 is reserved for padding **in both forms**, per §4.1.2/§5 —
    not just the one-byte form, despite a literal reading of §4.3 in
    isolation); `data` is `0..=255` bytes, stored directly (no length bias).
  - Both container types implement byte-identical `Parse`/`Serialize`
    round trips from their own canonical (trailing-padding) output.
    Padding position itself is canonicalized rather than preserved
    verbatim: RFC 8285 permits padding between elements (its own §4.2/§4.3
    worked examples do this), but padding carries no semantic content, so
    `Serialize` always emits a single trailing padding run regardless of
    where the original padding fell — decoding still recovers the
    identical element list either way (see
    `docs/rfc8285_header_ext.md`'s judgment call 4).
  - `docs/rfc8285_header_ext.md` — curated transcription of RFC 8285
    §4.1/§4.1.2/§4.2/§4.3 (fetched directly from the RFC), including its two
    worked-example byte diagrams and this crate's judgment calls (the
    ID-0-reserved-in-both-forms point above, and the one-byte "ID 0 with
    nonzero length" malformed-termination case).
  - `tests/round_trip_8285.rs` — spec-derived vectors instantiating both
    RFC 8285 worked examples (§4.2/§4.3) byte-for-byte, plus full-stack
    `RtpPacket` -> `HeaderExtension` -> `rfc8285::parse_extensions`
    composition tests, documented as spec-structure-derived (the RFC
    diagrams leave element IDs/data payloads abstract) rather than a real
    network capture.
  - New runnable example `rfc8285_extensions` (`required-features =
    ["rfc8285"]`): builds a packet with multiplexed one-byte-form
    extensions, serializes, parses it back, and decodes the elements.
  - New fuzz target `rtp_packet` (this crate previously had none):
    exercises `RtpPacket::parse` + round trip, plus (feature `rfc8285`)
    both `OneByteElements::parse` and `TwoByteElements::parse` directly on
    arbitrary bytes.
  - `ExtensionElements` (the `profile_id` dispatch enum) is exempt from the
    workspace's issue #204 label convention (`tests/label_coverage.rs` SKIP
    list) — a data-carrying dispatch wrapper, not a spec/field label.

## [0.1.0] - 2026-07-11

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
