# timed-metadata

[![crates.io](https://img.shields.io/crates/v/timed-metadata.svg)](https://crates.io/crates/timed-metadata)
[![docs.rs](https://img.shields.io/docsrs/timed-metadata)](https://docs.rs/timed-metadata)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](../LICENSE-MIT)

Convert DPI / timed-metadata signalling between **SCTE-35**, **HLS
`EXT-X-DATERANGE`** and **DASH `emsg`** losslessly, plus a first-pass
**CEA-608/708 and EBU Teletext to WebVTT** cue extractor. `no_std` + `alloc`,
independently versioned.

```toml
[dependencies]
timed-metadata = "0.8"
```

## What it does / what it does not

Ad-insertion and segmentation signalling travels in three formats across an OTT
chain: SCTE-35 `splice_info_section` in MPEG-2 TS, `#EXT-X-DATERANGE` in an HLS
playlist, and an inband `emsg` box in MP4/CMAF. This crate translates between them.
The original `splice_info_section` bytes travel verbatim (DATERANGE `SCTE35-*` hex,
emsg `message_data`), so nothing is re-encoded or interpreted away. A stateful
`Timeline` session adds a wall-clock anchor and 33-bit PTS wrap-unrolling.

It does **not** parse SCTE-35 itself (that is `scte35-splice`), does not parse or
write full HLS playlists (`broadcast-hls`) or full MP4 files, and does not make
ad decisions. The caption half is lossy by design (no placement or styling, see
below); for SRT/TTML conversion use `caption-convert`, which wraps these extractors.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ANSI/SCTE 35 | 2023r1 (via `scte35-splice`) | `splice_info_section` carried verbatim and parsed for id/time/duration | Full | |
| IETF RFC 8216 / draft-pantos-hls-rfc8216bis | draft -22 (1 May 2026, `specs/ietf_draft_pantos_hls_rfc8216bis.txt`) | §4.4.5.1 `EXT-X-DATERANGE` attributes incl. `SCTE35-CMD/OUT/IN` | Partial | SCTE-35 to DATERANGE and DATERANGE tag-line parse/render; no DATERANGE to SCTE-35 event conversion. |
| ANSI/SCTE 214-3 | unverified (not vendored) | §8.3.3 inband `emsg`, scheme `urn:scte:scte35:2013:bin` | Full for the binary scheme | Part number settled from DASH-IF IOP v4.3 §5.4.3 (see `convert/emsg.rs`). The MPD `EventStream` XML form (SCTE 214-1) is not implemented. |
| ISO/IEC 23009-1 (via `mp4-emsg`) | unverified | §5.10.3.3 Event Message Box, v0 <-> v1 re-timing | Full | `emsg_to_v0` / `emsg_to_v1` |
| W3C WebVTT + RFC 8216 §3.5 | curated rules: `specs/rules/webvtt-rules.md` | cue syntax, `X-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:<n>` | Partial | No cue settings (`line`/`position`/`align`), no styling tags. |
| ANSI/CTA-608-E, CTA-708-E (decode in `cc-data`) | CTA-608-E, CTA-708-E (CTA not vendored; 708 cross-source: GY/T 270-2013, vendored, a Chinese adaptation) | CC1 pop-on/roll-up/paint-on, 708 service windows | Partial | feature `cc-data`; documented losses in `webvtt` module docs. |
| ETSI EN 300 706 Enhanced Teletext (decode here) / EN 301 775 §4.5 (carriage in `dvb-vbi`) | V1.2.1 of each (`private/specs-md/INDEX.md`) | Level-1 subtitle pages, Hamming-8/4, odd parity, English national option | Partial | feature `teletext`; no enhancement packets, no styling, English-only national option (`docs/teletext-subtitles.md`). |

### Conversion matrix

| From \ To | HLS `EXT-X-DATERANGE` | DASH `emsg` | SCTE-35 bytes |
|---|---|---|---|
| SCTE-35 `splice_info_section` | `Timeline::to_daterange` / `convert::scte35_to_daterange` | `Timeline::to_emsg` / `convert::scte35_to_emsg` | identity |
| DASH `emsg` (SCTE-35 scheme) | not provided | `emsg_to_v0` / `emsg_to_v1` (re-time) | `convert::emsg_to_scte35` |
| HLS `EXT-X-DATERANGE` | tag-line `DateRange::parse_tag_line` / `to_tag_line` | not provided | raw bytes available on the parsed `DateRange`; no typed conversion |

| Caption source | Output | Feature | Status |
|---|---|---|---|
| CEA-608 CC1 (`cc_data()` triplets) | WebVTT cues | `cc-data` | Partial (lossy) |
| CEA-708 service (`cc_data()` triplets) | WebVTT cues | `cc-data` | Partial (lossy) |
| EBU Teletext Level-1 subtitle page | WebVTT cues | `teletext` | Partial (lossy) |
| Cue list | WebVTT document or HLS segment (`X-TIMESTAMP-MAP`) | always | Full |

## Feature flags

| Feature | Default | Description |
|---|---|---|
| `std` | on | Enable `std` in all dependencies. Without it: `no_std` + `alloc`, all conversions still available. |
| `serde` | on | Serialize/Deserialize on public types where supported. |
| `chrono` | on | `chrono` dependency (wall-clock helpers). |
| `cc-data` | off | CEA-608/708 to WebVTT cue extraction (`webvtt::Cea608CueExtractor`, `Cea708CueExtractor`). |
| `teletext` | off | EBU Teletext (EN 300 706) to WebVTT cue extraction (`webvtt::TeletextCueExtractor`, `webvtt::teletext`). |

## Quick start

```rust
use mp4_emsg::PresentationTime;
use timed_metadata::convert::{EmsgConfig, emsg_to_scte35};
use timed_metadata::{TimeAnchor, Timeline};

// A real Unified Streaming splice_info_section (splice id 2002, 24 s break).
let hex = "FC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D";
let raw: Vec<u8> = (0..hex.len())
    .step_by(2)
    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
    .collect();

// Wall-clock anchor: PTS 0 == 2024-01-15T12:00:00 UTC.
let anchor = TimeAnchor { pts_90k: 0, utc_epoch_ms: 1_705_320_000_000 };
let mut timeline = Timeline::with_anchor(anchor);
let event = timeline.push_scte35(&raw).unwrap();

// Edge 1: SCTE-35 -> HLS EXT-X-DATERANGE.
let line = timeline.to_daterange(&event).unwrap().to_tag_line().unwrap();
assert!(line.starts_with("#EXT-X-DATERANGE:"));
assert!(line.contains("SCTE35-"));

// Edge 2: SCTE-35 -> DASH emsg, and Edge 3: back to the verbatim section.
let cfg = EmsgConfig {
    timescale: 90_000,
    presentation: PresentationTime::Delta(0),
    event_duration: 2_160_000, // 24 s * 90000
    value: "34".to_string(),
    id: event.id.unwrap_or(0),
};
let emsg = timeline.to_emsg(&event, &cfg).unwrap();
assert_eq!(emsg_to_scte35(&emsg).unwrap(), raw);
```

## What is implemented

- **`convert`**: pure functions `scte35_to_daterange`, `scte35_to_emsg`,
  `emsg_to_scte35`, `emsg_to_v0`, `emsg_to_v1`; `EmsgConfig`, `SegmentTiming`.
- **`Timeline` / `TimeAnchor`**: stateful session, PTS to UTC mapping, 33-bit PTS
  wrap-unroll via `broadcast_common::clock33`; RFC 3339 output through `jiff`
  (out-of-range instants clamp rather than print odd years).
- **`DateRange`**: typed `EXT-X-DATERANGE` with checked rendering (`to_tag_line`
  returns `Result`: `ID`/`CLASS` containing `"`, CR or LF, and NaN / negative durations,
  are rejected) built on `broadcast-hls`'s shared attribute-list tokenizer.
- **`TimedEvent`**: neutral event (`EventKind`, id, media time and duration, source payload).
- **`webvtt`**: `Cue`, `write_document`, `write_segment`, `cue_block`, shared text
  helpers (`parse_timestamp`, `format_timestamp_with`, `unescape_payload`,
  `normalize_line_endings`, ...) used by `caption-convert` too; extractors behind
  features. Extractors never emit zero-length cues.

## Design notes

- Lossless carriage: SCTE-35 bytes are never re-encoded.
- `Error` is `#[non_exhaustive]`; the crate is `#![forbid(unsafe_code)]` and
  `no_std` + `alloc` without `std`.
- Hostile-input hardening: checked time arithmetic, a `clock33` tripwire test.
- Caption conversion is lossy and documents every loss; no round trip is claimed.

## Related crates

Depends on `scte35-splice`, `mp4-emsg`, `broadcast-hls`, `broadcast-common`, and
optionally `cc-data` / `dvb-vbi`. Used by `caption-convert` (caption half),
`transmux`, `hls-runtime`, `media-plane`, `multimux`, `media-doctor` and
`compliance-probe`.

## MSRV and licence

Rust **1.95.0**. MIT OR Apache-2.0.
