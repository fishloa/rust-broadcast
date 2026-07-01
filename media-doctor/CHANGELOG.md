# Changelog

All notable changes to `media-doctor` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `check_playlist` — text-input HLS playlist validator (RFC 8216): flags a missing
  `#EXTM3U` header, a media playlist without `#EXT-X-TARGETDURATION`, an `#EXTINF`
  duration exceeding the target, and a malformed `#EXT-X-DATERANGE` line (validated
  via `timed-metadata`). Adds `timed-metadata` dependency.
- `Scte35Check` diagnostic: container-level SCTE-35 splice consistency —
  reassembles `splice_info_section`s (table_id 0xFC) and flags unbalanced
  `splice_insert` out/in pairs (out with no matching in by stream end) and
  duplicate open "out"s per `splice_event_id`. Adds `scte35-splice` dependency.
- `PtsCheck` diagnostic: per-PID PES PTS/DTS monotonicity (33-bit wrap-unrolled,
  so a legal wrap is not flagged) + forbidden `PTS_DTS_flags == 0b01` detection
  (ITU-T H.222.0 §2.4.3.7). Honours signalled TS-layer discontinuities.
- Dependency on `mpeg-pes` for PES reassembly + PTS/DTS extraction.

### Fixed
- `PtsCheck` no longer false-positives on real streams. It now (a) only examines
  real PES PIDs (payload starts `00 00 01` + a PES stream_id), so PSI/SI PIDs
  like EIT (0x0012) are no longer misread as PES headers, and (b) validates the
  **decode timestamp** (DTS when present, else PTS) rather than PTS — legal
  B-frame PTS reordering is no longer flagged as `pts-backward`. Verified against
  real captures (`h264_aac.ts`, `france-tnt-pcr.ts`) which now yield zero findings.
- The CLI now runs the full diagnostic set (`SyncByteCheck`, `PatPmtVersionCheck`,
  `CcAnomalyCheck`, `PcrCheck`, `PtsCheck`, `Scte35Check`) — previously only
  `SyncByteCheck` ran.

_Unreleased — `media-doctor` has not yet been published to crates.io._
