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

use crate::box_types::{BoxRef, box_iter};
use crate::movie_fragment::{
    MovieFragmentBox, MovieFragmentHeaderBox, TFHD_DEFAULT_BASE_IS_MOOF,
    TrackFragmentBaseMediaDecodeTimeBox, TrackFragmentHeaderBox, TrackFragmentRunBox,
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
    box_iter(body)
        .map_while(|step| step.ok())
        .map(|(bx, _)| (bx.header.box_type.0, bx))
        .collect()
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
    /// `tfhd.track_id` (§8.8.7) — the key the cross-segment validator pairs
    /// trafs by, so a segment whose tracks are sparse or reordered is compared
    /// against the right predecessor (audit r05-W30b: pairing by traf *index*
    /// mis-paired them).
    track_id: Option<u32>,
    tfdt: Option<u64>,
    /// Sum of the `trun` sample durations across all `trun` of this `traf`.
    total_duration: u64,
}

/// Decode of a media segment's fragment(s) (used by both the per-segment and
/// cross-segment validators).
///
/// Aggregated over **every** `moof` in the segment, not just the first (audit
/// r05-W30b): returning only the first fragment's info made
/// [`validate_cmaf_track`] compare a multi-fragment segment's *first* `tfdt`
/// against the next segment's, reporting a false
/// `track.tfdt.discontinuity`. Each track's durations are summed and its
/// decode-time span tracked, so the cross-segment check uses the segment's
/// real end.
struct MediaInfo {
    /// `mfhd.sequence_number` of the segment's **first** fragment (the value a
    /// playlist orders by).
    sequence_number: Option<u32>,
    /// Per-track aggregate, keyed by `tfhd.track_id`, in first-seen order.
    tracks: Vec<TrackFragmentInfo>,
}

/// One media segment's aggregate for a single track (across all its `moof`s).
struct TrackFragmentInfo {
    track_id: u32,
    /// Lowest `tfdt` seen for the track in this segment.
    first_tfdt: Option<u64>,
    /// Highest `tfdt + Σ durations` seen for the track in this segment.
    last_decode_end: Option<u64>,
    /// Total sample duration across the track's fragments in this segment.
    total_duration: u64,
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

    let mut aggregated = MediaInfo {
        sequence_number: None,
        tracks: Vec::new(),
    };

    // One whole-file `mdat` scan shared by every moof's resolver call.
    let mdat_extents = crate::frag_offsets::mdat_ranges(bytes);
    // Validate each moof and its following mdat.
    for &mp in &moof_positions {
        let (_, moof_bx) = &top[mp];
        let (seq_no, info) = validate_moof(moof_bx.body, mp + 1, issues);
        // The parsed `moof`, for the shared §8.8.7 offset resolver below.
        let moof_parsed = MovieFragmentBox::parse_body(moof_bx.body).ok();
        // Byte offset of this `moof` in `bytes`, for the §8.8.7 resolver (which
        // needs file coordinates, not the box index).
        let base = bytes.as_ptr() as usize;
        let body_ptr = moof_bx.body.as_ptr() as usize;
        let moof_byte_off = (body_ptr >= base)
            .then(|| body_ptr - base)
            .and_then(|body_off| body_off.checked_sub(moof_bx.header.header_size()));
        if aggregated.sequence_number.is_none() {
            aggregated.sequence_number = seq_no;
        }

        // moof must be followed by mdat (§8.8.4). The resolver below validates
        // that the sample ranges land *inside* an mdat, so only the pairing is
        // checked here.
        let has_mdat_after = match top.get(mp + 1) {
            Some((t, _)) if t == &MDAT => true,
            _ => {
                issues.push(ConformanceIssue::error(
                    "media.mdat.missing",
                    format!(
                        "moof (box #{}) is not followed by an mdat box \
                         (ISO/IEC 14496-12 §8.8.4)",
                        mp + 1
                    ),
                ));
                false
            }
        };
        let _ = has_mdat_after;

        // Every `trun`'s resolved sample ranges must land inside the `mdat`
        // payload (§8.8.7 / §8.8.8). The ranges come from the crate's single
        // §8.8.7 resolver, `frag_offsets::sample_ranges`, so this validator and
        // the demuxers cannot disagree about where a fragment's bytes live
        // (audit r05-W30a: the previous arithmetic resolved each `trun` against
        // the previous one's end and ignored `tfhd.base_data_offset`, the
        // `default-base-is-moof` flag, and the "a later traf continues from
        // here" rule §8.8.7 defines for a second `traf` in one `moof`).
        let mut checked: Vec<(i64, i64)> = Vec::new();
        for (ti, traf) in info.iter().enumerate() {
            let (Some(track_id), Some(moof_box), Some(moof_off)) =
                (traf.track_id, moof_parsed.as_ref(), moof_byte_off)
            else {
                continue;
            };
            let ranges = match crate::frag_offsets::sample_ranges_in(
                bytes,
                &mdat_extents,
                moof_off,
                moof_box,
                track_id,
            ) {
                Ok(r) => r,
                Err(_) => {
                    issues.push(ConformanceIssue::error(
                            "media.mdat.overrun",
                            format!(
                                "moof #{}, traf #{}: sample data offsets do not resolve to a                                  range inside an mdat (ISO/IEC 14496-12 §8.8.7)",
                                mp + 1,
                                ti + 1
                            ),
                        ));
                    continue;
                }
            };
            // The run's overall span (first sample start .. last sample end), so
            // the message names a run and overlaps between runs are detectable.
            if let (Some(first), Some(last)) = (ranges.first(), ranges.last()) {
                checked.push((
                    i64::try_from(first.start).unwrap_or(i64::MAX),
                    i64::try_from(last.end).unwrap_or(i64::MAX),
                ));
            }
        }

        // Two runs must not claim the same bytes: overlapping ranges would read
        // one sample's data twice and leave another's unreachable.
        checked.sort_unstable();
        for w in checked.windows(2) {
            let ((a_lo, a_hi), (b_lo, b_hi)) = (w[0], w[1]);
            if a_hi > b_lo && b_hi > a_lo {
                issues.push(ConformanceIssue::error(
                    "media.mdat.overlap",
                    format!(
                        "moof #{}: trun data ranges {}..{} and {}..{} overlap                          (ISO/IEC 14496-12 §8.8.8)",
                        mp + 1,
                        a_lo,
                        a_hi,
                        b_lo,
                        b_hi
                    ),
                ));
            }
        }

        for traf in info {
            if let Some(track_id) = traf.track_id {
                match aggregated
                    .tracks
                    .iter_mut()
                    .find(|t| t.track_id == track_id)
                {
                    Some(t) => {
                        t.total_duration += traf.total_duration;
                        if let Some(tfdt) = traf.tfdt {
                            t.first_tfdt = Some(t.first_tfdt.map_or(tfdt, |f| f.min(tfdt)));
                            let end = tfdt.saturating_add(traf.total_duration);
                            t.last_decode_end = Some(t.last_decode_end.map_or(end, |e| e.max(end)));
                        }
                    }
                    None => {
                        let end = traf
                            .tfdt
                            .map(|tfdt| tfdt.saturating_add(traf.total_duration));
                        aggregated.tracks.push(TrackFragmentInfo {
                            track_id,
                            first_tfdt: traf.tfdt,
                            last_decode_end: end,
                            total_duration: traf.total_duration,
                        });
                    }
                }
            }
        }
    }

    Some(aggregated)
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

fn validate_moof(
    moof: &[u8],
    moof_no: usize,
    issues: &mut Vec<ConformanceIssue>,
) -> (Option<u32>, Vec<TrafInfo>) {
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

    (sequence_number, traf_infos)
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
    let track_id = tfhd.as_ref().map(|h| h.track_id);
    let default_duration = tfhd.as_ref().and_then(|h| h.default_sample_duration);

    // CMAF §7.3.2.3 / ISO/IEC 14496-12 §8.8.7: a CMAF fragment's `tfhd` must
    // set `default-base-is-moof`, so its `trun.data_offset` values resolve
    // against the `moof` start. Without it the base falls back to the
    // enclosing `traf`'s `base_data_offset` or the previous fragment's end —
    // a resolving rule no CMAF client implements.
    if let Some(h) = &tfhd
        && h.flags & TFHD_DEFAULT_BASE_IS_MOOF == 0
    {
        issues.push(ConformanceIssue::error(
            "media.tfhd.default-base-is-moof",
            format!(
                "moof #{moof_no}, traf #{traf_no}: tfhd does not set default-base-is-moof                  (ISO/IEC 23000-19 §7.3.2.3)"
            ),
        ));
    }

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
    // A single `media.sample.zero-duration` per traf, not one per bad sample
    // (audit r05-W30d): a fragment with a broken sample table produced
    // thousands of identical issues, drowning every other finding.
    let mut zero_duration_samples: u64 = 0;
    for (_, trun_bx) in &truns {
        let Ok(run) = TrackFragmentRunBox::parse_body(trun_bx.body) else {
            continue;
        };
        for s in &run.samples {
            let dur = s.sample_duration.or(default_duration).unwrap_or(0);
            total_duration += u64::from(dur);
            if dur == 0 {
                zero_duration_samples += 1;
            }
        }
    }
    if zero_duration_samples > 0 {
        issues.push(ConformanceIssue::error(
            "media.sample.zero-duration",
            format!(
                "moof #{moof_no}, traf #{traf_no}: {zero_duration_samples} sample(s) have \
                 zero duration (ISO/IEC 14496-12 §8.8.8)"
            ),
        ));
    }

    // CMAF §7.3.2.3: the first sample of a fragment must be a sync (SAP) sample
    // — a segment opening mid-GOP publishes a random-access point that is not
    // actually random-accessible, and a player seeking to it decodes garbage.
    if let Some(first_run) = truns
        .first()
        .and_then(|(_, bx)| TrackFragmentRunBox::parse_body(bx.body).ok())
    {
        // §8.8.8.1's precedence for a sample's flags: `trun.first_sample_flags`
        // (sample 0 of the run), else the sample's own `trun.sample_flags`, else
        // the fragment-wide `tfhd.default_sample_flags`. Missing the last link
        // let a fragment with a per-fragment non-sync default pass this check
        // (audit fix wave 2, item 8).
        let sync = first_run
            .first_sample_flags
            .map(|f| f & SAMPLE_FLAGS_IS_NON_SYNC == 0)
            .or_else(|| {
                first_run
                    .samples
                    .first()
                    .and_then(|s| s.sample_flags)
                    .map(|f| f & SAMPLE_FLAGS_IS_NON_SYNC == 0)
            })
            .or_else(|| {
                tfhd.as_ref()
                    .and_then(|h| h.default_sample_flags)
                    .map(|f| f & SAMPLE_FLAGS_IS_NON_SYNC == 0)
            });
        if sync == Some(false) {
            // WARNING, not ERROR: the same walker validates an LL-HLS part or an
            // LL-DASH chunk, where starting mid-GOP is legal and expected
            // (`#EXT-X-PART:INDEPENDENT=NO`). A *segment* whose first sample is
            // not a sync sample publishes random access the content does not
            // have (CMAF §7.3.2.3); a caller that knows the fragment is a segment
            // boundary can escalate this to an error.
            issues.push(ConformanceIssue::warning(
                "media.sample.non-sync-first",
                format!(
                    "moof #{moof_no}, traf #{traf_no}: the fragment's first sample is not a \
                     sync sample; a CMAF segment must begin at a random-access point, an \
                     LL part/chunk need not (ISO/IEC 23000-19 §7.3.2.3)"
                ),
            ));
        }
    }

    TrafInfo {
        track_id,
        tfdt,
        total_duration,
    }
}

/// `sample_is_non_sync_sample` in the 32-bit sample-flags word
/// (ISO/IEC 14496-12 §8.8.3.1): bit 16, set when the sample is NOT a sync point.
const SAMPLE_FLAGS_IS_NON_SYNC: u32 = 1 << 16;

// ---------------------------------------------------------------------------
// Cross-segment validation
// ---------------------------------------------------------------------------

/// Re-slice a direct child of `parent_body` as its whole box (header + body).
///
/// `child_body` yields only the body, which is right for the `*Box::parse_body`
/// parsers but not for a type whose [`Parse`](broadcast_common::Parse) impl
/// takes the whole box (e.g. [`TrackHeaderBox`]).
fn child_whole<'a>(owner: &'a [u8], parent_body: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    let bx_body = child_body(parent_body, fourcc)?;
    // Locate the body inside the original buffer by pointer arithmetic, then
    // step back over the header.
    let base = owner.as_ptr() as usize;
    let body_ptr = bx_body.as_ptr() as usize;
    if body_ptr < base {
        return None;
    }
    let body_off = body_ptr - base;
    // The box header is 8 bytes, or 16 for a largesize box; a `tkhd` inside a
    // `moov` is never largesize, and `parse` revalidates the size itself.
    let start = body_off.checked_sub(8)?;
    let end = body_off.checked_add(bx_body.len())?;
    owner.get(start..end)
}

/// The `track_id`s an init segment declares, from every `moov > trak > tkhd`
/// (ISO/IEC 14496-12 §8.3.2). Empty when no `moov`/`tkhd` can be parsed.
fn init_track_ids(init: &[u8]) -> Vec<u32> {
    use crate::init_segment::TrackHeaderBox;
    use broadcast_common::Parse;

    let Some(moov) = child_body(init, &MOOV) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for (t, trak) in children(moov) {
        if &t != b"trak" {
            continue;
        }
        // Re-slice the whole `tkhd` box (header + body): `TrackHeaderBox::parse`
        // expects the full box.
        if let Some(bytes) = child_whole(moov, trak.body, &TKHD)
            && let Ok(h) = TrackHeaderBox::parse(bytes)
        {
            ids.push(h.track_id);
        }
    }
    ids
}

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

    // The track ids the init segment declares (`moov.trak.tkhd.track_id`,
    // §8.3.2): a media segment's `tfhd.track_id` must name one of them
    // (audit r05-W30c). A fragment for an undeclared track cannot be decoded —
    // there is no `stsd` for it — and a client silently drops it.
    let declared = init_track_ids(init);

    let mut infos: Vec<MediaInfo> = Vec::with_capacity(segments.len());
    for seg in segments {
        if let Some(info) = validate_media_inner(seg, &mut issues) {
            for track in &info.tracks {
                if !declared.is_empty() && !declared.contains(&track.track_id) {
                    issues.push(ConformanceIssue::error(
                        "track.tfhd.unknown-track",
                        format!(
                            "media segment names track_id {} which the init segment's moov \
                             does not declare (ISO/IEC 14496-12 §8.8.7)",
                            track.track_id
                        ),
                    ));
                }
            }
            infos.push(info);
        }
    }

    for pair in infos.windows(2) {
        let (prev, next) = (&pair[0], &pair[1]);

        // mfhd sequence_number strictly increasing (§8.8.5).
        if let (Some(a), Some(b)) = (prev.sequence_number, next.sequence_number)
            && b <= a
        {
            issues.push(ConformanceIssue::warning(
                "track.mfhd.sequence",
                format!(
                    "mfhd sequence_number not strictly increasing: {a} then {b} \
                     (ISO/IEC 14496-12 §8.8.5)"
                ),
            ));
        }

        // tfdt continuity, keyed by `tfhd.track_id` (§8.8.12; CMAF §7.5.19).
        // Matching by traf *index* mis-paired a segment whose tracks are sparse
        // or reordered (audit r05-W30b); the track id is the identity the spec
        // defines. The check uses the track's aggregate for the segment: this
        // segment's first `tfdt` must equal the previous segment's decode end.
        for cur in &next.tracks {
            let Some(prev_track) = prev.tracks.iter().find(|t| t.track_id == cur.track_id) else {
                continue;
            };
            let (Some(prev_first), Some(prev_end), Some(cur_first)) = (
                prev_track.first_tfdt,
                prev_track.last_decode_end,
                cur.first_tfdt,
            ) else {
                continue;
            };
            if cur_first != prev_end {
                issues.push(ConformanceIssue::error(
                    "track.tfdt.discontinuity",
                    format!(
                        "track {}: tfdt baseMediaDecodeTime discontinuity - expected {} \
                         (prev first tfdt {} + sum durations), got {} \
                         (ISO/IEC 14496-12 §8.8.12, ISO/IEC 23000-19 §7.5.19)",
                        cur.track_id, prev_end, prev_first, cur_first
                    ),
                ));
            }
        }
    }

    issues
}
