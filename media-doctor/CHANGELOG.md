# Changelog

All notable changes to `media-doctor` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.1] - 2026-07-14
### Changed
- Widen the `transmux` dependency to `0.16` (was `0.15`): transmux 0.16.0 adds
  the CENC/CBCS encrypt path (issue #564) and makes one breaking struct-literal
  change (`dash::ContentProtectionSystem` gained a `pssh` field). media-doctor's
  own code is unchanged; this is a dependency-floor bump only.

## [0.4.0] - 2026-07-12
### Added
- `media-doctor watch` — a live, continuous compliance probe (issue #665,
  `docs/IDEAS.md` item #4): ingests a raw MPEG-TS feed over **UDP**
  (`--udp <host:port>`, unicast or multicast — auto-joins the IPv4 multicast
  group when the address is in range) and serves an accumulated snapshot as
  Prometheus text exposition format on `GET /metrics` (`--metrics-addr`,
  default `127.0.0.1:9090`).
  - **Scope note**: this release is UDP-only. The full product-vision idea
    also covers SRT; SRT ingest needs `srt-runtime`'s sans-IO handshake/ARQ
    engine and is left as a follow-up issue, not implemented here.
  - New dependency on `dvb-conformance`: every TS packet is fed to
    `ConformanceMonitor`, exposing the full ETSI TR 101 290 indicator set
    (`media_doctor_conformance_events_total{indicator=...,priority=...}`,
    `media_doctor_conformance_in_sync`), timed against wall-clock arrival
    time rather than stream-embedded PCR.
  - PMT-declared SCTE-35/H.264/HEVC/AAC-ADTS PIDs are discovered dynamically
    (via `dvb-si`'s `SiDemux`, not a fixed PID), feeding incremental
    SCTE-35 `splice_insert` open/closed tracking
    (`media_doctor_scte35_events_total`, `media_doctor_scte35_open_events`),
    decode-timestamp (DTS, else PTS) backward-jump detection
    (`media_doctor_pts_dts_anomalies_total`,
    `media_doctor_pts_dts_anomaly{pid=...}`), and declared-codec-vs-bitstream
    framing mismatch (`media_doctor_codec_signalling_mismatch{pid=...}`) —
    the same checks as `Scte35Check`/`PtsCheck`/`CodecSignallingCheck`,
    restructured to hold state across packets instead of scanning a whole
    buffer. `PcrCheck`/`CcAnomalyCheck` are intentionally not re-wired
    separately: `ConformanceMonitor` already computes the equivalent
    PCR-repetition/discontinuity and continuity-count indicators from the
    same per-packet data.
  - The ingest/metrics core (`media_doctor::WatchState`) is plain
    `no_std`+`alloc` logic with no socket dependency — `feed_datagram` takes
    a raw byte slice and `render_prometheus` renders the current snapshot,
    both unit-tested directly against a real capture
    (`fixtures/ts/m6-single.ts`) chunked into UDP-payload-sized pieces, with
    no socket opened. The `cli`-gated binary is a thin `UdpSocket`/
    `TcpListener` shell (two `std::thread`s sharing `Arc<Mutex<WatchState>>`
    — no async runtime) around this core.
  - Datagrams need not be 188-byte-aligned: `mpeg_ts::resync::TsResync`
    recovers sync-byte-aligned TS packets from whatever bytes are actually
    present, buffering partial packets across calls.
  - Prometheus exposition is hand-rolled (`# HELP`/`# TYPE` + label lines) —
    no `prometheus` crate dependency, matching this workspace's
    dependency-light CLI ethos; the format is simple enough that a small
    formatter is less code than a crate integration.

## [0.3.0] - 2026-07-04
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
