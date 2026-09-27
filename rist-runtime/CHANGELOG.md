# Changelog

All notable changes to this crate will be documented in this file.

## [Unreleased]

### Changed (breaking)
- Serializers now return an error, instead of silently truncating, when a length or count does
  not fit its wire field (#1129). `Error` gains a new `FieldOverflow` variant.

### Fixed
- **#1108 (RIST-W1)**: `arq::sender::Sender::new` now clamps `max_buffered` strictly below half
  the 16-bit sequence space (32 767). At `>= 32768`, once `seq` wrapped, `by_seq.insert` for a
  reused sequence number overwrote the OLDER generation's entry in place instead of going through
  `order`'s eviction path — `by_seq.len()` stopped growing (looking bounded) while `order` grew a
  duplicate `u16` on every send, forever.
- **#1108 (RIST-W2)**: due-request ordering (`Receiver::tick`) sorted numerically, not
  circularly. Across a 16-bit seq wrap, the newest losses sorted before older, more urgent ones,
  so the per-tick cap (`MAX_RANGE_ENTRIES`) dropped the oldest losses instead of the newest, and
  they aged out unrequested. Now sorted by circular distance from `next_expected`.
- `GenericNack`/`RttEcho` no longer silently truncate the RTCP `length` field (16 bits) when the
  packet's word count exceeds it — the FCI/padding is still fully written, so the old code
  emitted a misframed packet with `Ok` (#1129). `RangeNack`'s equivalent cast (already provably
  bounded by its own `MAX_RANGE_ENTRIES` check) is also switched to the same checked helper for
  consistency (#1108/RIST-W4 — audited, not a live bug: both real instances of this cast were
  already fixed by #1129).

## [0.2.0] - 2026-09-26

### Security
Fixes GHSA-w4f3-953h-6jfm, GHSA-q5vp-jg67-2xp9.

### Changed (breaking)
- `arq::Sender::on_range_nack`/`on_generic_nack` now take an additional
  `now: Duration` parameter (matching how the rest of this sans-IO crate
  already threads the clock) and require `&mut self` instead of `&self`,
  needed to fix the response-amplification issue below.

### Fixed
- `arq::Sender::on_range_nack`/`on_generic_nack` returned one retransmission
  per expanded sequence number with no deduplication across a NACK's ranges
  and no cap on the total response, so one small NACK repeating the same
  range across its (up to 16) ranges could still trigger a disproportionate
  volume of retransmitted payload — the earlier O(log n) lookup fix bounded
  lookup *cost* but not response *size*. Both methods now deduplicate + cap
  the response at the sender's buffered-packet count, and rate-limit any
  single sequence number to at most one retransmission per
  `MIN_RETRANSMIT_INTERVAL`.
- `arq::Receiver::feed` used a fixed 512-packet forward-gap ceiling
  (`MAX_GAP`) before treating a jump as a stream resync: too small for a
  legitimate high-bitrate loss burst (never NACKed past ~270 ms at
  20 Mbit/s), and one spoofed/forged far-ahead packet reset the whole
  stream immediately, discarding genuine tracked loss and buffered
  out-of-order packets. The ceiling now scales with the receiver's
  configured `receiver_buffer`, and a jump beyond it only resynchronises
  once confirmed by `RESYNC_CONFIRM_COUNT` consecutive packets continuing
  from the new position — an unconfirmed one-off jump is dropped as an
  outlier instead of resetting the stream.

## [0.1.1] - 2026-08-30

### Fixed
- **#977**: `arq::Sender::on_range_nack`/`on_generic_nack` looked up each
  expanded sequence number with an O(n) linear scan of the sent-packet
  buffer, so an adversarial RangeNack (`Additional = 0xFFFF`, TR-06-1
  §5.3.4's own called-out worst case) against a deep buffer multiplied into
  tens of millions of comparisons. The lookup buffer is now indexed by
  sequence number (a `BTreeMap`, so the crate stays `no_std`+`alloc`),
  turning each lookup into O(log n).
- **#978**: `arq::Receiver::feed` backfilled one `MissingState` BTreeMap
  entry per skipped sequence number with no cap on the gap size, so a single
  forged/spoofed packet claiming a sequence number tens of thousands ahead
  of the last-received one could insert tens of thousands of entries. Gaps
  wider than `MAX_GAP` (512) are now treated as a stream reset (resync on
  the new sequence number, drop stale tracking state) instead of being
  backfilled.

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
