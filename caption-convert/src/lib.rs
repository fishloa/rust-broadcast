//! Caption/subtitle format conversion: CEA-608/708, EBU Teletext, DVB
//! bitmap subtitling, and TTML/IMSC -> WebVTT, SRT, and IMSC 1.1 (issue
//! #931).
//!
//! # Why this crate exists
//!
//! Every *decoder* this crate needs already ships in this workspace
//! (`cc-data`, `dvb-vbi`, `dvb-subtitle`, `ttml-subtitle`), and the
//! CEA-608/708 -> WebVTT and Teletext -> WebVTT extraction itself already
//! lives in `timed-metadata` (issues #568/#666), including the
//! roll-up/pop-on/paint-on cue-boundary handling that naive converters get
//! wrong. What was missing was: a single crate that (1) wraps those
//! extractors behind a raw-carriage-bytes-in / caption-file-out API, (2)
//! adds the one direction `timed-metadata` does not have -- **parsing**
//! WebVTT back out, so WebVTT <-> SRT is a real two-way conversion, not just
//! a writer -- and (3) is explicit, in types and in docs, about which
//! conversions this crate has *not* built and why.
//!
//! # The conversion matrix
//!
//! [`matrix::MATRIX`] is the single source of truth (with a test pinning it
//! to a complete cross-product); this table is its human-readable mirror.
//! **Lossy** means implemented, but see the module doc for what is dropped.
//! **Unsupported** means structurally impossible without a fundamentally
//! different approach (OCR, mainly). **Not implemented** means plausible
//! future work this cut did not build.
//!
//! | from \ to            | WebVTT         | SRT            | IMSC Text        | IMSC Image        |
//! |-----------------------|----------------|----------------|-------------------|--------------------|
//! | CEA-608               | Lossy          | Lossy          | Not implemented   | Unsupported        |
//! | CEA-708               | Lossy          | Lossy          | Not implemented   | Unsupported        |
//! | Teletext               | Lossy          | Lossy          | Not implemented   | Unsupported        |
//! | DVB bitmap subtitle    | **Unsupported**| **Unsupported**| **Unsupported**   | Not implemented    |
//! | TTML/IMSC              | Not implemented| Not implemented| Lossless (identity)| Not implemented   |
//!
//! Call [`matrix::check`] before attempting a generic dispatch over these
//! four formats -- it returns [`Error::Unsupported`] with the same reason
//! text as the table above. For the four pairs this crate actually
//! implements there is no generic dispatcher at all: use
//! [`Cea608ToWebVtt`]/[`Cea708ToWebVtt`] (feature `cc-data`),
//! [`TeletextToWebVtt`] (feature `teletext`), or [`webvtt_to_srt`] /
//! [`srt_to_webvtt`] directly.
//!
//! ## Why DVB bitmap subtitles cannot become text
//!
//! ETSI EN 300 743 bitmap subtitling carries **pixels** (indexed via a CLUT
//! into region objects), not characters. Turning that into WebVTT/SRT/IMSC
//! Text requires OCR, which is out of scope for this crate (and this
//! project) -- so [`matrix::MATRIX`] marks all three `Unsupported`, not
//! silently empty.
//!
//! ## Why DVB bitmap -> IMSC Image is `NotImplemented`, not built
//!
//! IMSC 1.1's Image Profile (`ttml_subtitle::validation::Profile::Image`,
//! an `<image>` element referencing PNG bytes) is the *correct*, lossless
//! target -- no OCR needed, just re-encode the same pixels. It was not
//! built this cut because `dvb-subtitle` is deliberately carriage-only (its
//! own docs: it parses segment structure but does not decode pixels), so
//! producing a raster would mean building, from scratch, in this crate:
//!
//! 1. RLE decode of the 2-bit/4-bit/8-bit pixel-data sub-blocks (EN 300 743
//!    §7.2.5, Tables 42-44) into per-pixel CLUT indices.
//! 2. CLUT entry resolution (Y/Cb/Cr/T, §7.2.3/Table 32) to RGBA, including
//!    the ITU-R BT.601 YCbCr -> RGB matrix.
//! 3. Region/page compositing (object placement within a region, region
//!    placement within a page, §7.2.1-7.2.2).
//! 4. Image encoding (PNG, since IMSC 1.1 Annex references PNG as a
//!    supported `<image>` MIME type) -- itself needing a DEFLATE/zlib
//!    writer and PNG's own (reflected) CRC-32, distinct from this
//!    project's MPEG-2 CRC-32.
//!
//! That is a full rendering pipeline, not a format conversion, and was
//! judged too large to half-build in this cut (issue #931 explicitly
//! permits documenting it as unimplemented rather than shipping a fake or
//! partial renderer). [`matrix::MATRIX`] flags it `NotImplemented` (not
//! `Unsupported`): nothing here makes it impossible, only unbuilt.
//!
//! ## Why TTML/IMSC conversions were not attempted this cut
//!
//! Issue #931 prioritised four conversions for this cut: CEA-608/708 ->
//! WebVTT, Teletext -> WebVTT, WebVTT <-> SRT, and DVB bitmap -> IMSC Image.
//! TTML/IMSC -> WebVTT/SRT/Image were not among them, so they were not
//! attempted. The one TTML/IMSC row this crate *does* answer,
//! `TtmlImsc -> ImscText`, needs no conversion at all: it is the identity
//! case, already served directly by
//! `ttml_subtitle::Document::parse_str` + `ttml_subtitle::Validator`.
//!
//! # `no_std`
//!
//! `#![no_std]` + `alloc` at the core (parsing/writing WebVTT and SRT is
//! pure string handling over `alloc::String`/`Vec`, no filesystem or I/O).
//! The `std` feature (default on) only affects whether the dependencies
//! this crate re-exports link `std`; nothing in this crate's own logic
//! requires it. Examples and fixture-driven tests use `std::fs` to read
//! files, same as every other crate in this workspace.

#![no_std]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

pub mod error;
pub mod matrix;
mod srt;
mod time;
mod webvtt;

#[cfg(feature = "cc-data")]
#[cfg_attr(docsrs, doc(cfg(feature = "cc-data")))]
mod cc;
#[cfg(feature = "teletext")]
#[cfg_attr(docsrs, doc(cfg(feature = "teletext")))]
mod teletext;

pub use error::Error;
pub use matrix::{MATRIX, MatrixEntry, SourceFormat, Support, TargetFormat, check};
pub use srt::{format_srt_timestamp, parse_srt, write_srt};
pub use timed_metadata::MediaTime;
/// The shared cue type (start/end 90 kHz [`timed_metadata::MediaTime`] plus
/// plain display text), re-exported from `timed-metadata` so this crate's
/// WebVTT/SRT/CEA/Teletext modules all speak the same type.
pub use timed_metadata::webvtt::Cue;
pub use webvtt::{
    ParsedWebVtt, cue_block, escape_payload, format_timestamp, parse_webvtt, write_document,
    write_segment,
};

#[cfg(feature = "cc-data")]
#[cfg_attr(docsrs, doc(cfg(feature = "cc-data")))]
pub use cc::{Cea608ToWebVtt, Cea708ToWebVtt};
#[cfg(feature = "teletext")]
#[cfg_attr(docsrs, doc(cfg(feature = "teletext")))]
pub use teletext::TeletextToWebVtt;

/// Convert a WebVTT document to SRT (issue #931: "near-trivial", both are
/// plain text-and-timing formats).
///
/// Returns the SRT text plus whether anything was dropped: `true` if the
/// source used a construct SRT cannot represent (a cue identifier, cue
/// settings, or a `NOTE`/`STYLE`/`REGION` block -- see [`parse_webvtt`]'s
/// docs for the full list). Plain cues (the common case) round-trip
/// losslessly and this is `false`.
///
/// # Errors
///
/// Propagates [`parse_webvtt`]'s errors: [`Error::InvalidWebVtt`] /
/// [`Error::InvalidTimestamp`].
pub fn webvtt_to_srt(input: &str) -> Result<(alloc::string::String, bool), Error> {
    let parsed = parse_webvtt(input)?;
    Ok((write_srt(&parsed.cues), parsed.lossy))
}

/// Convert an SRT document to WebVTT. Lossless: SRT has no construct
/// WebVTT cannot represent.
///
/// # Errors
///
/// Propagates [`parse_srt`]'s errors: [`Error::InvalidSrt`] /
/// [`Error::InvalidTimestamp`].
pub fn srt_to_webvtt(input: &str) -> Result<alloc::string::String, Error> {
    let cues = parse_srt(input)?;
    Ok(write_document(&cues))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webvtt_to_srt_and_back_is_stable_for_plain_cues() {
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:02.000\nHello CMAF\n\n00:00:02.000 --> 00:00:04.000\nsecond cue\n";
        let (srt, lossy) = webvtt_to_srt(vtt).unwrap();
        assert!(!lossy);
        assert!(srt.starts_with("1\n"));
        let back = srt_to_webvtt(&srt).unwrap();
        let reparsed = parse_webvtt(&back).unwrap();
        assert!(!reparsed.lossy);
        assert_eq!(reparsed.cues.len(), 2);
        assert_eq!(reparsed.cues[0].text, "Hello CMAF");
        assert_eq!(reparsed.cues[1].text, "second cue");
    }

    #[test]
    fn webvtt_to_srt_flags_lossy_cue_settings() {
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:01.000 align:start\nhi\n";
        let (_, lossy) = webvtt_to_srt(vtt).unwrap();
        assert!(lossy);
    }
}
