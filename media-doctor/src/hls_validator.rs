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
//! | `hls-part-duration-range` | Warning | Part duration outside [85%, 100%] of PART-TARGET | §4.4.4.9 |
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

    // Try parsing as a Media Playlist first.
    match broadcast_hls::MediaPlaylist::parse(text) {
        Ok(media) => {
            validate_media_playlist(&media, report);
            // Also run EXTINF/TARGETDURATION checks on the original text
            // since the parser already validates these structurally, but
            // line-based checks catch edge cases on the raw text.
            legacy_line_checks(text, report);
        }
        Err(_media_err) => {
            // Try as a Master (multivariant) Playlist.
            match broadcast_hls::MasterPlaylist::parse(text) {
                Ok(master) => {
                    validate_master_playlist(&master, report);
                }
                Err(_master_err) => {
                    // Both structured parses failed. Report the parse failure
                    // and fall back to line-based checks for non-parse rules.
                    report.push(Finding::new(
                        Severity::Error,
                        Location::new(1, 0),
                        "hls-parse-error",
                        "Playlist failed to parse as a valid HLS Media or Multivariant Playlist",
                    ));
                    legacy_line_checks(text, report);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Media Playlist structured validation (RFC 8216bis)
// ---------------------------------------------------------------------------

fn validate_media_playlist(pl: &broadcast_hls::MediaPlaylist, report: &mut Report) {
    // hls-preload-hint-with-endlist — §4.4.5.3
    if pl.endlist {
        if let Some(ref ll) = pl.low_latency {
            if ll.preload_hint_part.is_some() {
                report.push(Finding::new(
                    Severity::Error,
                    Location::new(1, 0),
                    "hls-preload-hint-with-endlist",
                    "Playlist carries EXT-X-ENDLIST and EXT-X-PRELOAD-HINT — §4.4.5.3: a playlist with ENDLIST MUST NOT contain PRELOAD-HINT",
                ));
            }
        }
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
    // Part duration MUST be <= Part Target Duration, and >= 85% of Part Target
    // Duration (with exceptions: INDEPENDENT=YES, GAP=YES, followed by GAP=YES,
    // or final part of a parent segment).
    if let Some(ref ll) = pl.low_latency {
        let part_target = ll.part_target;
        if part_target > 0.0 {
            let lower = part_target * HLS_PART_DURATION_MIN_FRACTION;
            let upper = part_target;

            for (seg_idx, seg) in pl.segments.iter().enumerate() {
                let part_count = seg.parts.len();
                for (part_idx, part) in seg.parts.iter().enumerate() {
                    let is_last_of_seg = part_idx == part_count.saturating_sub(1);
                    let next_is_gap = seg.parts.get(part_idx + 1).is_some_and(|p| p.gap);

                    if !part.independent
                        && !part.gap
                        && !is_last_of_seg
                        && !next_is_gap
                        && (part.duration < lower || part.duration > upper)
                    {
                        report.push(Finding::new(
                            Severity::Error,
                            Location::new(seg_idx + 1, 0),
                            "hls-part-duration-range",
                            alloc::format!(
                                "Partial segment {part_idx} of segment {seg_idx} has duration {} — must be in [{lower:.3}, {upper:.3}] per §4.4.4.9",
                                part.duration,
                            ),
                        ));
                    }
                }
            }
        }
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
