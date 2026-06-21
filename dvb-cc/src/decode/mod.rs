//! CEA-608 / CEA-708 caption **decode** — the meaning of the caption byte pairs
//! that the [`crate::CcData`] carriage demuxes out of `cc_data()`.
//!
//! This layer sits above the carriage ([ETSI TS 101 154 Table B.9](crate)) and
//! interprets the line-21 (CEA-608) and DTVCC (CEA-708) byte streams into a
//! caption screen / window model whose displayed text can be extracted.
//!
//! - [`Cea608Decoder`] — fed the 608 byte-pairs (`cc_type` 0 = field 1, 1 =
//!   field 2). Decodes the line-21 control-code state machine per ANSI/CTA-608-E
//!   (`dvb-cc/docs/decode/cea608-decode.md`): pop-on (RCL/EOC), roll-up
//!   (RU2/RU3/RU4/CR), paint-on (RDC), PACs, mid-row codes, the standard /
//!   special / extended Western-European character sets, tab offsets, the four
//!   data channels CC1–CC4, with control-code doubling and field-2 XDS skip.
//! - [`Cea708Decoder`] — fed the DTVCC `cc_data` bytes. Reassembles caption
//!   channel packets, parses service blocks, and runs the C0/C1/G0/G1/G2/G3
//!   command interpreter — the window and pen model — per ANSI/CTA-708-E
//!   (`dvb-cc/docs/decode/cea708-decode.md`) and the 47 CFR §79.102 conformance
//!   model (`cea708-conformance.md`). Exposes the six services' window text.
//!
//! The shared caption display model — typed colour / opacity / edge / font
//! enumerations ([`Color`], [`Opacity`], [`EdgeType`], [`FontStyle`], …) — is
//! re-exported here, per the conformance model.
//!
//! Decoders are **one-way** (bytes → caption state) and **panic-free** on
//! arbitrary input: malformed / truncated / over-length byte streams are ignored
//! rather than panicking.

mod cea608;
mod cea708;
mod screen;

pub use cea608::{
    Cea608Channel, Cea608Color, Cea608Decoder, Cea608Mode, Cea608Row, Cea608Screen,
    Cea608StyledChar,
};
pub use cea708::{AnchorPoint, Cea708Decoder, Window, WindowState};
pub use screen::{
    Color, EdgeType, FontStyle, Justify, Opacity, PenOffset, PenSize, PrintDirection,
    ScrollDirection,
};
