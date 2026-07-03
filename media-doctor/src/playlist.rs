//! HLS playlist validator (RFC 8216) — free-function `check_playlist`.
//!
//! Checks a playlist for well-formedness rules: presence of `#EXTM3U`,
//! `#EXT-X-TARGETDURATION`, EXTINF durations not exceeding target, and
//! well-formed `#EXT-X-DATERANGE:` lines.

use crate::report::{Finding, Location, Report, Severity};
use alloc::vec::Vec;

/// Validate an HLS playlist, appending findings for each violation.
///
/// Line numbers in [`Location`] are 1-based; `pid` is always 0 (text input).
pub fn check_playlist(text: &str, report: &mut Report) {
    let lines: Vec<&str> = text.lines().collect();

    // First pass: collect TARGETDURATION and every EXTINF (line + duration).
    let mut has_targetduration = false;
    let mut targetduration_val: u64 = 0;
    let mut extinf_line_nums: Vec<usize> = Vec::new();
    let mut extinf_durations: Vec<f64> = Vec::new();

    for (i, line) in lines.iter().enumerate() {
        let line_num = i + 1; // 1-based
        let trimmed = line.trim();

        if trimmed.starts_with("#EXT-X-TARGETDURATION:") {
            has_targetduration = true;
            // Parse the integer after the colon
            if let Some(val_str) = trimmed.strip_prefix("#EXT-X-TARGETDURATION:") {
                targetduration_val = val_str.trim().parse::<u64>().unwrap_or(0);
            }
        }

        if trimmed.starts_with("#EXTINF:") {
            extinf_line_nums.push(line_num);
            if let Some(dur_str) = trimmed.strip_prefix("#EXTINF:") {
                // Duration is the part before the first comma
                let dur = dur_str.split(',').next().unwrap_or("0");
                let parsed: f64 = dur.trim().parse().unwrap_or(0.0);
                extinf_durations.push(parsed);
            } else {
                extinf_durations.push(0.0);
            }
        }
    }

    // Rule 1: hls-missing-extm3u — first non-empty line must be exactly "#EXTM3U"
    let first_non_empty = lines.iter().find(|l| !l.trim().is_empty());
    match first_non_empty {
        Some(line) if line.trim() == "#EXTM3U" => { /* ok */ }
        _ => {
            report.push(Finding::new(
                Severity::Error,
                Location::new(1, 0),
                "hls-missing-extm3u",
                "First non-empty line must be exactly '#EXTM3U'",
            ));
        }
    }

    // Rule 2: hls-missing-targetduration
    // A Media Playlist (has ≥1 #EXTINF) MUST contain #EXT-X-TARGETDURATION.
    if !extinf_line_nums.is_empty() && !has_targetduration {
        report.push(Finding::new(
            Severity::Error,
            Location::new(1, 0),
            "hls-missing-targetduration",
            "Media playlist with EXTINF entries must include #EXT-X-TARGETDURATION",
        ));
    }

    // Rule 3: hls-extinf-exceeds-target
    // Each EXTINF duration rounded to nearest int must be ≤ TARGETDURATION.
    if has_targetduration {
        for (idx, &dur) in extinf_durations.iter().enumerate() {
            // Round to nearest integer: add 0.5 and truncate (works in no_std)
            let rounded = (dur + 0.5) as u64;
            if rounded > targetduration_val {
                report.push(Finding::new(
                    Severity::Error,
                    Location::new(extinf_line_nums[idx], 0),
                    "hls-extinf-exceeds-target",
                    alloc::format!(
                        "EXTINF duration {dur} (rounded to {rounded}) exceeds TARGETDURATION {targetduration_val}"
                    ),
                ));
            }
        }
    }

    // Rule 4: hls-malformed-daterange
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("#EXT-X-DATERANGE:")
            && timed_metadata::DateRange::parse_tag_line(trimmed).is_err()
        {
            report.push(Finding::new(
                Severity::Error,
                Location::new(i + 1, 0),
                "hls-malformed-daterange",
                "Malformed #EXT-X-DATERANGE line",
            ));
        }
    }
}
