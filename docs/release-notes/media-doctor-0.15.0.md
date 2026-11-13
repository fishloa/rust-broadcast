# media-doctor 0.15.0

_Released 2026-11-14._

Major-class release (0.14 -> 0.15) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 0.14.0 -> 0.15.0.

## Added

- `check_container_codec` reports `hvcc-array-nal-type` and `hvc1-array-completeness` warnings.

## Fixed

- No false `dash-static-mpd-missing-duration` when last Period carries `@duration` or MPD carries `@minimumUpdatePeriod`.
- `avcc-sps-mismatch`/`hvcc-sps-mismatch` flag record level only when below SPS level.
- `PatPmtVersionCheck` drops PMT failing CRC-32.
- Shared codec PSI discovery skips bad PMTs and ignores reserved PAT PIDs.
- Open SCTE-35 event set capped at 1024 ids.
- Byte-identical `splice_info_section` treated as retransmission.
- `CcAnomalyCheck` flags adaptation-only packet with wrong CC.
- Unparseable `#EXT-X-TARGETDURATION`/`#EXTINF` no longer read as 0.
- ISOBMFF sniff accepts compressed `!mov`/`!mof`.

## Also in this release

### Added
- `hls-program-date-time-invalid` (Warning): an `#EXT-X-PROGRAM-DATE-TIME` that is not an ISO 8601 date-time (a bare date, say) is reported at its line from the playlist parser's new `MediaPlaylist::diagnostics` instead of failing the playlist (#2591).
- `media_doctor_scte35_cues_total{timing=...}` and the matching `WatchSnapshot` fields: each SCTE-35 cue is judged against the program's reference decode timestamp, by the same engine `compliance-probe` reports.
- Use shared `SpliceTracker` (scte35-splice) and `ConformanceMonitor::indicator_counts()` instead of local copies.

### Changed
- `udp::bind_udp` now uses `dvb_stream::udp::bind_standard` (shared socket bind) instead of its own socket2 sequence (#2472).
- `check_dash_mpd` reads MPDs through the new `broadcast-dash` crate (the `dash` feature now enables `dep:broadcast-dash` instead of `dep:dash-runtime`), so the validator depends on the manifest syntax alone, not on a player (#2355).
- `check_dash_mpd` reads MPDs with `dash_runtime::mpd` (new dependency, under `std`) instead of transmux's MPD model, which was removed (#2340). Its findings are pinned on every MPD fixture and the ISO/IEC 23009-1 examples.

### Fixed
- `has_adts_sync` drops its own syncword pre-test and relies on the strict `transmux::parse_adts_header`, so an AAC-declared PID carrying MP3 frames is now flagged.
- A `MPD@type="list"` MPD (ISO/IEC 23009-1 6th edition) is no longer reported with `dash-period-no-adaptation-sets` for Periods that import an MPD (`ImportedMPD`) or with `dash-static-mpd-missing-duration`; both were false positives.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `broadcast-hls` 0.6 -> 0.7, `container-probe` 0.3 -> 0.4, `dvb-conformance` 13 -> 14, `dvb-si` 13 -> 14, `mpeg-pes` 0.7 -> 0.8, `mpeg-ts` 0.7 -> 0.8, `scte35-splice` 5 -> 6, `timed-metadata` 0.9 -> 0.10, `transmux` 0.30 -> 0.31.
- New sibling dependencies: `broadcast-dash` 0.1, `dash-runtime` 0.4, `dvb-stream` 0.9.

---

Published from tag `media-doctor-v0.15.0`.
