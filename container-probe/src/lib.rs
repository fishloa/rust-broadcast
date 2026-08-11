//! Robust media container-format detection over a byte prefix — issue
//! [#960](https://github.com/fishloa/rust-broadcast/issues/960).
//!
//! Identifies a container/stream format from a caller-owned byte slice (a file
//! prefix, a network read) — one-shot, no IO, no state. The design is
//! `docs/superpowers/specs/2026-08-11-container-probe-design.md`.
//!
//! Every registered prober runs over the same buffer and scores its candidate
//! (`Evidence`); the highest score wins. A single best match is
//! `Probe::Identified`, two within `TIE_THRESHOLD` is `Probe::Ambiguous`,
//! nothing conclusive with no hope of progress is `Probe::Unknown`, and a
//! match that a longer buffer could confirm reports
//! `Probe::Insufficient { need_at_least }`.
//!
//! # Work-package 1 scope
//!
//! The core types, the confidence/scoring model, the dispatch harness, and the
//! MPEG-2 TS prober ([`ts`]). The remaining probers (ISOBMFF, Matroska/WebM,
//! MPEG-PS, FLV, WAV/Ogg/ASF, ADTS-MP3-AnnexB elementary streams) land in later
//! work packages; adding one is a one-line registry change (see `PROBERS`).
//!
//! `no_std` + `alloc`.

#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

pub mod ts;

use alloc::vec::Vec;

/// The identified container/stream format.
///
/// Each variant maps to exactly one prober module. `#[non_exhaustive]` so
/// adding a format is a minor bump.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// MPEG-2 Transport Stream — ISO/IEC 13818-1, 188-byte packets.
    MpegTs,
    /// ISO Base Media File Format (`.mp4`/`.mov`/CMAF) — ISO/IEC 14496-12.
    Isobmff,
    /// MPEG Program Stream — ISO/IEC 13818-1 §2.5.
    MpegPs,
    /// Matroska — EBML container.
    Matroska,
    /// WebM — an EBML container with `DocType "webm"`.
    WebM,
    /// FLV — Adobe Flash Video, File Format Specification v10.1.
    Flv,
    /// Material Exchange Format — SMPTE ST 377-1.
    Mxf,
    /// RIFF/WAVE — Microsoft WAV.
    Wav,
    /// Ogg — Xiph Ogg container.
    Ogg,
    /// ASF — Advanced Systems Format.
    Asf,
    /// Raw ADTS AAC elementary stream — ISO/IEC 13818-7.
    AdtsAac,
    /// Raw MPEG-1/2 Layer III (MP3) elementary stream — ISO/IEC 11172-3.
    Mp3,
    /// Annex B NAL-unit byte stream — ITU-T H.264/H.265.
    AnnexB,
}

impl Format {
    /// The human-readable name of the format.
    pub fn name(&self) -> &'static str {
        match self {
            Format::MpegTs => "MpegTs",
            Format::Isobmff => "Isobmff",
            Format::MpegPs => "MpegPs",
            Format::Matroska => "Matroska",
            Format::WebM => "WebM",
            Format::Flv => "Flv",
            Format::Mxf => "Mxf",
            Format::Wav => "Wav",
            Format::Ogg => "Ogg",
            Format::Asf => "Asf",
            Format::AdtsAac => "AdtsAac",
            Format::Mp3 => "Mp3",
            Format::AnnexB => "AnnexB",
        }
    }
}

broadcast_common::impl_spec_display!(Format);

/// Evidence strength behind a match, in named tiers (see the `TIER_*`
/// constants and the crate-root confidence model).
///
/// The score is set once by the prober that produced the candidate; the
/// harness only compares them. A higher value is stronger evidence. The value
/// is opaque; read it with [`Confidence::as_u8`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Confidence(u8);

impl Confidence {
    /// The raw score.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self.0
    }
}

/// What a prober learned on the way to its conclusion — the difference between
/// "it is TS" and "it is TS, 192-byte stride, first sync at offset 4".
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    /// MPEG-2 TS lattice: packet `stride` (188/192/204/208) and the byte
    /// offset of the first sync (`phase`), so a demuxer need not re-derive them.
    Ts {
        /// Byte distance between consecutive sync bytes (a packet length).
        stride: u16,
        /// Byte offset of the first sync byte in the probed region.
        phase: u16,
    },
    /// ISOBMFF: the `ftyp` major brand (if seen) and how many top-level boxes
    /// chained cleanly.
    Isobmff {
        /// The 4-character code from the `ftyp` major brand, if one was read.
        major_brand: Option<[u8; 4]>,
        /// Number of top-level boxes that chained cleanly into the buffer.
        boxes_walked: u8,
    },
    /// Matroska/WebM: the EBML header's `DocType` string.
    Ebml {
        /// The EBML DocType.
        doc_type: DocType,
    },
    /// No format-specific detail to report.
    None,
}

impl Detail {
    /// The human-readable name of the detail variant.
    pub fn name(&self) -> &'static str {
        match self {
            Detail::Ts { .. } => "Ts",
            Detail::Isobmff { .. } => "Isobmff",
            Detail::Ebml { .. } => "Ebml",
            Detail::None => "None",
        }
    }
}

broadcast_common::impl_spec_display!(Detail);

/// The EBML `DocType` string (`"webm"` or `"matroska"`) decoded from an EBML
/// header's `EBMLDocType` element.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocType {
    /// `DocType == "webm"` (WebM container).
    Webm,
    /// `DocType == "matroska"` (Matroska container).
    Matroska,
    /// Any other, unrecognised `DocType` string.
    Other,
}

impl DocType {
    /// The human-readable name of the DocType.
    pub fn name(&self) -> &'static str {
        match self {
            DocType::Webm => "Webm",
            DocType::Matroska => "Matroska",
            DocType::Other => "Other",
        }
    }
}

broadcast_common::impl_spec_display!(DocType);

/// One scored candidate: a format, the evidence strength behind it, and what
/// the prober learned on the way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    /// The candidate container/stream format.
    pub format: Format,
    /// The evidence strength behind this candidate.
    pub confidence: Confidence,
    /// Prober-specific detail.
    pub detail: Detail,
}

/// What the probe concluded.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// A single best match.
    Identified {
        /// The winning format.
        format: Format,
        /// Its evidence strength.
        confidence: Confidence,
        /// Prober detail.
        detail: Detail,
    },
    /// Two or more candidates within `TIE_THRESHOLD`, ordered by descending
    /// confidence. A caller that wants a decision takes the first; a caller
    /// that wants correctness refuses.
    Ambiguous {
        /// The tied candidates, best first.
        candidates: Vec<Candidate>,
    },
    /// Nothing matched, but a longer buffer could change that.
    Insufficient {
        /// The minimum buffer length that could plausibly resolve a match.
        need_at_least: usize,
    },
    /// Nothing matched and more bytes will not help.
    Unknown,
}

impl Probe {
    /// The human-readable name of the probe outcome variant.
    pub fn name(&self) -> &'static str {
        match self {
            Probe::Identified { .. } => "Identified",
            Probe::Ambiguous { .. } => "Ambiguous",
            Probe::Insufficient { .. } => "Insufficient",
            Probe::Unknown => "Unknown",
        }
    }
}

broadcast_common::impl_spec_display!(Probe);

impl Probe {
    /// The `Confidence` of the winning candidate, if any — a convenience for
    /// the common `Identified` / `Ambiguous` first-wins read.
    ///
    /// Returns `None` for `Insufficient`/`Unknown`.
    #[must_use]
    pub fn best_confidence(&self) -> Option<Confidence> {
        match self {
            Probe::Identified { confidence, .. } => Some(*confidence),
            Probe::Ambiguous { candidates } => candidates.first().map(|c| c.confidence),
            Probe::Insufficient { .. } | Probe::Unknown => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Confidence model — named tiers, no bare numbers (design §"Confidence model").
// ---------------------------------------------------------------------------

/// Unambiguous magic at a defined offset **plus** a structural check confirming
/// it (e.g. EBML magic + valid DocType, MXF partition key + well-formed BER).
const TIER_CERTAIN: u8 = 240;
/// Unambiguous magic at a defined offset, no further validation available
/// (FLV signature, `RIFF`…`WAVE`, `OggS`, ASF header GUID).
const TIER_STRONG: u8 = 192;
/// A validated structure chain, not merely a signature (an ISOBMFF box chain
/// whose sizes fit the buffer; a MPEG-PS pack header with valid marker bits).
const TIER_STRUCTURAL: u8 = 160;
/// A repeating sync lattice with many confirmations (`>= 8` TS syncs at a
/// consistent stride).
const TIER_LATTICE_STRONG: u8 = 144;
/// A repeating lattice with few confirmations (3-7 TS syncs).
const TIER_LATTICE_WEAK: u8 = 96;
/// A signature with meaningful false-positive probability (bare MPEG-PS pack
/// start code, bare AnnexB start code).
const TIER_HEURISTIC: u8 = 64;
/// Two candidates whose scores are within this gap are reported as
/// `Probe::Ambiguous` rather than silently choosing one.
const TIE_THRESHOLD: u8 = 16;
/// Default byte budget for `probe` — comfortably above the worst case (a
/// 208-byte-stride TS lattice needing 8 confirmations plus a full phase
/// search). Design §Performance.
const DEFAULT_BUDGET: usize = 64 * 1024;

/// The named tiers, exposed for tests. A prober assigns one of these (e.g.
/// [`Confidence::LATTICE_STRONG`]).
impl Confidence {
    /// Unambiguous magic **plus** a structural confirmation (240).
    pub const CERTAIN: Confidence = Confidence(TIER_CERTAIN);
    /// Unambiguous magic at a defined offset only (192).
    pub const STRONG: Confidence = Confidence(TIER_STRONG);
    /// A validated structure chain (160).
    pub const STRUCTURAL: Confidence = Confidence(TIER_STRUCTURAL);
    /// `>= 8` lattice confirmations at a consistent stride (144).
    pub const LATTICE_STRONG: Confidence = Confidence(TIER_LATTICE_STRONG);
    /// 3-7 lattice confirmations (96).
    pub const LATTICE_WEAK: Confidence = Confidence(TIER_LATTICE_WEAK);
    /// A signature with meaningful false-positive probability (64).
    pub const HEURISTIC: Confidence = Confidence(TIER_HEURISTIC);
}

/// A scored match from one prober. Internal; `Candidate` is the public form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Evidence {
    confidence: Confidence,
    detail: Detail,
}

/// What one prober concluded about its format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// The format matched with this evidence.
    Match(Evidence),
    /// The buffer may resolve with at least this many bytes.
    Insufficient(usize),
    /// No match, and more bytes will not change that.
    None,
}

/// A registered prober. Each prober is a pure function over a slice read no
/// further than `limit` bytes.
type Prober = fn(&[u8], limit: usize) -> Outcome;

/// The registry of probers, in declaration order. **Adding a prober for a new
/// format is a one-line change here**: append `(Format::X, x::probe)` and the
/// harness picks it up. Order does not bias the result — all probers always
/// run and the highest score wins.
const PROBERS: &[(Format, Prober)] = &[(Format::MpegTs, ts::probe)];

/// Probe with the default budget (`DEFAULT_BUDGET`).
pub fn probe(data: &[u8]) -> Probe {
    probe_with_budget(data, DEFAULT_BUDGET)
}

/// Probe reading at most `budget` bytes of `data`.
///
/// Every registered prober runs over the same bytes and scores its candidate;
/// the highest score wins. If the top two are within `TIE_THRESHOLD` the
/// result is `Ambiguous` (ordered by score); otherwise `Identified`. With no
/// candidates, `Insufficient` if any prober could conclude from more bytes
/// (reporting the smallest such `need_at_least`), else `Unknown`.
pub fn probe_with_budget(data: &[u8], budget: usize) -> Probe {
    // Read no further than this; `budget` may exceed the buffer.
    let limit = core::cmp::min(data.len(), budget);

    let mut candidates: Vec<Candidate> = Vec::new();
    let mut need_more: Option<usize> = None;

    for (format, prober) in PROBERS {
        match prober(data, limit) {
            Outcome::Match(ev) => candidates.push(Candidate {
                format: *format,
                confidence: ev.confidence,
                detail: ev.detail,
            }),
            Outcome::Insufficient(need) => {
                need_more = Some(match need_more {
                    Some(prev) => core::cmp::min(prev, need),
                    None => need,
                });
            }
            Outcome::None => {}
        }
    }

    match candidates.len() {
        0 => match need_more {
            Some(need) => Probe::Insufficient {
                need_at_least: core::cmp::max(need, 1),
            },
            None => Probe::Unknown,
        },
        _ => {
            // Descending confidence, then a stable format-name tie-break.
            candidates.sort_by(|a, b| {
                b.confidence
                    .cmp(&a.confidence)
                    .then_with(|| a.format.name().cmp(b.format.name()))
            });
            probe_to_identify(&candidates)
        }
    }
}

fn probe_to_identify(candidates: &[Candidate]) -> Probe {
    let top = &candidates[0];
    if let Some(second) = candidates.get(1) {
        // Scores are within `TIE_THRESHOLD` -> genuinely ambiguous, list all.
        if top
            .confidence
            .as_u8()
            .saturating_sub(second.confidence.as_u8())
            <= TIE_THRESHOLD
        {
            return Probe::Ambiguous {
                candidates: candidates.to_vec(),
            };
        }
    }
    Probe::Identified {
        format: top.format,
        confidence: top.confidence,
        detail: top.detail,
    }
}
