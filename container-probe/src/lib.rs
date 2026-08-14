//! Robust media **container-format detection** over a caller-owned byte slice.
//!
//! `container-probe` identifies whether a byte prefix (a file's leading bytes,
//! a network read) is an MPEG-2 Transport Stream, an ISOBMFF file, a
//! Matroska/WebM file, MXF, MPEG—PS, FLV, WAV, Ogg, ASF, or a raw ADTS AAC /
//! MP3 / Annex B elementary stream. One-shot over a slice: the crate holds no
//! buffer, owns no IO, and keeps no state.
//!
//! # Detected formats
//!
//! | Format | How it is detected | Best confidence tier |
//! |---|---|---|
//! | MPEG-2 TS | sync lattice over 188/192/204/208-byte strides | `LATTICE_STRONG` |
//! | ISOBMFF | box-chain walk (ISO/IEC 14496-12 §4.2) | `STRUCTURAL` |
//! | Matroska/WebM | EBML magic + `DocType` | `CERTAIN` |
//! | MXF | partition-pack key + BER length | `CERTAIN` |
//! | MPEG-PS | pack start code + marker bits | `STRUCTURAL` |
//! | FLV | `"FLV"` signature + header fields | `STRONG` |
//! | WAV | `"RIFF".."WAVE"` | `STRONG` |
//! | Ogg | `"OggS"` | `STRONG` |
//! | ASF | 16-byte header GUID | `STRONG` |
//! | ADTS AAC | frame-length chain | `LATTICE_STRONG` |
//! | MP3 | frame-length chain | `LATTICE_STRONG` |
//! | Annex B (H.264) | start-code NAL chain | `LATTICE_STRONG` |
//!
//! # Known gaps
//!
//! - **Annex B detection is H.264 only.** HEVC (H.265) NAL units use a 2-byte
//!   header with `nal_unit_type` at bits `[6:1]`, which this prober does not
//!   parse — it validates the H.264 1-byte header, so an HEVC stream fails the
//!   range check at the first NAL. HEVC is deliberately not implemented: this
//!   workspace does not implement a format without a real fixture to test it
//!   against, and no HEVC Annex B fixture exists in the repository.
//!
//! # The scored confidence model
//!
//! Every registered prober runs over the same bytes and returns a scored
//! candidate (an `Evidence`): a `Confidence` tier and a `Detail`. All probers
//! **always** run, so the answer does not depend on declaration order. The
//! highest score wins:
//!
//! | Tier | Value | Meaning |
//! |---|---|---|
//! | `CERTAIN` | 240 | Magic **plus** a structural confirmation |
//! | `STRONG` | 192 | Unambiguous magic at a defined offset |
//! | `STRUCTURAL` | 160 | A validated structure chain |
//! | `LATTICE_STRONG` | 128 | `>= 8` lattice/frame confirmations |
//! | `LATTICE_WEAK` | 96 | 3-7 lattice/frame confirmations |
//! | `HEURISTIC` | 64 | A signature with real false-positive risk |
//!
//! If the top two candidates are within `TIE_THRESHOLD` (16), the result is
//! [`Probe::Ambiguous`] with every candidate listed — never an arbitrary pick.
//! A container matched at `LATTICE_STRONG` or above zeroes every
//! elementary-stream candidate (ADTS/MP3/Annex B), on the principle that ES
//! frames inside a container payload are expected data, not evidence the file
//! is a raw stream.
//!
//! # `Insufficient` vs `Unknown`
//!
//! These are deliberately distinct, and a caller must not conflate them:
//!
//! - [`Probe::Insufficient { need_at_least }`](Probe) means **read more bytes** —
//!   nothing conclusive matched yet, but a longer buffer could change that. Use
//!   `need_at_least` to decide how much more to buffer.
//! - [`Probe::Unknown`] means **stop** — nothing matched and more bytes will not
//!   help.
//!
//! ## The loop that terminates
//!
//! `need_at_least` is guaranteed to exceed the number of bytes the probe
//! actually **examined**, which is `min(len, budget)` — not `len`. So a caller
//! that only grows the buffer, while leaving [`probe`]'s fixed
//! `DEFAULT_BUDGET` in place, can stall once the budget caps the read. Feed
//! `need_at_least` back as the budget too:
//!
//! ```no_run
//! # use container_probe::{probe_with_budget, Probe, DEFAULT_BUDGET};
//! # fn read_at_least(_n: usize) -> Vec<u8> { Vec::new() }
//! let mut buf = read_at_least(DEFAULT_BUDGET);
//! let verdict = loop {
//!     match probe_with_budget(&buf, buf.len()) {
//!         Probe::Insufficient { need_at_least, .. } if need_at_least > buf.len() => {
//!             let grown = read_at_least(need_at_least);
//!             // EOF with nothing conclusive: stop. The probe cannot answer
//!             // from this file, and no further read will change that.
//!             if grown.len() <= buf.len() {
//!                 break Probe::Unknown;
//!             }
//!             buf = grown;
//!         }
//!         other => break other,
//!     }
//! };
//! # let _ = verdict;
//! ```
//!
//! Passing `buf.len()` as the budget is what makes each turn examine ground the
//! last one did not. The `grown.len() <= buf.len()` arm is the caller's own
//! termination guarantee at EOF, and is not optional: the crate cannot know
//! whether more bytes exist.
//!
//! # `no_std` + `alloc`
//!
//! The crate is `#![no_std]` and links only `alloc`. Its single runtime
//! dependency is `broadcast-common`. The only allocation is the candidate `Vec`
//! of a genuine `Ambiguous` result. Build it without the default features for a
//! pure-`alloc` target.
//!
//! # Example
//!
//! ```
//! use container_probe::{probe, Probe};
//!
//! // An empty slice is `Insufficient`: reading more bytes could make it any
//! // registered format, so a caller must not stop — it must read more. This is
//! // the honest contract for a buffer that has not yet ruled anything out.
//! let p = probe(&[]);
//! match p {
//!     Probe::Insufficient { need_at_least, .. } => assert!(need_at_least >= 1),
//!     _ => unreachable!("an empty slice cannot be concluded from"),
//! }
//! ```
//!
//! # Non-goals
//!
//! - **No demuxing** — the probe identifies a format; parsing its content is a
//!   demuxer's job.
//! - **No codec identification** — "this is TS" is the answer, not "this TS
//!   carries H.264".
//! - **No file IO** — `no_std`; the caller supplies bytes.
//! - **No format conversion or repair** — identification only.
//! - **No incremental/streaming API** — the probe is one-shot over a slice; a
//!   streaming caller reads more and re-probes, guided by `Insufficient`'s
//!   `need_at_least`.
//!
//! `no_std` + `alloc`; runtime dependency is `broadcast-common` only.

#![no_std]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
// `container-probe` README is the canonical quick-start surface; `readme =
// "README.md"` in Cargo.toml makes docs.rs/crates.io render it, and this line
// also feeds its `rust` code blocks to rustdoc as doctests. The `doctest` cfg is
// set by rustdoc only when collecting doctests, so the README is compiled as
// doctests (never part of the build), which is what keeps the headline example
// from silently rotting (it once matched all four `Probe` variants with no
// wildcard arm, an E0004 for any downstream consumer, and nothing caught it).
#![cfg_attr(doctest, doc = include_str!("../README.md"))]

extern crate alloc;

// Unit tests in the prober modules need `std` for fixture file IO; the test
// build links it even though the crate is `#![no_std]`.
#[cfg(test)]
extern crate std;

mod adts;
mod annexb;
mod asf;
mod ebml;
mod flv;
mod isobmff;
mod mp3;
mod mpegps;
mod mxf;
mod ogg;
mod riff;
mod ts;

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
    /// Annex B NAL-unit byte stream — ITU-T H.264 only (HEVC is not detected;
    /// see the crate-root "Known gaps").
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

    /// `true` when this format is an elementary (packetised payload) stream
    /// that a strong container match should suppress — ADTS AAC, MP3, and
    /// Annex B. These carry no container framing of their own and routinely
    /// appear *inside* a container's payload, so a container verdict at
    /// `LATTICE_STRONG` or above must outvote them (see
    /// `suppress_elementary_streams`).
    pub fn is_elementary_stream(&self) -> bool {
        matches!(self, Format::AdtsAac | Format::Mp3 | Format::AnnexB)
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

    /// The human-readable name of this confidence tier (the `TIER_*` table).
    pub fn name(&self) -> &'static str {
        match self.0 {
            TIER_CERTAIN => "CERTAIN",
            TIER_STRONG => "STRONG",
            TIER_STRUCTURAL => "STRUCTURAL",
            TIER_LATTICE_STRONG => "LATTICE_STRONG",
            TIER_LATTICE_WEAK => "LATTICE_WEAK",
            TIER_HEURISTIC => "HEURISTIC",
            _ => "unknown",
        }
    }
}

broadcast_common::impl_spec_display!(Confidence);

/// How an ISOBMFF file carries its sample metadata — the discriminator a
/// consumer needs to choose a demuxer.
///
/// ISO/IEC 14496-12 allows both shapes and they are demuxed differently: a
/// fragmented file's samples are described by `moof` movie fragments (§8.8,
/// the CMAF/fMP4 shape), a progressive file's by `moov` sample tables (§8.7).
/// The prober walks the top-level boxes anyway, so it reports what it saw
/// rather than making every consumer re-walk the chain to find out — the same
/// reason [`Detail::Ts`] carries the stride and phase it measured.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsobmffLayout {
    /// A `moof` box was seen: fragmented (CMAF / fMP4 / DASH segment).
    Fragmented,
    /// A `moov` box was seen and no `moof`: progressive, with sample tables.
    Progressive,
    /// Neither was seen within the probed region — e.g. a buffer holding only
    /// `ftyp` and the start of a large `mdat`. The file is ISOBMFF, but which
    /// shape is undetermined from these bytes; a consumer should read further
    /// rather than assume.
    Unknown,
}

impl IsobmffLayout {
    /// Stable label.
    pub fn name(&self) -> &'static str {
        match self {
            IsobmffLayout::Fragmented => "fragmented",
            IsobmffLayout::Progressive => "progressive",
            IsobmffLayout::Unknown => "unknown",
        }
    }
}

broadcast_common::impl_spec_display!(IsobmffLayout);

/// What a prober learned on the way to its conclusion — the difference between
/// "it is TS" and "it is TS, 192-byte stride, first sync at offset 4".
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    /// MPEG-2 TS lattice: packet `stride` (188/192/204/208) and the byte
    /// offset of the first sync (`phase`), so a demuxer need not re-derive them.
    #[non_exhaustive]
    Ts {
        /// Byte distance between consecutive sync bytes (a packet length).
        stride: u16,
        /// Byte offset of the first sync byte in the probed region.
        phase: u16,
    },
    /// ISOBMFF: the `ftyp` major brand (if seen) and how many top-level boxes
    /// chained cleanly.
    #[non_exhaustive]
    Isobmff {
        /// The 4-character code from the `ftyp` major brand, if one was read.
        major_brand: Option<[u8; 4]>,
        /// Number of top-level boxes that chained cleanly into the buffer.
        boxes_walked: u8,
        /// Which structural layout the top-level walk observed — the
        /// discriminator between a fragmented and a progressive file.
        layout: IsobmffLayout,
    },
    /// Matroska/WebM: the EBML header's `DocType` string.
    #[non_exhaustive]
    Ebml {
        /// The EBML DocType.
        doc_type: DocType,
    },
    /// FLV: the header's `TypeFlags` (which tag types are present) and the
    /// `DataOffset` header size.
    #[non_exhaustive]
    Flv {
        /// `TypeFlags` bit 0 — audio tags present.
        has_audio: bool,
        /// `TypeFlags` bit 2 — video tags present.
        has_video: bool,
        /// The `DataOffset` field (bytes 5..9), the header size.
        data_offset: u32,
    },
    /// MXF: the decoded Partition Pack `PartitionKind` byte (byte 14 of the UL).
    #[non_exhaustive]
    Mxf {
        /// The Partition Kind byte (`0x02` Header / `0x03` Body / `0x04` Footer).
        partition_kind: u8,
    },
    /// MPEG-PS: whether the pack header's SCR/mux-rate marker bits validated.
    #[non_exhaustive]
    MpegPs {
        /// `true` when the SCR `'01'` prefix, the four SCR marker bits and the
        /// two `program_mux_rate` marker bits all validated (`STRUCTURAL`);
        /// `false` when only the pack start code matched (`HEURISTIC`).
        structurally_valid: bool,
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
            Detail::Flv { .. } => "Flv",
            Detail::Mxf { .. } => "Mxf",
            Detail::MpegPs { .. } => "MpegPs",
            Detail::None => "None",
        }
    }

    /// The `ftyp`/`styp` major brand as a string, when the identifying file
    /// carried one (an ISOBMFF result). The brand is a registered 4-character
    /// code, so this is the ergonomic view of the raw `[u8; 4]` in
    /// [`Detail::Isobmff`].
    ///
    /// Returns `None` when the detail is not `Isobmff`, when no brand was
    /// observed, or when the 4 bytes are not valid UTF-8.
    #[must_use]
    pub fn major_brand_str(&self) -> Option<&str> {
        if let Detail::Isobmff { major_brand, .. } = self {
            // `major_brand` is a reference into `self` (match ergonomics on the
            // connected `&self`), so the returned `&str` borrows from `self`.
            let bytes: &[u8; 4] = major_brand.as_ref()?;
            core::str::from_utf8(&bytes[..]).ok()
        } else {
            None
        }
    }
}

/// A lossless `Display` for [`Detail`]: every data-bearing variant renders its
/// fields, so `Detail::Ts { stride, phase }` is not collapsed to just `"Ts"`.
/// `Display` delegates to [`Detail::name`] (the #204 convention) for the label,
/// then appends the field data the variant actually carries.
impl core::fmt::Display for Detail {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Detail::Ts { stride, phase } => {
                write!(f, "Ts {{ stride: {stride}, phase: {phase} }}")
            }
            Detail::Isobmff {
                major_brand,
                boxes_walked,
                layout,
            } => {
                let brand = major_brand
                    .map(|b| alloc::string::String::from_utf8_lossy(&b[..]).into_owned())
                    .unwrap_or_else(|| "<none>".into());
                write!(
                    f,
                    "Isobmff {{ major_brand: {brand:?}, boxes_walked: {boxes_walked}, layout: {layout} }}"
                )
            }
            Detail::Ebml { doc_type } => write!(f, "Ebml {{ doc_type: {doc_type} }}"),
            Detail::Flv {
                has_audio,
                has_video,
                data_offset,
            } => write!(
                f,
                "Flv {{ has_audio: {has_audio}, has_video: {has_video}, data_offset: {data_offset} }}"
            ),
            Detail::Mxf { partition_kind } => {
                write!(f, "Mxf {{ partition_kind: 0x{partition_kind:02X} }}")
            }
            Detail::MpegPs { structurally_valid } => {
                write!(f, "MpegPs {{ structural: {structurally_valid} }}")
            }
            Detail::None => f.write_str("None"),
        }
    }
}

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
///
/// `#[non_exhaustive]` so adding a field is not a breaking change.
#[non_exhaustive]
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
    #[non_exhaustive]
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
    #[non_exhaustive]
    Ambiguous {
        /// The tied candidates, best first.
        candidates: Vec<Candidate>,
    },
    /// Nothing matched, but a longer buffer could change that.
    #[non_exhaustive]
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
const TIER_LATTICE_STRONG: u8 = 128;
/// A repeating lattice with few confirmations (3-7 TS syncs).
const TIER_LATTICE_WEAK: u8 = 96;
/// A signature with meaningful false-positive probability (bare MPEG-PS pack
/// start code, bare AnnexB start code).
const TIER_HEURISTIC: u8 = 64;
/// Two candidates whose scores are within this gap are reported as
/// `Probe::Ambiguous` rather than silently choosing one.
const TIE_THRESHOLD: u8 = 16;
/// Default byte budget for [`probe`] — comfortably above the worst case (a
/// 208-byte-stride TS lattice needing 8 confirmations plus a full phase
/// search). Design §Performance.
///
/// Public because a caller draining an [`Probe::Insufficient`] loop needs it:
/// `probe` never examines more than this many bytes however long the buffer
/// is, so growing the buffer alone can stall. See the crate-root "The loop
/// that terminates".
pub const DEFAULT_BUDGET: usize = 64 * 1024;

/// The named tiers, exposed for tests. A prober assigns one of these (e.g.
/// [`Confidence::LATTICE_STRONG`]).
impl Confidence {
    /// Unambiguous magic **plus** a structural confirmation (240).
    pub const CERTAIN: Confidence = Confidence(TIER_CERTAIN);
    /// Unambiguous magic at a defined offset only (192).
    pub const STRONG: Confidence = Confidence(TIER_STRONG);
    /// A validated structure chain (160).
    pub const STRUCTURAL: Confidence = Confidence(TIER_STRUCTURAL);
    /// `>= 8` lattice confirmations at a consistent stride (128).
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

/// The single shared decision every prober must make: is "no match" a *proof of
/// non-membership* — or merely the walk *running off the end of the supplied
/// region* mid-structure?
///
/// This is the whole `Insufficient` vs `Unknown` contract in one place. The
/// [`Probe::Unknown`] branch means "stop, more bytes will not help", which is
/// only ever true when the prober examined data it *fully saw* and that data is
/// invalid for the format. If the prober would have kept walking had the buffer
/// been longer, it has proven nothing and must answer [`Outcome::Insufficient`]
/// with `need` — a lower bound on bytes that could change the verdict.
///
/// `ran_out` signals which of the two happened: `true` when the prober stopped
/// because the region ended mid-structure (→ [`Outcome::Insufficient`], "read
/// more"), `false` when it examined the data and ruled the format out
/// (→ [`Outcome::None`], "stop"). Re-deciding this judgement separately in
/// each of the twelve probers is what let the contract break repeatedly at
/// different lengths on different formats; it lives here so the only question a
/// prober answers is the factual one — "did I run out of bytes?" — never the
/// policy one.
pub(crate) fn ran_out_or_ruled_out(ran_out: bool, need: usize) -> Outcome {
    if ran_out {
        Outcome::Insufficient(need)
    } else {
        Outcome::None
    }
}

/// Force a prober's `need_at_least` to exceed the bytes actually examined.
///
/// `Insufficient` promises the caller that reading more can change the answer.
/// A `need` at or below `limit` breaks that promise: the caller re-probes,
/// examines the same ground, gets the same answer, and never advances.
///
/// Enforced here, once, rather than trusted from twelve probers — the crate has
/// shipped this defect twice, from two different probers, with a green suite
/// both times.
///
/// A prober's own structural need is kept when it is larger; that is the more
/// useful hint (it names the byte the structure actually reaches, so one read
/// suffices instead of one-byte-at-a-time crawling).
///
/// This is a **backstop**. Every prober is currently expected to report a
/// structural need, so in normal operation this returns `need` unchanged. It is
/// separated out rather than inlined so it can be tested directly: a guard with
/// no reachable failing input is a guard nobody can trust.
fn normalise_need(need: usize, limit: usize) -> usize {
    core::cmp::max(need, geometric_floor(limit))
}

/// The smallest answer a prober with **no structural need** may give.
///
/// Half again as much as was examined, plus one. Strictly greater than `limit`,
/// so the caller always advances, and *geometric*, so a caller that cannot name
/// a structure still converges in O(log n) reads rather than O(n).
///
/// The arithmetic alternative is what shipped and had to be undone: every
/// prober computed `max(structural_floor, have + unit)`, and once `have`
/// overtook the floor the second term won, growing by one unit per turn —
/// `+4` for Annex B, `+188` for TS. The documented caller loop then reached
/// 36 bytes of a 256 KiB file after twelve reads. Terminating and useless are
/// different things, and only a turn-count bound tells them apart.
///
/// Over-asking is safe for a lower bound because the caller clamps to EOF and
/// re-probes what it actually got: a file shorter than this figure is probed
/// whole on the next turn, so nothing that was decidable becomes undecidable.
fn geometric_floor(limit: usize) -> usize {
    limit.saturating_add(limit / 2).saturating_add(1)
}

/// A registered prober. Each prober is a pure function over a slice read no
/// further than `limit` bytes.
type Prober = fn(&[u8], limit: usize) -> Outcome;

/// The registry of probers, in declaration order. **Adding a prober for a new
/// format is a one-line change here**: append `(Format::X, x::probe)` and the
/// harness picks it up. Order does not bias the result — all probers always
/// run and the highest score wins.
///
/// One entry per format. The EBML prober is registered under `Format::Matroska`
/// and reports `Format::WebM` (or stays Matroska) via its `Detail::Ebml`
/// `DocType`, resolved by [`candidate_format`].
const PROBERS: &[(Format, Prober)] = &[
    (Format::MpegTs, ts::probe),
    (Format::Isobmff, isobmff::probe),
    (Format::Matroska, ebml::probe),
    (Format::Mxf, mxf::probe),
    (Format::MpegPs, mpegps::probe),
    (Format::Flv, flv::probe),
    (Format::Wav, riff::probe),
    (Format::Ogg, ogg::probe),
    (Format::Asf, asf::probe),
    (Format::AdtsAac, adts::probe),
    (Format::Mp3, mp3::probe),
    (Format::AnnexB, annexb::probe),
];

/// Resolve the final candidate `Format`, given a registry format and the
/// prober's `Detail`. Only the EBML prober ever disagrees: `Detail::Ebml` with
/// `DocType::Webm` is a WebM, anything else is a Matroska (the registry label).
fn candidate_format(format: Format, detail: Detail) -> Format {
    match detail {
        Detail::Ebml { doc_type } => match doc_type {
            DocType::Webm => Format::WebM,
            DocType::Matroska | DocType::Other => Format::Matroska,
        },
        _ => format,
    }
}

/// Cross-prober **suppression**: when any container matches at
/// `LATTICE_STRONG` (128) or above, every elementary-stream candidate is
/// dropped.
///
/// ADTS frames, MP3 frames, and Annex B NAL units routinely appear *inside* a
/// real container's payload — they are the expected data, not evidence the file
/// is raw elementary audio or video. A high-entropy container can also align
/// enough syncwords to score weakly, so an elementary-stream candidate is never
/// allowed to outvote or tie a genuine container. The container's high score is
/// exactly the proof that it is (also) a container; ruling out the ES reading
/// is the one-directional, named trade this function makes (design §"Confidence
/// model", "Cross-prober suppression"). This must never be an implicit ordering
/// a later edit can silently reverse — it is the explicit final transformation
/// before scoring.
fn suppress_elementary_streams(candidates: &mut Vec<Candidate>) {
    let container_wins = candidates
        .iter()
        .any(|c| !c.format.is_elementary_stream() && c.confidence.as_u8() >= TIER_LATTICE_STRONG);
    if container_wins {
        candidates.retain(|c| !c.format.is_elementary_stream());
    }
}

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
                format: candidate_format(*format, ev.detail),
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

    // Cross-prober suppression: a container matched strongly -> zero the
    // elementary-stream candidates before scoring (see
    // `suppress_elementary_streams`).
    suppress_elementary_streams(&mut candidates);

    match candidates.len() {
        0 => match need_more {
            // `need_at_least` MUST exceed the bytes actually examined, or the
            // contract is a lie and the documented caller loop never advances.
            //
            // This is enforced here, once, rather than trusted from twelve
            // probers, because the crate has now shipped this defect twice from
            // two different probers. `ebml` reported `region.len() + 1`, which
            // looks like strict progress but is not: `limit` is capped at
            // `budget`, so `region.len()` saturates at `DEFAULT_BUDGET` and the
            // answer froze at 65537. Supplying more than that gave
            // `need_at_least <= supplied` — a fixed point a caller obeying the
            // contract spins on forever. `mp3`'s ID3 skip reached the same
            // fixed point from an honest, structure-derived need that simply
            // exceeded the budget.
            //
            // Normalising to a geometric floor guarantees the number always asks for
            // ground not yet examined. A prober's own structural need is kept
            // when it is larger, since that is the more useful hint.
            Some(need) => Probe::Insufficient {
                need_at_least: normalise_need(need, limit),
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
        // Scores are within `TIE_THRESHOLD` -> genuinely ambiguous. List only
        // the candidates actually within `TIE_THRESHOLD` of the winner (the
        // tied set), not every lower-scored also-ran: `Probe::Ambiguous`
        // documents "two or more candidates within `TIE_THRESHOLD`", and a
        // candidate well below the winner is not part of that tie.
        if top
            .confidence
            .as_u8()
            .saturating_sub(second.confidence.as_u8())
            <= TIE_THRESHOLD
        {
            let tied: Vec<Candidate> = candidates
                .iter()
                .copied()
                .take_while(|c| {
                    top.confidence.as_u8().saturating_sub(c.confidence.as_u8()) <= TIE_THRESHOLD
                })
                .collect();
            return Probe::Ambiguous { candidates: tied };
        }
    }
    Probe::Identified {
        format: top.format,
        confidence: top.confidence,
        detail: top.detail,
    }
}

#[cfg(test)]
mod no_length_relative_needs {
    //! The class-closing guard: **no** prober may derive `need_at_least` from
    //! how many bytes it was handed.
    //!
    //! Four consecutive audit rounds each found a different prober doing this,
    //! because each round's fix was applied to whichever prober the auditor had
    //! probed and then claimed for the class. Hand-written per-prober tests
    //! repeat that mistake by construction: they cover the probers someone
    //! thought of. This walks [`PROBERS`] itself, so a prober added later is
    //! covered the day it is registered.
    //!
    //! The property: for a fixed seed padded to increasing lengths, any prober
    //! that answers `Insufficient` at two or more of those lengths must give
    //! the *same* number every time. The need names a structure, and the
    //! structure does not move because the caller supplied more zeros. A need
    //! that grows with the buffer still terminates -- and crawls, at one unit
    //! per read.

    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    /// Seeds chosen to drive different probers into their "read more" path:
    /// a leading magic or sync that commits the prober to looking further,
    /// followed by nothing that resolves it.
    fn seeds() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("ts-sync", vec![0x47]),
            (
                "ebml-magic",
                vec![
                    0x1A, 0x45, 0xDF, 0xA3, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE,
                ],
            ),
            (
                "isobmff-largesize",
                vec![0x00, 0x00, 0x00, 0x01, b'f', b't', b'y', b'p'],
            ),
            ("adts-sync", vec![0xFF, 0xF1, 0x4C, 0x80, 0x22, 0x3F, 0xFC]),
            ("mp3-sync", vec![0xFF, 0xFB, 0x90, 0x64]),
            (
                "id3",
                vec![b'I', b'D', b'3', 0x04, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00],
            ),
            ("annexb-startcode", vec![0x00, 0x00, 0x00, 0x01, 0x67]),
            ("riff", vec![b'R', b'I', b'F', b'F']),
            ("ogg", vec![b'O', b'g', b'g']),
            (
                "mxf-key",
                vec![0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01],
            ),
            ("ps-startcode", vec![0x00, 0x00, 0x01, 0xBA]),
            ("flv", vec![b'F', b'L', b'V']),
            (
                "asf-guid",
                vec![0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11],
            ),
        ]
    }

    #[test]
    fn no_prober_derives_its_need_from_the_buffer_length() {
        // Short lengths are load-bearing, not padding. Most probers answer
        // `Insufficient(CONST)` only while the region is SHORTER than the
        // constant they need (a 16-byte MXF key, a 12-byte RIFF header), so a
        // length set that starts at 4 never reaches them at two lengths and the
        // guard goes blind on 7 of 12 probers. The per-prober coverage
        // assertion below is what surfaced that.
        let lengths = [
            1usize, 2, 3, 5, 6, 7, 10, 12, 14, 15, 20, 64, 256, 1024, 4096,
        ];
        let mut failures: Vec<alloc::string::String> = Vec::new();
        let mut exercised = 0usize;
        let mut covered: Vec<&Format> = Vec::new();

        for (format, prober) in PROBERS {
            for (seed_name, seed) in seeds() {
                let mut seen: Vec<(usize, usize)> = Vec::new();
                for len in lengths {
                    if len < seed.len() {
                        continue;
                    }
                    let mut buf = seed.clone();
                    buf.resize(len, 0x00);
                    if let Outcome::Insufficient(need) = prober(&buf, buf.len()) {
                        seen.push((len, need));
                    }
                }
                if seen.len() < 2 {
                    continue;
                }
                exercised += 1;
                covered.push(format);
                let first = seen[0].1;
                if seen.iter().any(|(_, n)| *n != first) {
                    failures.push(alloc::format!(
                        "  {:?} on seed {seed_name}: {seen:?} -- the need moves with the \
                         buffer length, so a caller advances one unit per read",
                        format
                    ));
                }
            }
        }

        // Per-prober coverage, NOT an aggregate count. The first version of
        // this test asserted `exercised >= 6` across all pairs and was vacuous
        // for exactly the probers it was written for: reverting `ts` and `adts`
        // to length-relative needs left it green, because those two never
        // reached `Insufficient` at two lengths and other pairs made the total
        // look healthy. An aggregate threshold hides a per-prober hole -- which
        // is the same mistake as fixing one prober and claiming the class.
        let missed: Vec<&Format> = PROBERS
            .iter()
            .map(|(f, _)| f)
            .filter(|f| !covered.contains(f))
            .collect();
        assert!(
            missed.is_empty(),
            "no seed drives {missed:?} to Insufficient at two or more lengths, so this \
             guard cannot see a length-relative need in {} of {} probers. Add a seed \
             that reaches them rather than lowering the bar.",
            missed.len(),
            PROBERS.len()
        );
        let _ = exercised;
        assert!(
            failures.is_empty(),
            "a prober's need_at_least must name a structure, not the buffer:\n{}",
            failures.join("\n")
        );
    }
}

#[cfg(test)]
mod need_normalisation {
    use super::*;

    /// A prober under-reporting its need must never reach the caller as a
    /// promise that cannot be kept.
    ///
    /// The two shipped instances of this defect: `ebml` reported
    /// `region.len() + 1`, which saturates at the budget and froze at 65537;
    /// `mp3`'s ID3 skip reported an honest structural need that simply exceeded
    /// the budget. Both left `need_at_least <= supplied` for a large enough
    /// buffer — a fixed point.
    ///
    /// MUTATION VERIFIED: replacing the body with `need` fails the first case
    /// below with `left: 10, right: 65537`.
    #[test]
    fn an_under_reported_need_is_raised_past_the_examined_bytes() {
        // A prober that under-reports badly at a large limit is raised to the
        // geometric floor (limit + limit/2 + 1), not merely to limit + 1: an
        // arithmetic bump converges in O(n) reads, which terminates but crawls.
        assert_eq!(normalise_need(10, 65536), 98_305);
        // Exactly at the limit is still a fixed point, so it must be raised.
        assert_eq!(normalise_need(4096, 4096), 6145);
        // Just past the limit still crawls, so it is raised too.
        assert_eq!(normalise_need(4097, 4096), 6145);
        // A larger structural need is the better hint and must be preserved,
        // never clamped down to the floor.
        assert_eq!(normalise_need(268_435_465, 65536), 268_435_465);
        // Degenerate: zero need at zero limit must still ask for something.
        assert_eq!(normalise_need(0, 0), 1);
    }

    /// The invariant, stated over a range rather than at points, so a future
    /// formula that happens to satisfy the five cases above cannot pass.
    #[test]
    fn the_result_always_exceeds_the_limit() {
        for limit in [0usize, 1, 4, 188, 4095, 4096, 65535, 65536, 131_072] {
            for need in [0usize, 1, 4, 100, 65536, 65537, 1_000_000] {
                let got = normalise_need(need, limit);
                assert!(
                    got > limit,
                    "normalise_need({need}, {limit}) = {got}, which does not exceed the \
                     {limit} bytes examined -- a caller would re-probe forever"
                );
                // Geometric, not merely greater: an answer of `limit + 1` still
                // converges in O(n) reads. Growth must be multiplicative so a
                // caller that cannot name a structure still finishes in O(log n).
                assert!(
                    got >= limit + limit / 2,
                    "normalise_need({need}, {limit}) = {got} grows arithmetically; \
                     the floor must be at least 1.5x the bytes examined or the \
                     documented caller loop crawls"
                );
                assert!(
                    got >= need,
                    "normalise_need({need}, {limit}) = {got} discarded the prober's \
                     larger structural need {need}"
                );
            }
        }
    }
}

#[cfg(test)]
mod tier_spacing {
    use super::*;

    /// Every tier, highest first. Adding a tier without adding it here is
    /// caught by the exhaustiveness assertion below.
    const TIERS: [(&str, u8); 6] = [
        ("CERTAIN", TIER_CERTAIN),
        ("STRONG", TIER_STRONG),
        ("STRUCTURAL", TIER_STRUCTURAL),
        ("LATTICE_STRONG", TIER_LATTICE_STRONG),
        ("LATTICE_WEAK", TIER_LATTICE_WEAK),
        ("HEURISTIC", TIER_HEURISTIC),
    ];

    /// The scoring model reports `Ambiguous` when the top two candidates are
    /// within `TIE_THRESHOLD`. For that to mean "the evidence is genuinely
    /// equal" rather than "the tiers happen to sit close together", adjacent
    /// tiers must be separated by strictly more than `TIE_THRESHOLD` — so a
    /// candidate on a *lower* tier can never tie with one above it, and only a
    /// same-tier collision is ever reported as ambiguous.
    ///
    /// This is the invariant behind moving `TIER_LATTICE_STRONG` 144 -> 128:
    /// at 144 the gap to `TIER_STRUCTURAL` (160) was exactly 16, so a
    /// `STRUCTURAL` container and a `LATTICE_STRONG` elementary stream were
    /// reported as tied despite the model ranking one strictly above the other.
    /// Without this guard the next tier added lands back in that trap silently.
    #[test]
    fn adjacent_tiers_are_further_apart_than_the_tie_threshold() {
        for pair in TIERS.windows(2) {
            let (hi_name, hi) = pair[0];
            let (lo_name, lo) = pair[1];
            assert!(
                hi > lo,
                "TIERS must be listed strictly descending: {hi_name} ({hi}) is not above {lo_name} ({lo})"
            );
            let gap = hi - lo;
            assert!(
                gap > TIE_THRESHOLD,
                "{hi_name} ({hi}) and {lo_name} ({lo}) are {gap} apart, which is not more than \
                 TIE_THRESHOLD ({TIE_THRESHOLD}): two candidates on these different tiers would \
                 be reported as Ambiguous even though the model ranks one strictly above the other"
            );
        }
    }

    /// `TIERS` above must list every tier the crate defines. `Confidence`'s
    /// public constants are the enumeration of record, so pin the two together:
    /// a new `pub const` on `Confidence` whose value is missing from `TIERS`
    /// would otherwise skip the spacing check entirely.
    #[test]
    fn tiers_covers_every_public_confidence_constant() {
        let public = [
            ("CERTAIN", Confidence::CERTAIN),
            ("STRONG", Confidence::STRONG),
            ("STRUCTURAL", Confidence::STRUCTURAL),
            ("LATTICE_STRONG", Confidence::LATTICE_STRONG),
            ("LATTICE_WEAK", Confidence::LATTICE_WEAK),
            ("HEURISTIC", Confidence::HEURISTIC),
        ];
        assert_eq!(
            public.len(),
            TIERS.len(),
            "TIERS and Confidence's public tier constants have drifted apart"
        );
        for (name, c) in public {
            let found = TIERS.iter().find(|(n, _)| *n == name);
            let (_, v) = found.unwrap_or_else(|| panic!("tier {name} is missing from TIERS"));
            assert_eq!(
                *v,
                c.as_u8(),
                "tier {name}: TIERS says {v}, Confidence::{name} says {}",
                c.as_u8()
            );
        }
    }
}

#[cfg(test)]
mod dispatch {
    use super::*;
    use alloc::vec;

    /// Every `Format` variant must be *reachable*: either its own prober is a
    /// `PROBERS` row, or `candidate_format` can produce it from a registered
    /// one. Deleting a `PROBERS` row for a format that only that row reaches
    /// silently disables detection of a real format; this test turns that into
    /// a hard failure instead.
    ///
    /// The only cross-row transformation is the EBML one: the EBML prober is
    /// registered under `Format::Matroska` and `candidate_format` promotes a
    /// `DocType::Webm` to `Format::WebM`. Every other registered format maps to
    /// itself, so the reachable set is *exactly* `PROBERS`'s formats ∪ `WebM`.
    #[test]
    fn every_format_variant_is_reachable() {
        let any_registered: Vec<Format> = PROBERS.iter().map(|(f, _)| *f).collect();
        // The one produced-by-candidate_format case (not itself in PROBERS).
        let webm = candidate_format(
            Format::Matroska,
            Detail::Ebml {
                doc_type: DocType::Webm,
            },
        );
        assert_eq!(webm, Format::WebM);

        // The complete variant list, drifted-shielded against `Format`.
        for variant in [
            Format::MpegTs,
            Format::Isobmff,
            Format::MpegPs,
            Format::Matroska,
            Format::WebM,
            Format::Flv,
            Format::Mxf,
            Format::Wav,
            Format::Ogg,
            Format::Asf,
            Format::AdtsAac,
            Format::Mp3,
            Format::AnnexB,
        ] {
            assert!(
                any_registered.contains(&variant) || variant == Format::WebM,
                "Format::{} is not reachable: it is neither a PROBERS row nor \
                 produced by candidate_format from one",
                variant.name()
            );
        }
        // Belt-and-braces: also verify every registered row maps *to itself*
        // under `candidate_format` except the EBML special case, so a future
        // edit cannot quietly re-point a row elsewhere and strand its format.
        for &(f, _) in PROBERS {
            let resolved = candidate_format(f, Detail::None);
            assert!(
                resolved == f || f == Format::Matroska,
                "{:?} row resolves to {:?} under no-detail — unexpected drift",
                f,
                resolved
            );
        }
    }

    /// `Probe::Ambiguous` must carry only the candidates actually within
    /// `TIE_THRESHOLD` of the winner, not every lower-scored also-ran (the
    /// variant documents "two or more candidates within `TIE_THRESHOLD`").
    #[test]
    fn ambiguous_lists_only_genuinely_tied_candidates() {
        let mk = |format: Format, score: u8| Candidate {
            format,
            confidence: Confidence(score),
            detail: Detail::None,
        };
        // Scores 240, 232, 192: 232 is within 16 of 240 (tied), 192 is not.
        let candidates = vec![
            mk(Format::MpegTs, 240),
            mk(Format::Isobmff, 232),
            mk(Format::MpegPs, 192),
        ];
        let p = probe_to_identify(&candidates);
        match p {
            Probe::Ambiguous { candidates: tied } => {
                assert_eq!(tied.len(), 2, "only the top two are within TIE_THRESHOLD");
                assert_eq!(tied[0].format, Format::MpegTs);
                assert_eq!(tied[1].format, Format::Isobmff);
            }
            other => panic!("expected Ambiguous, got {other:?}"),
        }
    }

    /// A top candidate that beats its runner-up by more than `TIE_THRESHOLD` is
    /// `Identified`, not `Ambiguous`.
    #[test]
    fn decisive_winner_is_not_ambiguous() {
        let mk = |format: Format, score: u8| Candidate {
            format,
            confidence: Confidence(score),
            detail: Detail::None,
        };
        let candidates = vec![
            mk(Format::MpegTs, 240),
            mk(Format::Isobmff, 200),
            mk(Format::MpegPs, 64),
        ];
        match probe_to_identify(&candidates) {
            Probe::Identified { format, .. } => assert_eq!(format, Format::MpegTs),
            other => panic!("expected Identified, got {other:?}"),
        }
    }
}
