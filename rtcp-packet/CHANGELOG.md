# Changelog

All notable changes to `rtcp-packet` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0] - 2026-10-05
### Changed (breaking)
- **Behaviour change (#1123)**: SR/RR/SDES/BYE/APP parsing now validates and strips the `P`
  (padding) octets per RFC 3550 §6.4.1 instead of leaving them in the body, and **rejects** a
  padded packet whose padding count is zero or exceeds the body. Input that previously parsed
  (with padding bytes leaked into a BYE reason / APP `data`) now parses correctly or returns
  `Error::InvalidValue`. Re-serialization of these typed packets still emits `P=0`.

### Added
- `RtcpPacket` gained a new variant, `Unknown { packet_type, count, padding, payload }`,
  for any `PT` outside the RFC 3550 §6 core set (200-204) — e.g. RTPFB=205/
  PSFB=206 [RFC 4585] or XR=207 [RFC 3611]. Common-header framing only; the
  body is opaque and round-trips byte-identical, including the `P` bit and trailing padding
  octets (kept inside `payload`, flagged by `padding`). Serializing an `Unknown` whose
  `packet_type` is 200-204 is rejected (it could not parse back as `Unknown`). `RtcpPacket` is
  already `#[non_exhaustive]`, so this is additive, not breaking (#1071).

### Fixed
- `CompoundPacket`'s leading-packet rule now also accepts a leading
  `RtcpPacket::Unknown` (an unrecognized `PT`), per RFC 5506 §3.4.2/§4.1
  Reduced-Size RTCP; a leading SDES/BYE/APP is still rejected (#1071).
- `CompoundPacket::parse` rejected the **whole** datagram — including a
  perfectly valid leading SR/RR — the moment it walked into a packet whose
  `PT` was outside 200-204, since `RtcpPacket::parse` returned `Err` for any
  unrecognized type. A browser's WebRTC RTCP is mostly RFC 4585 PSFB/RTPFB
  feedback (NACK/PLI/REMB/transport-cc) or RFC 3611 XR, so this discarded a
  real peer's own SR/RR stats on nearly every datagram. Fixed by the new
  `RtcpPacket::Unknown` variant above; see `tests/real_browser_fixture.rs`
  for a real Chromium capture exercising exactly this shape (#1071).

## [0.3.1] - 2026-08-30

### Fixed
- **#973**: RTCP length-field serialization used a bare `as u16` cast that
  silently wrapped for payloads > ~256 KB. Now uses `u16::try_from` and
  returns `Error::InvalidValue` on overflow.

## [0.3.0] - 2026-08-11

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Added
- `tests/non_exhaustive_coverage.rs` drift guard (issue #806). No public API
  or behaviour change.

## [0.2.0] - 2026-07-29

### Changed (BREAKING)
- **Requires `broadcast-common` 9.** No functional or API change of this
  crate's own; the bump exists solely to carry the new requirement.
  `broadcast-common` 9.0.0 changed `Encrypt::encrypt` to take `&mut self` (so a
  stateful implementor can own a running per-key IV counter — it fixes a
  duplicate-IV/two-time-pad defect), and its `Parse`/`Serialize` traits appear
  in this crate's public API, so a consumer cannot mix a `broadcast-common` 8
  build with this one. That makes it a breaking release even though no line of
  logic here moved.

## [0.1.0] - 2026-07-11

### Added

- New crate, extracted from `transmux::rtcp` (issue #654, part of epic #653)
  unchanged in behavior: a spec-complete RFC 3550 §6 RTCP control-packet
  codec, so it can be reused outside `transmux` (in the spirit of
  `rtp-packet`, extracted the same way in #646/#650).
- `SenderReport` (SR, §6.4.1, PT 200) / `ReceiverReport` (RR, §6.4.2, PT 201)
  / `ReportBlock` (the shared 24-byte reception report block, with the
  §6.4.1 24-bit **signed** `cumulative_lost` sign-extended to `i32` on parse
  and masked back to 24 bits on serialize).
- `SourceDescription` / `SdesChunk` / `SdesItem` / `SdesItemType` (SDES,
  §6.5, PT 202): typed CNAME/NAME/EMAIL/PHONE/LOC/TOOL/NOTE/PRIV item types,
  the type-0 list terminator, and 32-bit chunk padding.
- `Bye` (§6.6, PT 203): SSRC/CSRC list + optional UTF-8 reason text, padded
  to a 32-bit boundary.
- `App` (§6.7, PT 204): subtype (carried in the header's 5-bit count field),
  SSRC, 4-byte ASCII name, 32-bit-aligned application data.
- `RtcpPacket` / `RtcpPacketType`: the PT-byte dispatch enum, with the
  project's #204 `name()` + `impl_spec_display!` label pair.
- `CompoundPacket` (§6.1): a `Vec<RtcpPacket>` enforcing the "first packet
  must be SR or RR" compound-packet rule on both parse and construction,
  with byte-exact round-trip across the whole compound.
- `docs/rtcp.md` — curated transcription of RFC 3550 §6 (fetched directly
  from the RFC), the implementation/audit oracle for this crate. Documents
  two known decode-completeness gaps carried over unchanged from the
  pre-extraction implementation: SR/RR profile-specific extensions (§6.4.1,
  undefined by RFC 3550 itself) are not decoded or preserved on round-trip,
  and the SDES PRIV item's (§6.5.8) internal `prefix`/`value` sub-structure
  is exposed as a flat `text` string rather than separately typed.
- `tests/spec_vectors.rs` — byte-identical round-trip tests over
  spec-derived wire vectors (one per packet type + a compound packet),
  computed directly from the RFC 3550 §6 bit diagrams independently of this
  crate's own serializer. No real RTCP capture exists in this workspace to
  draw a fixture from (unlike `rtp-packet`'s `rtp_simple.bin`: transmux's
  RTCP module was never wired to a hub `Package`/`Unpackage` spoke, so there
  is no producer to capture from), so this crate uses the project's
  documented spec-derived-vector fallback (`docs/CRATE-ACCEPTANCE.md` §3)
  instead, same as `rtp-packet`'s padding/CSRC/extension coverage. The
  in-module `#[cfg(test)]` unit tests (round-trip + mutation-bites) carried
  over from `transmux::rtcp` remain in `src/packet.rs` unchanged.
- Two runnable examples: `build_sender_report` (construct an SR with two
  report blocks from typed fields and serialize) and
  `parse_compound_packet` (build an SR+SDES compound packet, parse it back,
  and confirm a byte-exact round trip).
- `#![no_std]` (via `#![cfg_attr(not(feature = "std"), no_std)]`) + `alloc`;
  builds standalone with `--no-default-features` and on a bare-metal
  (`thumbv7em-none-eabi`) target.
- `serde` support behind the `serde` feature.
- `tests/label_coverage.rs` — the workspace's issue #204 label-convention
  drift-guard; passes today over the three spec/field enums this crate
  defines (`RtcpPacketType`, `SdesItemType`, the `RtcpPacket` dispatch enum),
  all already labelled.
