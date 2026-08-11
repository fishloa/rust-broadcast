# Changelog

All notable changes to this crate will be documented in this file.

## [Unreleased]

## [0.1.0] - 2026-08-11

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Added

- `arq` — a sans-IO receiver/sender ARQ reliability engine (issue #741):
  `arq::Receiver` implements TR-06-1 §5.3.1's Reorder Section +
  Retransmission Reassembly Section two-stage buffer, loss detection, and
  retry-capped retransmission-request scheduling; `arq::Sender` implements
  the §5.3.3 sender-side NACK response (locate a previously-sent packet by
  sequence number for retransmission — the lookup mechanism itself is left
  to the implementation per §5.3.3, so this is a bounded ring buffer).
  `arq::ranges_to_fci`/`arq::seqs_to_fci` convert a coalesced loss list into
  the bitmask (Generic NACK) wire format, reproducing TR-06-1 Appendix A's
  worked example exactly.
  - **Retry timing is not a TR-06-1 transcription.** TR-06-1 §5.3.4 states
    outright that retransmission-request backoff is "left to the discretion
    of the implementer," and TR-06-2:2024 was checked and adds no formula
    either. This engine's default retry scheduler is instead modeled on
    **librist** (BSD-2-Clause, the VSF reference implementation, read
    directly from `src/rist-common.c`/`src/flow.c`): an 8-sample RTT EWMA,
    first request at `1.0 * rtt`, every subsequent retry at `1.1 * rtt`,
    clamped to a configurable `[recovery_rtt_min, recovery_rtt_max]`, giving
    up on age-out-of-buffer or a configurable retry cap (age checked first).
    TR-06-1 Appendix B's flat suggested defaults (1000 ms receiver buffer,
    70 ms reorder section, 7 retransmission requests, ~132 ms derived
    interval) remain the documented fallback used only until a receiver's
    first real RTT sample arrives. See the `arq` module doc's Attribution
    section for the full accounting — librist is corroborating evidence for
    a workable policy, not the specification, and the two are not
    conflated.
  - `arq::rtt::rtt_sample` turns a completed `RttEcho` (§5.2.6) request/
    response round trip into an RTT sample for the scheduler.
  - Bite-proofed against the real `fixtures/rist/rist-simple-loss25pct-loopback.pcap`
    capture (`tests/arq_frame15_loss_reproduction.rs`): the engine's own,
    independently-computed NACK output for the verified isolated-loss shape
    documented in `fixtures/rist/PROVENANCE.md` is byte-identical to
    librist's real frame-15 `RangeNack` payload.

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
