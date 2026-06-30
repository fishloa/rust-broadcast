# Changelog

## [Unreleased]

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
