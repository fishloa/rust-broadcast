# Changelog

## [Unreleased]

## [0.5.0] - 2026-08-11

### Added
- Feature `cli` (on by default), gating the `ts-fix` binary and its `clap`
  dependency. `cargo install ts-fix` is unaffected; `--no-default-features`
  now yields a library with no `clap` in its tree.

### Changed
- The crate root has carried `#![cfg_attr(not(feature = "std"), no_std)]`
  since 0.1.0, but no build could satisfy it: `clap` was an unconditional
  dependency, so `--no-default-features` still dragged in `anstyle` and the
  std runtime. Gating `clap` behind the new `cli` feature makes the
  attribute reachable, and `ts-fix` is now built for `thumbv7em-none-eabi`
  by CI's `no_std` job. The library itself needed no source change — it uses
  no `std` path.
- `ops::continuity::ContinuityOp`'s legal-duplicate detection (ITU-T H.222.0
  §2.4.3.3) now delegates to the new shared
  `broadcast_common::ts_dup::is_legal_duplicate_pair`, replacing a
  hand-rolled FNV-1a-style hash (`payload_hash`) that independently
  implemented the same byte-identity-except-PCR rule already duplicated in
  `dvb-conformance` and `media-doctor`. `PidState` now stores the previous
  packet's raw bytes instead of a hash of them. No behaviour change: the
  old hash already covered the full packet minus the PCR field (header +
  adaptation-field body + payload), so it was already correct on this
  property — only the *mechanism* (hash vs. direct comparison) was
  duplicated, and a hash carries a theoretical (if astronomically remote)
  collision risk the direct comparison doesn't. Confirmed unchanged by this
  crate's full test suite, including the exact 5-legal-duplicate /
  0-remaining-error assertions on `fixtures/ts/m6-duplicate.ts` and
  `m6-single.ts`; added a new unit test
  (`splice_countdown_difference_is_not_a_legal_duplicate`) pinning the
  AF-body (non-PCR) byte-identity requirement this refactor preserves.
  `tests/cc_repair.rs`'s own hand-rolled `hash_payload_skip_pcr` (a fourth,
  simpler and less complete copy of the same rule, used only to establish
  ground truth in test assertions) is replaced by a direct call to the same
  shared function.
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
## [0.4.2] - 2026-07-30

### Fixed
- Floor `mpeg-ts` to `0.3.1`. The `^0.3` bucket also contains 0.3.0, which is
  built against `broadcast-common` 8, so a consumer could resolve two
  `broadcast-common` majors into one graph and hit trait-resolution errors
  pointing at this crate's internals (#858).

## [0.4.1] - 2026-07-30

### Added
- `tests/label_coverage.rs` + `tests/non_exhaustive_coverage.rs` drift guards
  (issue #806). No public API or behaviour change.

## [0.4.0] - 2026-07-29

### Changed (BREAKING)
- **Requires `broadcast-common` 9** (issue #819). No functional or API change of
  this crate's own.

  Staying on `broadcast-common` 8 was not neutral: this crate's types implement
  `Parse`/`Serialize` from whichever major it links, so a consumer that used it
  alongside a 9-based crate (`transmux` 0.20, `dvb-si` 9, …) got **both majors
  in one graph**, and the trait methods resolved against the wrong one —
  surfacing as `no method named to_bytes found` / `no function named parse
  found` on types that plainly have them, with the compiler pointing at
  `broadcast-common-8.x/src/traits.rs`.

  The 9.0.0 wave originally shipped only the crates needed to publish
  `transmux`/`media-plane`/`multimux`, on the reasoning that everything else
  stayed coherent on its own 8 line. That reasoning was wrong: these crates
  exist to be composed, and the breakage only appears in a consumer that mixes
  them.

## [0.3.1] - 2026-07-21
### Changed
- Widen the internal `mpeg-ts` dependency to `0.3` (was `0.2`; issue #663).
  `ops::psi_regen::PsiRegenOp`'s internal PAT rebuild now calls the renamed
  `mpeg_ts::mux::SectionPacketiser`/`packetise` (was `SectionPacketizer`/
  `packetize`) — an internal identifier rename following `mpeg-ts` 0.3's
  British-spelling rename, no public API or behaviour change to `ts-fix`.

## [0.3.0] - 2026-07-04
### Added
- PCR-discontinuity detection + repair (#562):
  - `discontinuity::detect_pcr_discontinuities` + `PcrDiscontinuity` — scan a TS
    buffer for PCR jumps on every PCR-bearing PID, classified as **flagged**
    (`discontinuity_indicator == 1`, ISO/IEC 13818-1 §2.4.3.5 — a legal
    system-time-base change) or **unflagged** (ETSI TR 101 290 v1.4.1 §5.2.2
    Table 5.0b indicator 2.3b `PCR_discontinuity_indicator_error` — a genuine
    defect). The 2.3b threshold is reused verbatim from
    `dvb_conformance::ConformanceMonitor`, never re-derived.
  - `TsFixBuilder::honor_pcr_discontinuity()` — new **honor** repair mode: sets
    `discontinuity_indicator` on genuine, unflagged PCR breaks without
    rewriting any timestamp byte (only the AF flags bit changes). CLI flag
    `--honor-pcr-discontinuity`.
  - `restamp_pcr` (Interpolate mode) now classifies every observed forward
    jump against the same TR 101 290 2.3b threshold instead of a bare
    modulus-half heuristic: a genuine, unflagged break is never adopted as a
    "sane" observation, and the PID's anchor is permanently frozen onto its
    pre-break rate from that point on, so the restamped output stays on one
    continuous PCR timeline across (and past) the break. `FromBitrate` mode
    already shipped this guarantee for free.
  - New `dvb-conformance` dependency (path, workspace-pinned).

## [0.2.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [0.1.0] — 2026-07-01
### Added
- PES access-unit reconstruction (`pes::reconstruct_access_units` + `AccessUnit`):
  reassemble the PES access units on given PIDs from a TS buffer — framing only,
  no codec bitstream parsing — exposing per-AU PID / PTS / DTS + the reassembled
  PES bytes (via `mpeg-pes`). Gives future ops the AU boundaries they need
  (e.g. clean cut points). Adds `mpeg-pes` dependency.
- SCTE-35 cue preservation guarantee (tests): PID-filter keep-mode passes the
  splice PID + its `splice_info_section`s through byte-intact, and `restamp_pcr`
  leaves SCTE-35 sections untouched while it rewrites the PCR PID (the cue is
  preserved across remux). Shifting the splice PTS to match a restamped PCR is
  tracked separately (#417).

### Fixed
- `restamp_pcr` (Interpolate mode) now handles the 33-bit PCR base wrap: a legal
  wrap (where the raw 27 MHz value appears to decrease) is recognised via a
  modular forward-distance test on `2^33 × 300`, instead of being mistaken for a
  corrupt/non-monotonic observation and recomputed into a bogus discontinuity.
  Computed values wrap at the PCR boundary (ISO/IEC 13818-1 §2.4.3.5).

### Added
- `restamp_pcr(cfg: PcrRestamp)` builder method + `PcrRestamp` config enum with
  `interpolate()` and `from_bitrate(bps)` constructors — recompute PCR values
  on the PCR PID via mpeg-ts `OwnedTsPacket::set_pcr` (ISO/IEC 13818-1 §2.4.3.5).
- `TimingContext` in `ops::StreamModel` — forward-compat 27 MHz clock/anchor
  state, designed for reuse by PTS/DTS-wrap in v0.2.
- Engine canonical ordering now enforced in `TsFixBuilder::build()`:
  filter_pids → regen_psi → repair_continuity → restamp_pcr → stuffing.
- CLI flags `--restamp-pcr-interpolate` and `--restamp-pcr-bitrate <BPS>`.
- Fault-inject PCR restamp integration test (`tests/pcr_restamp.rs`).

### Changed
- **thinned onto mpeg-ts editors**: `continuity.rs` now writes the continuity
  counter via `OwnedTsPacket::set_continuity_counter` instead of raw nibble
  twiddling on `buf[3]`. `stuffing.rs` now builds null packets via
  `OwnedTsPacket::null_packet` instead of raw byte construction. No raw wire
  bytes remain in `ts-fix/src/ops/{continuity,stuffing}.rs`.
