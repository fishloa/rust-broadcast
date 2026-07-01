# Changelog

## [Unreleased]

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
