#![doc = include_str!("../README.md")]

#![no_std]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

/// The finishing methods every converter shares (`finalize`, `cues`,
/// `into_cues`, `into_webvtt`, `into_srt`), expanded inside an `impl` whose
/// struct holds the cue extractor in a field named `extractor`. One copy, so
/// the CEA-608, CEA-708 and Teletext converters cannot drift apart.
#[cfg(any(feature = "cc-data", feature = "teletext"))]
macro_rules! converter_tail {
    () => {
        /// Close any still-open cue at end of stream, at `end_pts_90k`.
        pub fn finalize(&mut self, end_pts_90k: u64) {
            self.extractor.finalize(end_pts_90k);
        }

        /// The cues extracted so far.
        #[must_use]
        pub fn cues(&self) -> &[timed_metadata::webvtt::Cue] {
            self.extractor.cues()
        }

        /// Consume the converter, returning the extracted cues.
        #[must_use]
        pub fn into_cues(self) -> alloc::vec::Vec<timed_metadata::webvtt::Cue> {
            self.extractor.into_cues()
        }

        /// Consume the converter, rendering a standalone WebVTT document.
        #[must_use]
        pub fn into_webvtt(self) -> alloc::string::String {
            crate::webvtt::write_document(&self.into_cues())
        }

        /// Consume the converter, rendering an SRT document.
        #[must_use]
        pub fn into_srt(self) -> alloc::string::String {
            crate::srt::write_srt(&self.into_cues())
        }
    };
}

pub mod error;
pub mod matrix;
mod srt;
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
