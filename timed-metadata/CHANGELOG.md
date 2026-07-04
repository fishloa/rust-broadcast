# Changelog — timed-metadata

All notable changes to this crate. Format: [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Added
- **SEI-carried caption input wired to `webvtt`** (#599, follow-up to #568):
  the `Cea608CueExtractor`/`Cea708CueExtractor` API is unchanged — it already
  consumed carriage-agnostic `cc_data::CcTriplet` slices — but this release
  proves and tests the second carriage source, `transmux::nal::caption_cc_data`
  (ATSC A/53 caption SEI in H.264/HEVC access units), converges on the exact
  same cues as the PES `cc_data()` path (#568): the same committed
  `fixtures/cc/cea608_cc1_synthetic.txt` frames, re-wrapped in an SEI NAL
  instead of fed raw, produce byte-identical `Cue`s. Also validated against a
  real ATSC A/53 caption SEI capture (dev-dependency on `transmux` for its
  `TsDemux` + `caption_cc_data`, test-only), decoded text cross-checked
  against an independent `ffmpeg`-derived oracle.

## [0.3.0] - 2026-07-04
### Added
- **`webvtt`** module (feature `cc-data`, off by default): converts CEA-608/708
  closed captions to WebVTT cues (#568). `Cea608CueExtractor` /
  `Cea708CueExtractor` wrap `cc-data`'s decode-only 608/CC1 and 708-service
  models and derive cue start/end boundaries by diffing the decoded displayed
  text after each fed access-unit frame (pop-on boundaries land exactly on
  EOC/erase since `cc-data` only mutates the *displayed* buffer on those
  commands; roll-up/paint-on boundaries are best-effort per visible-text
  change — documented as a known simplification). `Cue` + `write_document` /
  `write_segment` (always available, no `cc-data` dependency) render W3C
  WebVTT §4 cue blocks and RFC 8216 §3.5 HLS segmented output with
  `X-TIMESTAMP-MAP=MPEGTS:<n>,LOCAL:00:00:00.000`, reusing `Timeline`'s
  33-bit PTS wrap-unroll. Lossy by design: no cue `line`/`position`/`align`
  settings and no inline styling (`<i>`/`<u>`/`<c>`) are emitted in this first
  pass — see the module docs for the full list of documented losses.
  Validated against a synthetic-but-spec-real CEA-608 CC1 fixture
  (`fixtures/cc/cea608_cc1_synthetic.txt`, CTA-608-E control/PAC/char codes)
  covering pop-on, roll-up, and paint-on; emitted WebVTT additionally
  cross-checked against `ffmpeg` when available.

## [0.2.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [0.1.2] — 2026-07-01
### Added
- emsg version 0 ↔ version 1 conversion (`emsg_to_v0` / `emsg_to_v1` + `SegmentTiming`),
  recomputing the timing field against the segment EPT (`T = EPT + delta` ==
  `presentation_time`), honouring `timescale` equality and carrying PTO for
  Movie↔Period alignment (ISO/IEC 23009-1:2022 §5.10.3.3). Byte-identical
  round-trip verified against real v0 + v1 (DASH-IF livesim) SCTE-35 emsg fixtures.

## 0.1.1 — 2026-06-29

### Changed
- Dependency `broadcast-common` bump (renamed from `dvb-common`); no API change.

## 0.1.0 — 2026-06-27

Initial release.

### Added

- **`TimedEvent`** — canonical hub type carrying the event's abstract kind
  (`EventKind`: `BreakStart`, `BreakEnd`, `Chapter`, `Unspecified`), optional
  media time, duration, and the lossless verbatim source payload (`SourcePayload::Scte35`
  / `SourcePayload::Emsg`).
- **`TimeAnchor`** — maps a 90 kHz PTS to a UTC wall-clock instant; `rfc3339()`
  converts any `MediaTime` to an ISO-8601 string.
- **`DateRange`** — typed `EXT-X-DATERANGE` model with `to_tag_line()` /
  `parse_tag_line()` (RFC 8216 / draft-pantos-hls-rfc8216bis §4.4.5.1).
- **`convert::scte35_to_daterange`** — pure SCTE-35 → DATERANGE edge.
- **`convert::scte35_to_emsg`** / **`convert::emsg_to_scte35`** — pure
  SCTE-35 ↔ DASH `emsg` edges (SCTE 214-3, scheme `urn:scte:scte35:2013:bin`).
- **`Timeline`** — stateful session: holds the `TimeAnchor`, unrolls 33-bit PTS
  wrap, and exposes `push_scte35` / `to_daterange` / `to_emsg`.
- `no_std` + `alloc`; features: `std` (default), `serde` (default), `chrono` (default).
- `label_coverage` drift-guard (CI gate for `EventKind` / `Scte35Cue` labels).

### Deferred (v0.2+)

- SCTE-104 ingest.
- ID3 timed metadata carriage.
- `segmentation_type_id`-based `EventKind` refinement beyond binary out/in.
- `chrono::DateTime` interop helpers behind the `chrono` feature.
