//! HLS playlist validator (RFC 8216bis draft-pantos-hls-rfc8216bis-22).
//!
//! Accepts a playlist text and a [`Report`](crate::Report), appending findings
//! for spec violations. Reuses [`broadcast_hls::MediaPlaylist::parse`] and
//! [`broadcast_hls::MasterPlaylist::parse`] as the structured parse layer — no
//! regex-based or line-by-line re-parsing of recognised tags.
//!
//! # Detection strategy
//!
//! 1. Attempt `MediaPlaylist::parse` first (the common case). If it succeeds,
//!    validate rule invariants the parser itself doesn't enforce (e.g. part
//!    duration limits, cross-reference integrity).
//! 2. If media parse fails, attempt `MasterPlaylist::parse` and apply
//!    multivariant rules.
//! 3. If both fail, fall back to the legacy line-based check (the old
//!    `check_playlist` logic) for the rules that still apply — a parse failure
//!    *is* a finding.
//!
//! After the structured parse, also run a minimal set of line-based checks
//! on the original text for rules the structured model doesn't enforce:
//! DATERANGE well-formedness (via `timed_metadata::DateRange::parse_tag_line`).
//!
//! # Rule IDs
//!
//! | ID | Severity | Description | Clause |
//! |---|---|---|---|
//! | `hls-parse-error` | Error | Playlist fails to parse as valid HLS | §4 |
//! | `hls-missing-extm3u` | Error | First non-empty line is not `#EXTM3U` | §4.4.1.1 |
//! | `hls-missing-targetduration` | Error | Media playlist with segments lacks TARGETDURATION | §4.4.3.1 |
//! | `hls-extinf-exceeds-target` | Error | EXTINF duration exceeds TARGETDURATION | §4.4.3.1 |
//! | `hls-part-duration-range` | Error | Part duration above PART-TARGET, or below 85% of it (exemptions apply to the lower bound only) | §4.4.4.9 |
//! | `hls-preload-hint-with-endlist` | Error | PRELOAD-HINT in a playlist with ENDLIST | §4.4.5.3 |
//! | `hls-skip-without-can-skip-until` | Error | EXT-X-SKIP without CAN-SKIP-UNTIL in SERVER-CONTROL | §4.4.5.2, §4.4.3.8 |
//! | `hls-malformed-daterange` | Error | DATERANGE line fails `DateRange::parse_tag_line` | §4.4.5.1 |

use crate::report::{Finding, Location, Report, Severity};
use alloc::vec::Vec;

/// Validate an HLS playlist text, appending findings for each violation.
///
/// Line numbers in [`Location`] are 1-based; `pid` is always 0.
pub fn check_hls_playlist(text: &str, report: &mut Report) {
    // Always run line-based DATERANGE checks on the original text — the
    // structured parser stores DATERANGE lines verbatim in `extra_tags`
    // without validating their internal attribute structure.
    check_daterange_lines(text, report);

    // Classify first: a Multivariant Playlist is identified by carrying an
    // `#EXT-X-STREAM-INF` or `#EXT-X-I-FRAME-STREAM-INF` tag (RFC 8216bis
    // §4.4.6.1/§4.4.6.5), and a Media Playlist by carrying `#EXTINF` or
    // `#EXT-X-TARGETDURATION`. Trying Media-then-Master by parse outcome
    // instead sent a *Media* Playlist that merely failed its media parse
    // (e.g. no segments and a missing TARGETDURATION) into the
    // multivariant branch, where `validate_master_playlist` had no rules and
    // reported clean (audit MD-W11).
    let class = classify_playlist(text);
    let media_attempt = broadcast_hls::MediaPlaylist::parse(text);
    let master_attempt = broadcast_hls::MasterPlaylist::parse(text);

    match class {
        PlaylistClass::Media => match media_attempt {
            Ok(media) => {
                validate_media_playlist(&media, report);
                legacy_line_checks(text, report);
            }
            Err(err) => {
                // The playlist *is* a Media Playlist (media-only tags are
                // present) but does not parse. Report the parse error with its
                // own location and reason — "failed to parse" with no line
                // number is exactly the case a validator exists for.
                report_parse_error(&err, "Media", report);
                legacy_line_checks(text, report);
            }
        },
        PlaylistClass::Master => match master_attempt {
            Ok(master) => validate_master_playlist(&master, report),
            Err(err) => report_parse_error(&err, "Multivariant", report),
        },
        // Neither kind's signature tags are present. If one of the parses
        // nonetheless succeeds, trust it (the tags may be spelled in a way
        // the classifier does not look for); otherwise report whichever
        // error carries more information.
        PlaylistClass::Unknown => match (media_attempt, master_attempt) {
            (Ok(media), _) => {
                validate_media_playlist(&media, report);
                legacy_line_checks(text, report);
            }
            // A playlist carrying neither a Media nor a Multivariant
            // signature tag is not a Playlist either parser can vouch for,
            // even when one of them happens to accept it. Reporting it clean
            // is the vacuous answer this classification exists to avoid: a
            // Media Playlist with neither `#EXTINF` nor
            // `#EXT-X-TARGETDURATION` (only `#EXT-X-ENDLIST`, say) lands
            // here (audit MD-W11).
            (_, Ok(_)) => {
                report.push(Finding::new(
                    Severity::Error,
                    Location::new(1, 0),
                    "hls-unsupported-playlist",
                    "Playlist carries neither a Media Playlist tag (§4.4.3.1 EXT-X-TARGETDURATION, §4.4.4.1 EXTINF) nor a Multivariant Playlist tag (§4.4.6.1 EXT-X-STREAM-INF, §4.4.6.5 EXT-X-I-FRAME-STREAM-INF) — it cannot be validated as either",
                ));
            }
            (Err(media_err), Err(_)) => {
                report_parse_error(&media_err, "Media or Multivariant", report);
                legacy_line_checks(text, report);
            }
        },
    }
}

/// Which kind of Playlist `text` declares itself to be, from the tags it
/// carries (RFC 8216bis §4.4.6.1/§4.4.6.5 for a Multivariant Playlist,
/// §4.4.3.1/§4.4.4.1 for a Media Playlist).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaylistClass {
    Media,
    Master,
    Unknown,
}

fn classify_playlist(text: &str) -> PlaylistClass {
    let mut media = false;
    let mut master = false;
    for line in text.lines() {
        let tag = line.trim();
        if tag.starts_with("#EXT-X-STREAM-INF:") || tag.starts_with("#EXT-X-I-FRAME-STREAM-INF:") {
            master = true;
        }
        if tag.starts_with("#EXTINF:") || tag.starts_with("#EXT-X-TARGETDURATION:") {
            media = true;
        }
    }
    match (media, master) {
        // A playlist carrying both is malformed; report it as the Media kind
        // so the media rules still apply.
        (true, _) => PlaylistClass::Media,
        (false, true) => PlaylistClass::Master,
        (false, false) => PlaylistClass::Unknown,
    }
}

/// Emit an `hls-parse-error` carrying the parser's own line number and reason.
fn report_parse_error(err: &broadcast_hls::Error, kind: &str, report: &mut Report) {
    // The parser's error carries the offending line number and the line
    // itself (RFC 8216bis §4.2); a validator that reports "failed to parse"
    // with no location is exactly the useless case (audit MD-W11).
    let line = match err {
        broadcast_hls::Error::HlsParse { line_no, .. } => *line_no,
        _ => 1,
    };
    report.push(Finding::new(
        Severity::Error,
        Location::new(line, 0),
        "hls-parse-error",
        alloc::format!("Playlist failed to parse as a valid {kind} Playlist: {err}"),
    ));
}

// ---------------------------------------------------------------------------
// Media Playlist structured validation (RFC 8216bis)
// ---------------------------------------------------------------------------

fn validate_media_playlist(pl: &broadcast_hls::MediaPlaylist, report: &mut Report) {
    // hls-preload-hint-with-endlist — §4.4.5.3
    if pl.endlist
        && let Some(ref ll) = pl.low_latency
        && ll.preload_hint_part.is_some()
    {
        report.push(Finding::new(
            Severity::Error,
            Location::new(1, 0),
            "hls-preload-hint-with-endlist",
            "Playlist carries EXT-X-ENDLIST and EXT-X-PRELOAD-HINT — §4.4.5.3: a playlist with ENDLIST MUST NOT contain PRELOAD-HINT",
        ));
    }

    // hls-skip-without-can-skip-until — §4.4.5.2, §4.4.3.8
    if pl.skip.is_some() {
        let has_can_skip = pl
            .low_latency
            .as_ref()
            .and_then(|ll| ll.can_skip_until)
            .is_some();
        if !has_can_skip {
            report.push(Finding::new(
                Severity::Error,
                Location::new(1, 0),
                "hls-skip-without-can-skip-until",
                "EXT-X-SKIP present but EXT-X-SERVER-CONTROL has no CAN-SKIP-UNTIL — §4.4.3.8",
            ));
        }
    }

    // hls-part-duration-range — §4.4.4.9
    // "The duration of a Partial Segment MUST be less than or equal to the
    // Part Target Duration. The duration of each Partial Segment MUST be at
    // least 85% of the Part Target Duration, with the exception of Partial
    // Segments with the INDEPENDENT=YES or GAP=YES attribute, Partial Segments
    // that are immediately followed by a Partial Segment with a GAP=YES
    // attribute, and the final Partial Segment of any Parent Segment."
    //
    // The exemption list attaches to the 85% floor only — the upper bound is
    // checked for every part. The open (in-progress) segment's parts are
    // checked too: they are the live edge and the ones a client is about to
    // fetch.
    if let Some(ref ll) = pl.low_latency {
        let part_target = ll.part_target.map_or(0.0, |pt| pt.get());
        if part_target > 0.0 {
            let lower = part_target * HLS_PART_DURATION_MIN_FRACTION;
            let upper = part_target;

            // The "immediately followed by a GAP=YES part" exemption needs no
            // special handling across a Parent Segment boundary: the part
            // that ends a segment is *already* floor-exempt as "the final
            // Partial Segment of any Parent Segment" (§4.4.4.9), so whether
            // the next segment's first part is a GAP cannot change the
            // verdict for it. Within a segment the following part is checked
            // directly.
            for (seg_idx, seg) in pl.segments.iter().enumerate() {
                check_part_durations(&seg.parts, seg_idx + 1, lower, upper, report);
            }
            if let Some(ref open) = pl.open_segment {
                check_part_durations(&open.parts, pl.segments.len() + 1, lower, upper, report);
            }
        }
    }
}

/// Apply the §4.4.4.9 part-duration bounds to one parent segment's parts.
///
/// The upper bound (`<= PART-TARGET`) applies to **every** part; the
/// exemptions (INDEPENDENT, GAP, followed-by-GAP, final part of the parent
/// segment) relax only the 85% lower bound.
fn check_part_durations(
    parts: &[broadcast_hls::PartSpec],
    segment_number: usize,
    lower: f64,
    upper: f64,
    report: &mut Report,
) {
    let part_count = parts.len();
    for (part_idx, part) in parts.iter().enumerate() {
        let is_last_of_seg = part_idx == part_count.saturating_sub(1);
        let next_is_gap = parts.get(part_idx + 1).is_some_and(|p| p.gap);
        let lower_exempt = part.independent || part.gap || is_last_of_seg || next_is_gap;

        let duration = part.duration.get();
        // A tolerance for the exact-float comparison: the durations come from
        // decimal text with three fractional digits, so a mathematically
        // equal bound can land one ULP off. 1 ms is far below any real
        // violation and far above the representation error.
        let over_upper = duration > upper + PART_DURATION_EPSILON_S;
        let under_lower = !lower_exempt && duration < lower - PART_DURATION_EPSILON_S;
        if !over_upper && !under_lower {
            continue;
        }

        let bound = if over_upper {
            alloc::format!("must be <= {upper:.3} (PART-TARGET)")
        } else {
            alloc::format!("must be >= {lower:.3} (85% of PART-TARGET)")
        };
        let exempt_note = if over_upper && lower_exempt {
            " — the §4.4.4.9 exemptions relax only the 85% floor, not this upper bound"
        } else {
            ""
        };
        report.push(Finding::new(
            Severity::Error,
            Location::new(segment_number, 0),
            "hls-part-duration-range",
            alloc::format!(
                "Partial segment {part_idx} of segment {segment_number} has duration {duration} \
                 — {bound} per §4.4.4.9{exempt_note}",
            ),
        ));
    }
}

// ---------------------------------------------------------------------------
// Master Playlist structured validation (RFC 8216bis)
// ---------------------------------------------------------------------------

fn validate_master_playlist(_pl: &broadcast_hls::MasterPlaylist, _report: &mut Report) {
    // Master playlist rules: the structured parser already validates required
    // attributes. Future rules (cross-referential integrity) can be added here.
}

// ---------------------------------------------------------------------------
// DATERANGE line checks on original text (RFC 8216bis §4.4.5.1)
// ---------------------------------------------------------------------------

fn check_daterange_lines(text: &str, report: &mut Report) {
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("#EXT-X-DATERANGE:")
            && timed_metadata::DateRange::parse_tag_line(trimmed).is_err()
        {
            report.push(Finding::new(
                Severity::Error,
                Location::new(i + 1, 0),
                "hls-malformed-daterange",
                "Malformed #EXT-X-DATERANGE line — §4.4.5.1",
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Legacy line-based checks (fallback when structured parse fails, and for
// rules the structured model doesn't fully enforce on the raw text).
// ---------------------------------------------------------------------------

fn legacy_line_checks(text: &str, report: &mut Report) {
    let lines: Vec<&str> = text.lines().collect();

    // hls-missing-extm3u — §4.4.1.1
    let first_non_empty = lines.iter().find(|l| !l.trim().is_empty());
    match first_non_empty {
        Some(line) if line.trim() == "#EXTM3U" => { /* ok */ }
        _ => {
            report.push(Finding::new(
                Severity::Error,
                Location::new(1, 0),
                "hls-missing-extm3u",
                "First non-empty line must be exactly '#EXTM3U' — §4.4.1.1",
            ));
        }
    }

    // Collect TARGETDURATION and EXTINF
    let mut has_targetduration = false;
    let mut targetduration_val: u64 = 0;
    let mut extinf_line_nums: Vec<usize> = Vec::new();
    let mut extinf_durations: Vec<f64> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let line_num = i + 1;
        let trimmed = line.trim();

        if trimmed.starts_with("#EXT-X-TARGETDURATION:") {
            has_targetduration = true;
            if let Some(val_str) = trimmed.strip_prefix("#EXT-X-TARGETDURATION:") {
                targetduration_val = val_str.trim().parse::<u64>().unwrap_or(0);
            }
        }

        if trimmed.starts_with("#EXTINF:") {
            extinf_line_nums.push(line_num);
            if let Some(dur_str) = trimmed.strip_prefix("#EXTINF:") {
                let dur = dur_str.split(',').next().unwrap_or("0");
                let parsed: f64 = dur.trim().parse().unwrap_or(0.0);
                extinf_durations.push(parsed);
            } else {
                extinf_durations.push(0.0);
            }
        }
    }

    // hls-missing-targetduration — §4.4.3.1
    if !extinf_line_nums.is_empty() && !has_targetduration {
        report.push(Finding::new(
            Severity::Error,
            Location::new(1, 0),
            "hls-missing-targetduration",
            "Media playlist with EXTINF entries must include #EXT-X-TARGETDURATION — §4.4.3.1",
        ));
    }

    // hls-extinf-exceeds-target — §4.4.3.1
    if has_targetduration {
        for (idx, &dur) in extinf_durations.iter().enumerate() {
            let rounded = (dur + 0.5) as u64;
            if rounded > targetduration_val {
                report.push(Finding::new(
                    Severity::Error,
                    Location::new(extinf_line_nums[idx], 0),
                    "hls-extinf-exceeds-target",
                    alloc::format!(
                        "EXTINF duration {dur} (rounded to {rounded}) exceeds TARGETDURATION {targetduration_val} — §4.4.3.1",
                    ),
                ));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum fraction of `PART-TARGET` a part duration must be (§4.4.4.9).
const HLS_PART_DURATION_MIN_FRACTION: f64 = 0.85;

/// Tolerance for the part-duration bound comparisons, in seconds — see
/// [`check_part_durations`].
const PART_DURATION_EPSILON_S: f64 = 0.001;
