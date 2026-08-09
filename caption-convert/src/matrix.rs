//! The conversion matrix: which `(source, target)` pairs this crate
//! implements, and — for every pair it does not — *why*, so a caller never
//! silently gets empty output for a format this crate cannot honestly
//! convert.
//!
//! [`MATRIX`] is the single source of truth; [`lookup`] and [`check`] read
//! it. The crate root docs render the same table for humans.

use crate::error::Error;

/// A caption/subtitle **source** this crate can read.
///
/// `Cea608`/`Cea708` (CTA-608-E / CTA-708-E, carried per `cc-data`'s
/// `cc_data()` model), `Teletext` (ETSI EN 300 706 subtitle pages, carried
/// per `dvb-vbi`'s `TeletextDataField`), `DvbBitmapSubtitle` (ETSI EN 300 743
/// bitmap subtitling, an *image* format — see [`MATRIX`]'s notes on why it
/// cannot become plain text without OCR), and `TtmlImsc` (W3C TTML2 / IMSC
/// 1.1, parsed by `ttml-subtitle`).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SourceFormat {
    /// CEA-608 (line-21) closed captions (CTA-608-E).
    Cea608,
    /// CEA-708 (DTVCC) closed captions (CTA-708-E).
    Cea708,
    /// EBU Teletext subtitle page (ETSI EN 300 706).
    Teletext,
    /// DVB bitmap subtitling (ETSI EN 300 743) — an image format.
    DvbBitmapSubtitle,
    /// W3C TTML2 / IMSC 1.1 document.
    TtmlImsc,
}

impl SourceFormat {
    /// Stable label for this variant.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            SourceFormat::Cea608 => "cea-608",
            SourceFormat::Cea708 => "cea-708",
            SourceFormat::Teletext => "teletext",
            SourceFormat::DvbBitmapSubtitle => "dvb-bitmap-subtitle",
            SourceFormat::TtmlImsc => "ttml-imsc",
        }
    }
}
broadcast_common::impl_spec_display!(SourceFormat);

/// A caption/subtitle **target** this crate can emit.
///
/// `WebVtt` (W3C WebVTT) and `Srt` (SubRip Text — no formal specification;
/// this crate follows the de facto ffmpeg/VLC-compatible format) are plain
/// text-and-timing formats. `ImscText` / `ImscImage` are W3C TTML2 / IMSC
/// 1.1's two disjoint profiles (`ttml-subtitle::validation::Profile`):
/// `ImscImage` is the only honest target for a bitmap source (no OCR).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TargetFormat {
    /// W3C WebVTT.
    WebVtt,
    /// SubRip Text (SRT) — no formal specification.
    Srt,
    /// IMSC 1.1 Text Profile.
    ImscText,
    /// IMSC 1.1 Image Profile.
    ImscImage,
}

impl TargetFormat {
    /// Stable label for this variant.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            TargetFormat::WebVtt => "webvtt",
            TargetFormat::Srt => "srt",
            TargetFormat::ImscText => "imsc-text",
            TargetFormat::ImscImage => "imsc-image",
        }
    }
}
broadcast_common::impl_spec_display!(TargetFormat);

/// How a `(source, target)` conversion is (or is not) supported.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Support {
    /// Implemented; no information is dropped.
    Lossless,
    /// Implemented; some information is deliberately dropped (see the
    /// matrix entry's `reason`). Never silent — every function that performs
    /// a lossy conversion documents (and, for `webvtt_to_srt`, returns) the
    /// fact that it lost something.
    Lossy,
    /// Structurally impossible for this crate to do honestly (e.g. bitmap
    /// pixels -> plain text needs OCR, which is out of scope). This will
    /// never become `Lossless`/`Lossy` without a fundamentally different
    /// approach.
    Unsupported,
    /// Not implemented in this cut, but plausible future work (e.g. it needs
    /// a rendering pipeline this crate has not built yet). Distinct from
    /// `Unsupported`: nothing prevents building it later.
    NotImplemented,
}

impl Support {
    /// Stable label for this variant.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Support::Lossless => "lossless",
            Support::Lossy => "lossy",
            Support::Unsupported => "unsupported",
            Support::NotImplemented => "not implemented",
        }
    }
}
broadcast_common::impl_spec_display!(Support);

/// One row of [`MATRIX`]: a `(from, to)` pair, its [`Support`] class, and a
/// human-readable reason (always populated — even for `Lossless` rows, so
/// the table is self-explanatory without cross-referencing module docs).
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub struct MatrixEntry {
    /// The source format.
    pub from: SourceFormat,
    /// The target format.
    pub to: TargetFormat,
    /// The support class for this pair.
    pub support: Support,
    /// Why. Always populated.
    pub reason: &'static str,
}

use SourceFormat::{Cea608, Cea708, DvbBitmapSubtitle, Teletext, TtmlImsc};
use Support::{Lossless, Lossy, NotImplemented, Unsupported};
use TargetFormat::{ImscImage, ImscText, Srt, WebVtt};

/// The full `SourceFormat` x `TargetFormat` cross-product (5 x 4 = 20 rows).
/// Every pair is listed explicitly — including the ones that make no sense
/// (e.g. a text caption source targeting the Image profile) — so nothing is
/// covered by omission.
pub const MATRIX: &[MatrixEntry] = &[
    // --- CEA-608 ---
    MatrixEntry {
        from: Cea608,
        to: WebVtt,
        support: Lossy,
        reason: "roll-up/pop-on/paint-on text and cue timing carry over \
                 exactly (timed-metadata::webvtt::Cea608CueExtractor, issue \
                 #568); PAC row/column placement, mid-row style attributes, \
                 and colour are dropped (no line/position cue settings or \
                 <i>/<u>/<c> tags are emitted)",
    },
    MatrixEntry {
        from: Cea608,
        to: Srt,
        support: Lossy,
        reason: "same loss as Cea608 -> WebVtt (SRT has no placement/style \
                 model either, so the WebVTT step adds no further loss); \
                 implemented as Cea608 -> WebVTT -> SRT",
    },
    MatrixEntry {
        from: Cea608,
        to: ImscText,
        support: NotImplemented,
        reason: "not attempted this cut; would need a Cue -> TTML <p> \
                 mapping (straightforward) plus a decision on whether to \
                 preserve PAC placement as TTML region/style (not \
                 straightforward) -- see issue #931",
    },
    MatrixEntry {
        from: Cea608,
        to: ImscImage,
        support: Unsupported,
        reason: "the source is already text; rendering it to an image \
                 profile has no defined purpose here",
    },
    // --- CEA-708 ---
    MatrixEntry {
        from: Cea708,
        to: WebVtt,
        support: Lossy,
        reason: "window text and cue timing carry over exactly \
                 (timed-metadata::webvtt::Cea708CueExtractor, issue #568); \
                 window geometry/anchor, pen colour/style, and any \
                 non-primary service the caller did not select are dropped",
    },
    MatrixEntry {
        from: Cea708,
        to: Srt,
        support: Lossy,
        reason: "same loss as Cea708 -> WebVtt; implemented as \
                 Cea708 -> WebVTT -> SRT",
    },
    MatrixEntry {
        from: Cea708,
        to: ImscText,
        support: NotImplemented,
        reason: "not attempted this cut; same shape of work as \
                 Cea608 -> ImscText plus 708's richer window model",
    },
    MatrixEntry {
        from: Cea708,
        to: ImscImage,
        support: Unsupported,
        reason: "the source is already text; rendering it to an image \
                 profile has no defined purpose here",
    },
    // --- Teletext ---
    MatrixEntry {
        from: Teletext,
        to: WebVtt,
        support: Lossy,
        reason: "page text and cue timing carry over exactly \
                 (timed-metadata::webvtt::TeletextCueExtractor, issue #666); \
                 double-height/double-size, colour, boxing, and enhancement \
                 packets (Level 1.5+) are dropped -- only basic Level 1 rows \
                 1-24 are decoded",
    },
    MatrixEntry {
        from: Teletext,
        to: Srt,
        support: Lossy,
        reason: "same loss as Teletext -> WebVtt; implemented as \
                 Teletext -> WebVTT -> SRT",
    },
    MatrixEntry {
        from: Teletext,
        to: ImscText,
        support: NotImplemented,
        reason: "not attempted this cut; same shape of work as \
                 Cea608 -> ImscText",
    },
    MatrixEntry {
        from: Teletext,
        to: ImscImage,
        support: Unsupported,
        reason: "the source is already text; rendering it to an image \
                 profile has no defined purpose here",
    },
    // --- DVB bitmap subtitle ---
    MatrixEntry {
        from: DvbBitmapSubtitle,
        to: WebVtt,
        support: Unsupported,
        reason: "DVB bitmap subtitles (ETSI EN 300 743) are pixels, not \
                 text; producing WebVTT cues would require OCR, which is \
                 out of scope for this crate -- see the crate root docs",
    },
    MatrixEntry {
        from: DvbBitmapSubtitle,
        to: Srt,
        support: Unsupported,
        reason: "same reason as DvbBitmapSubtitle -> WebVtt: needs OCR, \
                 out of scope",
    },
    MatrixEntry {
        from: DvbBitmapSubtitle,
        to: ImscText,
        support: Unsupported,
        reason: "same reason as DvbBitmapSubtitle -> WebVtt: needs OCR, \
                 out of scope",
    },
    MatrixEntry {
        from: DvbBitmapSubtitle,
        to: ImscImage,
        support: NotImplemented,
        reason: "the correct lossless target (IMSC 1.1 Image Profile \
                 carries an <image> element referencing PNG bytes -- no \
                 OCR needed), but not built this cut: it needs a full \
                 rendering pipeline dvb-subtitle deliberately does not \
                 provide (it is carriage-only) -- RLE decode of the 2/4/8 \
                 bit pixel-data sub-blocks (EN 300 743 SS7.2.5), CLUT entry \
                 resolution (Y/Cb/Cr/T -> RGBA), region/page compositing, \
                 and PNG encoding. Judged too large for this cut; see the \
                 crate root docs",
    },
    // --- TTML/IMSC ---
    MatrixEntry {
        from: TtmlImsc,
        to: WebVtt,
        support: NotImplemented,
        reason: "not attempted this cut; not one of the four conversions \
                 prioritised for this cut (issue #931)",
    },
    MatrixEntry {
        from: TtmlImsc,
        to: Srt,
        support: NotImplemented,
        reason: "not attempted this cut; not one of the four conversions \
                 prioritised for this cut (issue #931)",
    },
    MatrixEntry {
        from: TtmlImsc,
        to: ImscText,
        support: Lossless,
        reason: "identity: already TTML/IMSC. Use \
                 `ttml_subtitle::Document::parse_str` + \
                 `Validator::new(Profile::Text)` directly -- no conversion \
                 or wrapper needed here",
    },
    MatrixEntry {
        from: TtmlImsc,
        to: ImscImage,
        support: NotImplemented,
        reason: "converting a Text Profile document to Image Profile would \
                 need the same rendering pipeline as \
                 DvbBitmapSubtitle -> ImscImage (text -> raster); a \
                 document already claiming Image Profile is the identity \
                 case above (TtmlImsc -> ImscText) and needs no conversion \
                 either -- not attempted this cut",
    },
];

/// Find the [`MatrixEntry`] for `(from, to)`. Every pair in the
/// [`SourceFormat`] x [`TargetFormat`] cross-product has exactly one entry
/// (`tests::matrix_is_a_complete_cross_product` pins this), so this never
/// panics in practice.
#[must_use]
pub fn lookup(from: SourceFormat, to: TargetFormat) -> &'static MatrixEntry {
    MATRIX
        .iter()
        .find(|e| e.from == from && e.to == to)
        .expect("MATRIX covers the full SourceFormat x TargetFormat cross-product")
}

/// Check whether `(from, to)` is implemented (`Lossless` or `Lossy`).
/// Returns the matrix entry on success; on `Unsupported`/`NotImplemented`
/// returns [`Error::Unsupported`] carrying the same reason a caller would
/// find in [`MATRIX`] -- so a generic caller never has to guess why a
/// conversion silently produced nothing, because it cannot silently produce
/// nothing: this check (or the total absence of a function for that pair)
/// is the gate.
pub fn check(from: SourceFormat, to: TargetFormat) -> Result<&'static MatrixEntry, Error> {
    let entry = lookup(from, to);
    match entry.support {
        Support::Lossless | Support::Lossy => Ok(entry),
        Support::Unsupported | Support::NotImplemented => Err(Error::Unsupported {
            from,
            to,
            support: entry.support,
            reason: entry.reason,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    const ALL_SOURCES: [SourceFormat; 5] = [Cea608, Cea708, Teletext, DvbBitmapSubtitle, TtmlImsc];
    const ALL_TARGETS: [TargetFormat; 4] = [WebVtt, Srt, ImscText, ImscImage];

    #[test]
    fn matrix_is_a_complete_cross_product() {
        assert_eq!(MATRIX.len(), ALL_SOURCES.len() * ALL_TARGETS.len());
        for from in ALL_SOURCES {
            for to in ALL_TARGETS {
                let matches = MATRIX
                    .iter()
                    .filter(|e| e.from == from && e.to == to)
                    .count();
                assert_eq!(matches, 1, "expected exactly one entry for {from} -> {to}");
            }
        }
    }

    #[test]
    fn every_reason_is_populated() {
        for e in MATRIX {
            assert!(
                !e.reason.is_empty(),
                "{} -> {} has an empty reason",
                e.from,
                e.to
            );
        }
    }

    #[test]
    fn check_ok_for_lossy_and_lossless() {
        assert!(check(Cea608, WebVtt).is_ok());
        assert!(check(TtmlImsc, ImscText).is_ok());
    }

    #[test]
    fn check_errs_for_unsupported_and_not_implemented() {
        let err = check(DvbBitmapSubtitle, WebVtt).unwrap_err();
        match err {
            Error::Unsupported {
                support: Support::Unsupported,
                ..
            } => {}
            other => panic!("expected Unsupported, got {other:?}"),
        }
        let err = check(DvbBitmapSubtitle, ImscImage).unwrap_err();
        match err {
            Error::Unsupported {
                support: Support::NotImplemented,
                ..
            } => {}
            other => panic!("expected NotImplemented, got {other:?}"),
        }
    }

    #[test]
    fn display_matches_name() {
        assert_eq!(Cea608.to_string(), "cea-608");
        assert_eq!(WebVtt.to_string(), "webvtt");
        assert_eq!(Lossy.to_string(), "lossy");
    }
}
