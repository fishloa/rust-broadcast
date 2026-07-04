# Changelog

All notable changes to `media-doctor` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### Added
- Codec-level signalling-vs-bitstream cross-validation checks (issue #567), reusing
  `transmux`'s SPS/NAL/ADTS decoders — no duplicated parsing:
  - `CodecSignallingCheck`: flags a PMT-declared H.264/HEVC/AAC-ADTS PID
    (`stream_type` `0x1B`/`0x24`/`0x0F`) whose elementary stream never once looks
    like that codec's framing (no Annex B NAL / no ADTS sync anywhere) —
    `codec-signalling-mismatch`.
  - `FpsCadenceCheck`: flags an AVC/HEVC track whose VUI-declared frame rate
    disagrees (>10%) with the measured PES-timestamp sample cadence —
    `fps-cadence-mismatch`.
  - `ParamSetsCheck`: flags an IDR (AVC) / IRAP (HEVC) access unit that appears
    on the wire before its PID's SPS+PPS have been observed —
    `missing-parameter-sets`.
  - `InterlaceCheck`: surfaces AVC `frame_mbs_only_flag == 0` (interlaced coding
    tools) — `avc-interlaced-content` (Info; TS/PMT signalling carries no
    progressive/interlace container claim to compare against).
  - `check_container_codec` — a new ISOBMFF/CMAF codec-level check (fragmented via
    `Fmp4Demux`, progressive via `ProgressiveDemux`) for MP4/CMAF input: `avcC`/
    `hvcC` profile/level/chroma vs the record's own embedded SPS
    (`avcc-sps-mismatch`/`hvcc-sps-mismatch`), sample-entry `width`/`height` vs the
    SPS-decoded coded dimensions (`container-sps-dimension-mismatch`), AVC
    `frame_mbs_only_flag == 0` (`avc-interlaced-content`, mirroring `InterlaceCheck`),
    and an Annex B start code left in place of a sample's 4-byte NAL length prefix
    on an AVC/HEVC track (`length-prefix-violation`).
  - The CLI `check` command now sniffs its input (`0x47` TS sync at packet 0/1)
    and runs the TS diagnostic set (v1 + the new codec checks above) for a TS
    file, or `check_container_codec` for an ISOBMFF/CMAF file — one command,
    no new flag.
- Dependency on `transmux` (path, `default-features = false`) for the SPS/NAL/ADTS
  decoders and the `Media`/`Fmp4Demux`/`ProgressiveDemux`/`TsDemux` IR reused by
  the checks above.

## [0.2.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [0.1.0] — 2026-07-01
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
