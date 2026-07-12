//! CEA-608/708 -> WebVTT cue conversion (issue #568).
//!
//! Cite: `specs/rules/webvtt-rules.md` (curated W3C WebVTT §4 cue syntax +
//! RFC 8216 §3.5 `X-TIMESTAMP-MAP`), and CTA-608-E / CTA-708-E for the
//! caption semantics (decode owned entirely by the `cc-data` crate, feature
//! `cc-data`).
//!
//! This module is split in two independent halves:
//!
//! - [`Cue`] + [`write_document`] / [`write_segment`] (always available):
//!   pure WebVTT serialization from an already-extracted cue list. No
//!   dependency on `cc-data`.
//! - [`Cea608CueExtractor`] / [`Cea708CueExtractor`] (feature `cc-data`):
//!   turn a decoded CEA-608 CC1 channel / CEA-708 service into a [`Cue`]
//!   sequence by feeding one access unit's `cc_data()` triplets at a time,
//!   tagged with that access unit's 33-bit PTS.
//! - [`TeletextCueExtractor`] (feature `teletext`): turns an EBU Teletext
//!   (ETSI EN 300 706) subtitle page into a [`Cue`] sequence by feeding one
//!   access unit's [`dvb_vbi::TeletextDataField`]s at a time. Unlike the CEA
//!   extractors, the protocol decode (Hamming-8/4 FEC, character sets, page
//!   composition) is NOT owned by the carriage crate (`dvb-vbi` is
//!   deliberately carriage-only — see its module docs) — it lives in
//!   [`teletext`] instead. See that module's docs for the full design.
//!
//! # Cue-boundary detection (the key design decision)
//!
//! `cc-data`'s decoders expose only *state* (`channel_text()` /
//! `service_text()`), not a "just committed" event. This module derives cue
//! boundaries by **diffing that displayed text before and after each fed
//! frame**:
//!
//! - **Pop-on**: `channel_text()`/`screen()` reflect only the *displayed*
//!   memory; while composing a caption (RCL -> PAC -> characters) writes go
//!   to the *non-displayed* buffer and produce no diff. The buffers swap only
//!   on EOC, so a diff-detected boundary is *exactly* the EOC commit event,
//!   and the next diff (typically an EDM erase, or the next EOC) is exactly
//!   the "next erase/replace" the spec notes describe.
//! - **Roll-up** / **paint-on**: characters are written directly to the
//!   displayed buffer, so a diff can fire on every visible change (finer
//!   grained than "one cue per committed row"). In practice a roll-up row is
//!   usually written in one batch before the next control code, so real
//!   streams still produce one boundary per row; pathologically slow
//!   per-character delivery would fragment further. Documented as a known
//!   simplification (see `webvtt-rules.md`'s 608/708 mapping notes).
//!
//! # Documented losses (round-trip is NOT claimed; conversion is lossy)
//!
//! - **Placement**: no `line`/`position`/`align` cue settings are emitted —
//!   `webvtt-rules.md` allows omitting them ("otherwise omit (player
//!   defaults)"); row/column/window-anchor information from 608 PACs / 708
//!   window geometry is dropped.
//! - **Styling**: italics/underline/colour (608 mid-row/PAC attributes, 708
//!   pen attributes) are not carried into the payload as `<i>`/`<u>`/`<c>`
//!   tags; only plain decoded text is emitted.
//! - **708 scope**: only a single service's window text (`service_text`) is
//!   read; multi-window overlap ordering beyond priority, and non-service-1
//!   services, are the caller's choice of `service_number` but are not
//!   auto-merged across services.
//! - **Roll-up granularity**: see cue-boundary detection above.

mod cue;
#[cfg(feature = "teletext")]
pub mod teletext;
mod writer;

pub use cue::Cue;
#[cfg(feature = "teletext")]
pub use cue::TeletextCueExtractor;
#[cfg(feature = "cc-data")]
pub use cue::{Cea608CueExtractor, Cea708CueExtractor};
pub use writer::{cue_block, escape_payload, format_timestamp, write_document, write_segment};
