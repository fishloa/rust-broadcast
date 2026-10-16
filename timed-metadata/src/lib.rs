//! Timed-metadata / DPI signalling conversion core.
//!
//! Translates SCTE-35 splice information to the carriages used in OTT
//! delivery: HLS `EXT-X-DATERANGE` (RFC 8216 / draft-pantos-hls-rfc8216bis
//! §4.4.5.1) and DASH `emsg` (ANSI/SCTE 214-3 §8.3.3, scheme
//! `urn:scte:scte35:2013:bin`).
//! The DASH `emsg` conversion is bidirectional (SCTE-35 to and from `emsg`);
//! the HLS `EXT-X-DATERANGE` conversion is currently one-way (SCTE-35 to
//! DATERANGE only) — the reverse edge is out of scope for now, see
//! [`convert::scte35_to_daterange`] doc.
//!
//! Conversions are lossless: the original `splice_info_section` bytes are
//! carried verbatim (DATERANGE `SCTE35-OUT` hex, emsg `message_data`).
//!
//! Pure functions live in [`convert`]; the stateful [`Timeline`] session adds a
//! wall-clock [`TimeAnchor`] and 33-bit PTS wrap-unrolling.
//!
//! [`webvtt`] converts a different kind of timed metadata — CEA-608/708
//! closed captions — to WebVTT cues (W3C WebVTT + RFC 8216 §3.5
//! `X-TIMESTAMP-MAP`), feature `cc-data` for the CEA-608/708 extraction half.
//! Lossy by design (see its module docs).
//!
//! # What it does / what it does not
//!
//! The original `splice_info_section` bytes travel verbatim, so nothing is
//! re-encoded. It does **not** parse SCTE-35 itself (`scte35-splice`), parse or
//! write whole HLS playlists (`broadcast-hls`) or MP4 files, or make ad
//! decisions. Caption conversion is lossy (no placement or styling); for SRT /
//! TTML use the `caption-convert` crate.
//!
//! # Standards supported
//!
//! | Standard | Edition | Scope | Status |
//! |---|---|---|---|
//! | ANSI/SCTE 35 | 2023r1 (via `scte35-splice`) | section carried verbatim | Full |
//! | RFC 8216 / draft-pantos-hls-rfc8216bis | draft -22 | §4.4.5.1 `EXT-X-DATERANGE` | Partial (SCTE-35 to DATERANGE, tag-line parse/render; no DATERANGE to SCTE-35) |
//! | ANSI/SCTE 214-3 | unverified (not vendored) | §8.3.3 inband `emsg`, `urn:scte:scte35:2013:bin` | Full (binary scheme only) |
//! | ISO/IEC 23009-1 | unverified | §5.10.3.3 `emsg`, v0 <-> v1 re-timing | Full |
//! | W3C WebVTT + RFC 8216 §3.5 | curated rules | cue syntax, `X-TIMESTAMP-MAP` | Partial |
//! | CTA-608-E / CTA-708-E (decode in `cc-data`) | unverified (not vendored) | CC1, 708 service | Partial, feature `cc-data` |
//! | ETSI EN 300 706 / EN 301 775 §4.5 | V1.2.1 | Level-1 subtitle pages | Partial, feature `teletext` |
//!
//! | From \ To | `EXT-X-DATERANGE` | `emsg` | SCTE-35 bytes |
//! |---|---|---|---|
//! | SCTE-35 | [`convert::scte35_to_daterange`] | [`convert::scte35_to_emsg`] | identity |
//! | `emsg` | not provided | [`convert::emsg_to_v0`] / [`convert::emsg_to_v1`] | [`convert::emsg_to_scte35`] |
//! | `EXT-X-DATERANGE` | [`DateRange::parse_tag_line`] / [`DateRange::to_tag_line`] | not provided | raw bytes on the parsed [`DateRange`] |
//!
//! # Feature flags
//!
//! | Feature | Default | Effect |
//! |---|---|---|
//! | `std` | on | `std` in all dependencies; off = `no_std` + `alloc` |
//! | `serde` | on | serde support on public types |
//! | `chrono` | on | `chrono` dependency |
//! | `cc-data` | off | CEA-608/708 to WebVTT extractors |
//! | `teletext` | off | EBU Teletext to WebVTT extractor |
//!
//! # Quick start
//!
//! ```
//! use mp4_emsg::PresentationTime;
//! use timed_metadata::convert::{EmsgConfig, emsg_to_scte35};
//! use timed_metadata::{TimeAnchor, Timeline};
//!
//! // A real Unified Streaming splice_info_section (splice id 2002, 24 s break).
//! let hex = "FC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D";
//! let raw: Vec<u8> = (0..hex.len())
//!     .step_by(2)
//!     .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
//!     .collect();
//!
//! // Wall-clock anchor: PTS 0 == 2024-01-15T12:00:00 UTC.
//! let anchor = TimeAnchor { pts_90k: 0, utc_epoch_ms: 1_705_320_000_000 };
//! let mut timeline = Timeline::with_anchor(anchor);
//! let event = timeline.push_scte35(&raw).unwrap();
//!
//! // Edge 1: SCTE-35 -> HLS EXT-X-DATERANGE.
//! let line = timeline.to_daterange(&event).unwrap().to_tag_line().unwrap();
//! assert!(line.starts_with("#EXT-X-DATERANGE:"));
//! assert!(line.contains("SCTE35-"));
//!
//! // Edge 2: SCTE-35 -> DASH emsg, and Edge 3: back to the verbatim section.
//! let cfg = EmsgConfig {
//!     timescale: 90_000,
//!     presentation: PresentationTime::Delta(0),
//!     event_duration: 2_160_000, // 24 s * 90000
//!     value: "34".to_string(),
//!     id: event.id.unwrap_or(0),
//! };
//! let emsg = timeline.to_emsg(&event, &cfg).unwrap();
//! assert_eq!(emsg_to_scte35(&emsg).unwrap(), raw);
//! ```
//!
//! # Design notes and related crates
//!
//! `#![forbid(unsafe_code)]`, `no_std` + `alloc` without `std`, `Error` is
//! `#[non_exhaustive]`, checked time arithmetic. Depends on `scte35-splice`,
//! `mp4-emsg`, `broadcast-hls`, `broadcast-common`, optionally `cc-data` /
//! `dvb-vbi`; used by `caption-convert`, `transmux`, `hls-runtime`,
//! `media-plane` and `multimux`. MSRV 1.95.0; MIT OR Apache-2.0.
#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

pub mod anchor;
pub mod convert;
pub mod daterange;
pub mod error;
pub mod event;
pub mod timeline;
pub mod webvtt;

pub use anchor::TimeAnchor;
pub use daterange::DateRange;
pub use error::{Error, Result};
pub use event::{EventKind, MediaDuration, MediaTime, SourcePayload, TimedEvent};
pub use timeline::Timeline;

/// 90 kHz — the SCTE-35 / MPEG-2 PTS clock.
pub const PTS_HZ: u64 = 90_000;
