//! fMP4 / CMAF structural conformance validator — the fMP4 analogue of a
//! TR 101 290 monitor.
//!
//! Walks the ISOBMFF box tree of an initialization and/or media segment and
//! reports structural conformance issues ([`ConformanceIssue`], graded by
//! [`Severity`]) against ISO/IEC 14496-12:2015 (ISOBMFF), ISO/IEC 23000-19
//! (CMAF), and DASH-IF structural conventions. It never decodes media — it
//! only inspects box structure, presence, ordering, and the sample-size /
//! `data_offset` arithmetic.
//!
//! Parsing is done with the crate's own box walker ([`crate::box_iter`] /
//! [`crate::parse_box`]) and the `movie_fragment` field parsers, so malformed
//! input yields issues rather than panics: every fallible read is matched, and
//! nothing is `unwrap`ped on parsed lengths.
//!
//! # Checks
//!
//! ## Initialization segment ([`validate_init_segment`], ISO/IEC 14496-12)
//! - **`init.ftyp.missing`** (ERROR) — `ftyp` must be the first box (§4.3).
//! - **`init.ftyp.not-first`** (ERROR) — a box precedes `ftyp` (§4.3, §6.2.3).
//! - **`init.moov.missing`** (ERROR) — a `moov` box is required (§8.2.1).
//! - **`init.mvhd.missing`** (ERROR) — `moov` must contain `mvhd` (§8.2.2).
//! - **`init.trak.missing`** (ERROR) — `moov` must contain ≥1 `trak` (§8.3.1).
//! - **`init.trak.incomplete`** (ERROR) — each `trak` needs `tkhd` (§8.3.2) +
//!   `mdia`(`mdhd` §8.4.2, `hdlr` §8.4.3, `minf`(`stbl`(`stsd` §8.5.2))).
//! - **`init.mvex.missing`** (WARNING) — a fragmented movie's `moov` should
//!   carry `mvex`/`trex` (§8.8.1/§8.8.3); its absence means the init segment
//!   is not marked fragmented.
//!
//! ## Media segment ([`validate_media_segment`], ISO/IEC 14496-12 + CMAF)
//! - **`media.styp.missing`** (WARNING) — CMAF media segments begin with
//!   `styp` (§8.16.2; CMAF ISO/IEC 23000-19 §7.3.2.3).
//! - **`media.styp.brand`** (WARNING) — the `styp` brand set should include a
//!   segment brand (`msdh`/`msix`/`cmf*`) (CMAF §7.3.2.3).
//! - **`media.moof.missing`** (ERROR) — a media segment carries a `moof`
//!   (§8.8.4).
//! - **`media.mfhd.missing`** (ERROR) — `moof` must contain `mfhd` with a
//!   `sequence_number` (§8.8.5).
//! - **`media.traf.missing`** (ERROR) — `moof` must contain ≥1 `traf` (§8.8.6).
//! - **`media.tfhd.missing`** (ERROR) — each `traf` needs a `tfhd` (§8.8.7).
//! - **`media.tfdt.missing`** (ERROR) — each `traf` needs a `tfdt`; CMAF
//!   requires the baseMediaDecodeTime (§8.8.12; CMAF §7.5.19).
//! - **`media.trun.missing`** (ERROR) — each `traf` needs ≥1 `trun` (§8.8.8).
//! - **`media.moof.multi-traf`** (WARNING) — a CMAF fragment SHOULD carry a
//!   single track (one `traf`) (CMAF §7.3.2.3).
//! - **`media.mdat.missing`** (ERROR) — a `moof` must be followed by `mdat`
//!   (§8.8.4 / §8.1.1).
//! - **`media.mdat.orphan`** (ERROR) — an `mdat` with no preceding `moof`.
//! - **`media.mdat.overrun`** (ERROR) — the resolved `trun.data_offset` plus
//!   the sum of `trun` sample sizes must land within the `mdat` payload
//!   (§8.8.8: sample data is addressed inside `mdat`).
//! - **`media.sample.zero-duration`** (ERROR) — a sample duration of 0 is a
//!   timing fault (§8.8.8, `sample_duration`).
//!
//! ## Cross-segment ([`validate_cmaf_track`])
//! - **`track.tfdt.discontinuity`** (ERROR) — across consecutive segments the
//!   `tfdt` baseMediaDecodeTime must be contiguous: `next.tfdt ==
//!   prev.tfdt + Σ(prev sample durations)` (ISO/IEC 14496-12 §8.8.12; CMAF
//!   §7.5.19 contiguous decode timeline). Any gap or overlap is flagged.
//! - **`track.mfhd.sequence`** (WARNING) — `mfhd.sequence_number` should be
//!   strictly increasing across segments (§8.8.5).

use crate::box_types::{BoxRef, parse_box};
use crate::movie_fragment::{
    MovieFragmentHeaderBox, TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentHeaderBox,
    TrackFragmentRunBox,
};
use crate::segments::SegmentTypeBox;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// Box four-CCs (named constants — no magic numbers)
// ---------------------------------------------------------------------------

const FTYP: [u8; 4] = *b"ftyp";
const MOOV: [u8; 4] = *b"moov";
const MVHD: [u8; 4] = *b"mvhd";
const TRAK: [u8; 4] = *b"trak";
const TKHD: [u8; 4] = *b"tkhd";
const MDIA: [u8; 4] = *b"mdia";
const MDHD: [u8; 4] = *b"mdhd";
const HDLR: [u8; 4] = *b"hdlr";
const MINF: [u8; 4] = *b"minf";
const STBL: [u8; 4] = *b"stbl";
const STSD: [u8; 4] = *b"stsd";
const MVEX: [u8; 4] = *b"mvex";
const TREX: [u8; 4] = *b"trex";
const STYP: [u8; 4] = *b"styp";
const MOOF: [u8; 4] = *b"moof";
const MFHD: [u8; 4] = *b"mfhd";
const TRAF: [u8; 4] = *b"traf";
const TFHD: [u8; 4] = *b"tfhd";
const TFDT: [u8; 4] = *b"tfdt";
const TRUN: [u8; 4] = *b"trun";
const MDAT: [u8; 4] = *b"mdat";

/// CMAF/DASH segment brands accepted for the `styp` box (CMAF §7.3.2.3): the
/// DASH segment brands `msdh`/`msix` and any CMAF `cmf*` brand.
fn is_segment_brand(b: &[u8; 4]) -> bool {
    b == b"msdh" || b == b"msix" || &b[..3] == b"cmf"
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Severity of a [`ConformanceIssue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Severity {
    /// A structural violation that makes the segment non-conformant.
    Error,
    /// A deviation from a SHOULD-level convention; the segment may still play.
    Warning,
}

impl Severity {
    /// Spec/label token for the severity, per the #204 label convention.
    pub fn name(&self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

broadcast_common::impl_spec_display!(Severity);

/// A single conformance finding: a severity, a stable machine-readable `code`,
/// and a human-readable `message`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ConformanceIssue {
    /// How serious the finding is.
    pub severity: Severity,
    /// Stable dotted identifier for the check (e.g. `media.tfdt.missing`).
    pub code: &'static str,
    /// Human-readable explanation, with the offending values.
    pub message: String,
}

impl ConformanceIssue {
    fn error(code: &'static str, message: String) -> Self {
        Self {
            severity: Severity::Error,
            code,
            message,
        }
    }
    fn warning(code: &'static str, message: String) -> Self {
        Self {
            severity: Severity::Warning,
            code,
            message,
        }
    }
}

// ---------------------------------------------------------------------------
// Generic box-tree helpers (never panic on malformed input)
// ---------------------------------------------------------------------------

/// Walk the direct children of a container `body`, collecting `(fourcc, BoxRef)`
/// pairs. Stops cleanly at the first malformed child (so a truncated tail does
/// not panic and does not abort the whole walk with an error).
fn children(body: &[u8]) -> Vec<([u8; 4], BoxRef<'_>)> {
    let mut out = Vec::new();
    let mut remaining = body;
    while !remaining.is_empty() {
        match parse_box(remaining) {
            Ok((bx, consumed)) => {
                out.push((bx.header.box_type.0, bx));
                if consumed == 0 || consumed > remaining.len() {
                    break;
                }
                remaining = &remaining[consumed..];
            }
            Err(_) => break,
        }
    }
    out
}

/// Whether a container `body` has a direct child of the given four-CC.
fn has_child(body: &[u8], fourcc: &[u8; 4]) -> bool {
    children(body).iter().any(|(t, _)| t == fourcc)
}

/// Return the body of the first direct child with the given four-CC.
fn child_body<'a>(body: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    children(body)
        .into_iter()
        .find(|(t, _)| t == fourcc)
        .map(|(_, bx)| bx.body)
}

// ---------------------------------------------------------------------------
// Init-segment validation
// ---------------------------------------------------------------------------

/// Validate a fMP4/CMAF **initialization segment** against ISO/IEC 14496-12.
///
/// Returns every structural [`ConformanceIssue`] found (empty ⇒ conformant).
/// Malformed input is reported as issues, never a panic.
pub fn validate_init_segment(bytes: &[u8]) -> Vec<ConformanceIssue> {
    let mut issues = Vec::new();
    let top = children(bytes);

    // ftyp present and first (§4.3, §6.2.3).
    match top.iter().position(|(t, _)| t == &FTYP) {
        None => issues.push(ConformanceIssue::error(
            "init.ftyp.missing",
            "no ftyp box: an initialization segment must begin with ftyp (ISO/IEC 14496-12 §4.3)"
                .to_string(),
        )),
        Some(0) => {}
        Some(pos) => issues.push(ConformanceIssue::error(
            "init.ftyp.not-first",
            format!(
                "ftyp is box #{} but must be the first box (ISO/IEC 14496-12 §4.3, §6.2.3)",
                pos + 1
            ),
        )),
    }

    // moov present (§8.2.1).
    let Some(moov) = child_body(bytes, &MOOV) else {
        issues.push(ConformanceIssue::error(
            "init.moov.missing",
            "no moov box: an initialization segment requires a movie box (ISO/IEC 14496-12 §8.2.1)"
                .to_string(),
        ));
        return issues;
    };

    validate_moov(moov, &mut issues);
    issues
}

fn validate_moov(moov: &[u8], issues: &mut Vec<ConformanceIssue>) {
    // mvhd (§8.2.2).
    if !has_child(moov, &MVHD) {
        issues.push(ConformanceIssue::error(
            "init.mvhd.missing",
            "moov has no mvhd (ISO/IEC 14496-12 §8.2.2)".to_string(),
        ));
    }

    // ≥1 trak (§8.3.1).
    let traks: Vec<_> = children(moov)
        .into_iter()
        .filter(|(t, _)| t == &TRAK)
        .collect();
    if traks.is_empty() {
        issues.push(ConformanceIssue::error(
            "init.trak.missing",
            "moov has no trak: at least one track is required (ISO/IEC 14496-12 §8.3.1)"
                .to_string(),
        ));
    }
    for (idx, (_, trak)) in traks.iter().enumerate() {
        validate_trak(trak.body, idx + 1, issues);
    }

    // mvex/trex — fragmented-movie marker (§8.8.1/§8.8.3).
    match child_body(moov, &MVEX) {
        None => issues.push(ConformanceIssue::warning(
            "init.mvex.missing",
            "moov has no mvex: the init segment is not marked as a fragmented movie \
             (ISO/IEC 14496-12 §8.8.1) — required for a fragmented (CMAF/DASH) workflow"
                .to_string(),
        )),
        Some(mvex) => {
            if !has_child(mvex, &TREX) {
                issues.push(ConformanceIssue::warning(
                    "init.mvex.missing",
                    "mvex has no trex: fragmented tracks need per-track defaults \
                     (ISO/IEC 14496-12 §8.8.3)"
                        .to_string(),
                ));
            }
        }
    }
}

fn validate_trak(trak: &[u8], track_no: usize, issues: &mut Vec<ConformanceIssue>) {
    let mut missing: Vec<&str> = Vec::new();

    if !has_child(trak, &TKHD) {
        missing.push("tkhd");
    }
    match child_body(trak, &MDIA) {
        None => missing.push("mdia"),
        Some(mdia) => {
            if !has_child(mdia, &MDHD) {
                missing.push("mdia>mdhd");
            }
            if !has_child(mdia, &HDLR) {
                missing.push("mdia>hdlr");
            }
            match child_body(mdia, &MINF) {
                None => missing.push("mdia>minf"),
                Some(minf) => match child_body(minf, &STBL) {
                    None => missing.push("mdia>minf>stbl"),
                    Some(stbl) => {
                        if !has_child(stbl, &STSD) {
                            missing.push("mdia>minf>stbl>stsd");
                        }
                    }
                },
            }
        }
    }

    if !missing.is_empty() {
        issues.push(ConformanceIssue::error(
            "init.trak.incomplete",
            format!(
                "trak #{track_no} is missing required box(es): {} \
                 (ISO/IEC 14496-12 §8.3.2/§8.4)",
                missing.join(", ")
            ),
        ));
    }
}

// ---------------------------------------------------------------------------
// Media-segment validation
// ---------------------------------------------------------------------------

/// A per-`traf` decode of the fields the validator needs downstream.
struct TrafInfo {
    tfdt: Option<u64>,
    /// Sum of the `trun` sample durations across all `trun` of this `traf`.
    total_duration: u64,
    /// Sum of the `trun` sample sizes across all `trun` of this `traf`.
    total_size: u64,
    /// The smallest resolved `data_offset` seen (relative to moof start).
    min_data_offset: Option<i64>,
}

/// Decode of a single media segment's `moof` (used by both the per-segment and
/// cross-segment validators).
struct MediaInfo {
    sequence_number: Option<u32>,
    trafs: Vec<TrafInfo>,
}

/// Validate a fMP4/CMAF **media segment** against ISO/IEC 14496-12 + CMAF.
///
/// Returns every structural [`ConformanceIssue`] found (empty ⇒ conformant).
/// Malformed input is reported as issues, never a panic.
pub fn validate_media_segment(bytes: &[u8]) -> Vec<ConformanceIssue> {
    let mut issues = Vec::new();
    validate_media_inner(bytes, &mut issues);
    issues
}

/// Core media-segment walk. Returns the decoded [`MediaInfo`] (for the first
/// `moof`) so the cross-segment validator can reuse it without re-walking.
fn validate_media_inner(bytes: &[u8], issues: &mut Vec<ConformanceIssue>) -> Option<MediaInfo> {
    let top = children(bytes);

    // styp (CMAF §7.3.2.3 / §8.16.2).
    match top.iter().find(|(t, _)| t == &STYP) {
        None => issues.push(ConformanceIssue::warning(
            "media.styp.missing",
            "no styp box: a CMAF media segment begins with styp \
             (ISO/IEC 14496-12 §8.16.2, ISO/IEC 23000-19 §7.3.2.3)"
                .to_string(),
        )),
        Some((_, bx)) => {
            // Re-parse the whole styp box (header + body) for its brands.
            let whole = styp_whole(bytes, bx);
            match whole.and_then(|w| SegmentTypeBox::parse_box(w).ok()) {
                Some(styp) => {
                    let ok = is_segment_brand(&styp.major_brand)
                        || styp.compatible_brands.iter().any(is_segment_brand);
                    if !ok {
                        issues.push(ConformanceIssue::warning(
                            "media.styp.brand",
                            "styp carries no recognised segment brand (msdh/msix/cmf*) \
                             (ISO/IEC 23000-19 §7.3.2.3)"
                                .to_string(),
                        ));
                    }
                }
                None => issues.push(ConformanceIssue::warning(
                    "media.styp.brand",
                    "styp box could not be parsed for its brand list".to_string(),
                )),
            }
        }
    }

    // moof (§8.8.4) + moof↔mdat pairing (§8.1.1).
    let moof_positions: Vec<usize> = top
        .iter()
        .enumerate()
        .filter(|(_, (t, _))| t == &MOOF)
        .map(|(i, _)| i)
        .collect();
    let mdat_positions: Vec<usize> = top
        .iter()
        .enumerate()
        .filter(|(_, (t, _))| t == &MDAT)
        .map(|(i, _)| i)
        .collect();

    if moof_positions.is_empty() {
        issues.push(ConformanceIssue::error(
            "media.moof.missing",
            "no moof box: a media segment requires a movie fragment (ISO/IEC 14496-12 §8.8.4)"
                .to_string(),
        ));
    }

    // mdat with no immediately-preceding moof → orphan.
    for &mp in &mdat_positions {
        if mp == 0 || top[mp - 1].0 != MOOF {
            issues.push(ConformanceIssue::error(
                "media.mdat.orphan",
                format!(
                    "mdat (box #{}) is not immediately preceded by a moof \
                     (ISO/IEC 14496-12 §8.1.1/§8.8.4)",
                    mp + 1
                ),
            ));
        }
    }

    let mut first_info: Option<MediaInfo> = None;

    // Validate each moof and its following mdat.
    for &mp in &moof_positions {
        let (_, moof_bx) = &top[mp];
        let info = validate_moof(moof_bx.body, mp + 1, issues);

        // moof must be followed by mdat (§8.8.4).
        let mdat_body = match top.get(mp + 1) {
            Some((t, bx)) if t == &MDAT => Some(bx.body),
            _ => {
                issues.push(ConformanceIssue::error(
                    "media.mdat.missing",
                    format!(
                        "moof (box #{}) is not followed by an mdat box \
                         (ISO/IEC 14496-12 §8.8.4)",
                        mp + 1
                    ),
                ));
                None
            }
        };

        // trun sample sizes + data_offset must land inside the mdat payload
        // (§8.8.8). data_offset is relative to the moof start under CMAF's
        // default-base-is-moof; the mdat payload starts at (moof_size + mdat
        // header) from the moof start.
        if let Some(mdat_body) = mdat_body {
            let moof_size = moof_bx.header.size;
            // mdat header size: the box header preceding `bx.body`.
            let mdat_hdr = top[mp + 1].1.header.header_size() as u64;
            let mdat_payload_end = moof_size + mdat_hdr + mdat_body.len() as u64;
            for (ti, traf) in info.trafs.iter().enumerate() {
                if let Some(off) = traf.min_data_offset {
                    let start = off;
                    let end = start + traf.total_size as i64;
                    // Valid data lives at [moof_start .. mdat_payload_end].
                    if start < 0 || (end as u64) > mdat_payload_end {
                        issues.push(ConformanceIssue::error(
                            "media.mdat.overrun",
                            format!(
                                "moof #{}, traf #{}: trun samples span offset {}..{} but the \
                                 mdat payload ends at {} (ISO/IEC 14496-12 §8.8.8)",
                                mp + 1,
                                ti + 1,
                                start,
                                end,
                                mdat_payload_end
                            ),
                        ));
                    }
                }
            }
        }

        if first_info.is_none() {
            first_info = Some(info);
        }
    }

    first_info
}

/// Re-slice the whole styp box (header + body) from the segment bytes so it can
/// be parsed by [`SegmentTypeBox::parse_box`], which expects the full box.
fn styp_whole<'a>(bytes: &'a [u8], bx: &BoxRef<'a>) -> Option<&'a [u8]> {
    // Find the styp box in the top-level walk by matching the body slice's
    // start against the original buffer, then take header_size + body.
    let hdr = bx.header.header_size();
    // The body slice is a sub-slice of `bytes`; compute its offset.
    let base = bytes.as_ptr() as usize;
    let body_ptr = bx.body.as_ptr() as usize;
    if body_ptr < base {
        return None;
    }
    let body_off = body_ptr - base;
    let start = body_off.checked_sub(hdr)?;
    let end = body_off.checked_add(bx.body.len())?;
    bytes.get(start..end)
}

fn validate_moof(moof: &[u8], moof_no: usize, issues: &mut Vec<ConformanceIssue>) -> MediaInfo {
    let mut sequence_number = None;

    // mfhd (§8.8.5).
    match child_body(moof, &MFHD) {
        None => issues.push(ConformanceIssue::error(
            "media.mfhd.missing",
            format!("moof #{moof_no} has no mfhd (ISO/IEC 14496-12 §8.8.5)"),
        )),
        Some(mfhd) => match MovieFragmentHeaderBox::parse_body(mfhd) {
            Ok(h) => sequence_number = Some(h.sequence_number),
            Err(_) => issues.push(ConformanceIssue::error(
                "media.mfhd.missing",
                format!("moof #{moof_no} mfhd could not be parsed (ISO/IEC 14496-12 §8.8.5)"),
            )),
        },
    }

    // ≥1 traf (§8.8.6).
    let trafs: Vec<_> = children(moof)
        .into_iter()
        .filter(|(t, _)| t == &TRAF)
        .collect();
    if trafs.is_empty() {
        issues.push(ConformanceIssue::error(
            "media.traf.missing",
            format!("moof #{moof_no} has no traf (ISO/IEC 14496-12 §8.8.6)"),
        ));
    }
    // CMAF: a fragment SHOULD carry a single track (CMAF §7.3.2.3).
    if trafs.len() > 1 {
        issues.push(ConformanceIssue::warning(
            "media.moof.multi-traf",
            format!(
                "moof #{moof_no} carries {} traf boxes; a CMAF fragment SHOULD be single-track \
                 (ISO/IEC 23000-19 §7.3.2.3)",
                trafs.len()
            ),
        ));
    }

    let mut traf_infos = Vec::with_capacity(trafs.len());
    for (idx, (_, traf)) in trafs.iter().enumerate() {
        traf_infos.push(validate_traf(traf.body, moof_no, idx + 1, issues));
    }

    MediaInfo {
        sequence_number,
        trafs: traf_infos,
    }
}

fn validate_traf(
    traf: &[u8],
    moof_no: usize,
    traf_no: usize,
    issues: &mut Vec<ConformanceIssue>,
) -> TrafInfo {
    // tfhd (§8.8.7) — parse for default_sample_duration/size, tolerate absence.
    let tfhd = child_body(traf, &TFHD);
    if tfhd.is_none() {
        issues.push(ConformanceIssue::error(
            "media.tfhd.missing",
            format!("moof #{moof_no}, traf #{traf_no} has no tfhd (ISO/IEC 14496-12 §8.8.7)"),
        ));
    }
    let tfhd = tfhd.and_then(|b| TrackFragmentHeaderBox::parse_body(b).ok());
    let default_duration = tfhd.as_ref().and_then(|h| h.default_sample_duration);
    let default_size = tfhd.as_ref().and_then(|h| h.default_sample_size);

    // tfdt (§8.8.12; CMAF §7.5.19 requires it).
    let tfdt = match child_body(traf, &TFDT) {
        None => {
            issues.push(ConformanceIssue::error(
                "media.tfdt.missing",
                format!(
                    "moof #{moof_no}, traf #{traf_no} has no tfdt: CMAF requires the \
                     baseMediaDecodeTime (ISO/IEC 14496-12 §8.8.12, ISO/IEC 23000-19 §7.5.19)"
                ),
            ));
            None
        }
        Some(b) => match TrackFragmentBaseMediaDecodeTimeBox::parse_body(b) {
            Ok(t) => Some(t.base_media_decode_time()),
            Err(_) => {
                issues.push(ConformanceIssue::error(
                    "media.tfdt.missing",
                    format!("moof #{moof_no}, traf #{traf_no} tfdt could not be parsed"),
                ));
                None
            }
        },
    };

    // trun (§8.8.8) — need ≥1; accumulate sizes/durations + min data_offset.
    let truns: Vec<_> = children(traf)
        .into_iter()
        .filter(|(t, _)| t == &TRUN)
        .collect();
    if truns.is_empty() {
        issues.push(ConformanceIssue::error(
            "media.trun.missing",
            format!("moof #{moof_no}, traf #{traf_no} has no trun (ISO/IEC 14496-12 §8.8.8)"),
        ));
    }

    let mut total_duration: u64 = 0;
    let mut total_size: u64 = 0;
    let mut min_data_offset: Option<i64> = None;
    for (_, trun_bx) in &truns {
        let Ok(run) = TrackFragmentRunBox::parse_body(trun_bx.body) else {
            continue;
        };
        if let Some(off) = run.data_offset {
            let off = off as i64;
            min_data_offset = Some(min_data_offset.map_or(off, |m: i64| m.min(off)));
        }
        for s in &run.samples {
            let dur = s.sample_duration.or(default_duration).unwrap_or(0);
            let sz = s.sample_size.or(default_size).unwrap_or(0);
            total_duration += dur as u64;
            total_size += sz as u64;
            if dur == 0 {
                issues.push(ConformanceIssue::error(
                    "media.sample.zero-duration",
                    format!(
                        "moof #{moof_no}, traf #{traf_no}: a sample has zero duration \
                         (ISO/IEC 14496-12 §8.8.8)"
                    ),
                ));
            }
        }
    }

    TrafInfo {
        tfdt,
        total_duration,
        total_size,
        min_data_offset,
    }
}

// ---------------------------------------------------------------------------
// Cross-segment validation
// ---------------------------------------------------------------------------

/// Validate a whole CMAF **track**: an initialization segment plus its media
/// segments in presentation order.
///
/// Runs [`validate_init_segment`] on `init`, [`validate_media_segment`] on each
/// element of `segments`, and adds the cross-segment continuity checks:
///
/// - `track.tfdt.discontinuity` (ERROR): `next.tfdt` must equal
///   `prev.tfdt + Σ(prev sample durations)` (contiguous decode timeline).
/// - `track.mfhd.sequence` (WARNING): `mfhd.sequence_number` must strictly
///   increase.
pub fn validate_cmaf_track(init: &[u8], segments: &[&[u8]]) -> Vec<ConformanceIssue> {
    let mut issues = validate_init_segment(init);

    let mut infos: Vec<MediaInfo> = Vec::with_capacity(segments.len());
    for seg in segments {
        if let Some(info) = validate_media_inner(seg, &mut issues) {
            infos.push(info);
        }
    }

    for pair in infos.windows(2) {
        let (prev, next) = (&pair[0], &pair[1]);

        // mfhd sequence_number strictly increasing (§8.8.5).
        if let (Some(a), Some(b)) = (prev.sequence_number, next.sequence_number) {
            if b <= a {
                issues.push(ConformanceIssue::warning(
                    "track.mfhd.sequence",
                    format!(
                        "mfhd sequence_number not strictly increasing: {a} then {b} \
                         (ISO/IEC 14496-12 §8.8.5)"
                    ),
                ));
            }
        }

        // tfdt continuity per track index (§8.8.12; CMAF §7.5.19).
        // Match trafs pairwise by index (single-track fragments in order).
        let n = prev.trafs.len().min(next.trafs.len());
        for i in 0..n {
            if let (Some(pt), Some(nt)) = (prev.trafs[i].tfdt, next.trafs[i].tfdt) {
                let expected = pt.saturating_add(prev.trafs[i].total_duration);
                if nt != expected {
                    issues.push(ConformanceIssue::error(
                        "track.tfdt.discontinuity",
                        format!(
                            "traf #{}: tfdt baseMediaDecodeTime discontinuity — expected {} \
                             (prev tfdt {} + Σ durations {}), got {} \
                             (ISO/IEC 14496-12 §8.8.12, ISO/IEC 23000-19 §7.5.19)",
                            i + 1,
                            expected,
                            pt,
                            prev.trafs[i].total_duration,
                            nt
                        ),
                    ));
                }
            }
        }
    }

    issues
}
