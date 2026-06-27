//! Timed-metadata / DPI signalling conversion core.
//!
//! Translates SCTE-35 splice information to and from the carriages used in OTT
//! delivery: HLS `EXT-X-DATERANGE` (RFC 8216 / draft-pantos-hls-rfc8216bis
//! §4.4.5.1) and DASH `emsg` (SCTE 214-3, scheme `urn:scte:scte35:2013:bin`).
//!
//! Conversions are lossless: the original `splice_info_section` bytes are
//! carried verbatim (DATERANGE `SCTE35-OUT` hex, emsg `message_data`).
//!
//! Pure functions live in [`convert`]; the stateful [`Timeline`] session adds a
//! wall-clock [`TimeAnchor`] and 33-bit PTS wrap-unrolling.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod anchor;
pub mod convert;
pub mod daterange;
pub mod error;
pub mod event;
pub mod timeline;

pub use anchor::TimeAnchor;
pub use daterange::DateRange;
pub use error::{Error, Result};
pub use event::{EventKind, MediaDuration, MediaTime, SourcePayload, TimedEvent};
pub use timeline::Timeline;

/// 90 kHz — the SCTE-35 / MPEG-2 PTS clock.
pub const PTS_HZ: u64 = 90_000;
