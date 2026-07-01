# Changelog

All notable changes to `media-doctor` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `PtsCheck` diagnostic: per-PID PES PTS/DTS monotonicity (33-bit wrap-unrolled,
  so a legal wrap is not flagged) + forbidden `PTS_DTS_flags == 0b01` detection
  (ITU-T H.222.0 §2.4.3.7). Honours signalled TS-layer discontinuities.
- Dependency on `mpeg-pes` for PES reassembly + PTS/DTS extraction.

### Fixed
- The CLI now runs the full diagnostic set (`SyncByteCheck`, `PatPmtVersionCheck`,
  `CcAnomalyCheck`, `PcrCheck`, `PtsCheck`) — previously only `SyncByteCheck` ran.

_Unreleased — `media-doctor` has not yet been published to crates.io._
