//! DASH MPD manifest validator (ISO/IEC 23009-1).
//!
//! Accepts an MPD XML text and a [`Report`](crate::Report), appending findings
//! for spec violations. Reuses [`transmux::Mpd::parse`] as the structured parse
//! layer — no regex-based or manual XML re-parsing.
//!
//! # Rule IDs
//!
//! | ID | Severity | Description | Clause |
//! |---|---|---|---|
//! | `dash-parse-error` | Error | MPD XML fails to parse | §5.3 |
//! | `dash-representation-id-duplicate` | Error | Duplicate `@id` across Representations | §5.3.5.2 |
//! | `dash-segment-timeline-monotonic` | Error | SegmentTimeline `t` values not monotonic | §5.3.9.6 |
//! | `dash-static-mpd-missing-duration` | Error | Static MPD missing `mediaPresentationDuration` | §5.3.1.2 |
//! | `dash-dynamic-mpd-no-availability-start` | Error | Dynamic MPD missing `availabilityStartTime` | §5.3.1.2 |
//! | `dash-bandwidth-mismatch` | Warning | Representation bandwidth contradicts AdaptationSet peers | §5.3.5.2 |
//! | `dash-period-no-adaptation-sets` | Error | Period has no AdaptationSets | §5.3.2 |

use crate::report::{Finding, Location, Report, Severity};
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

/// Validate a DASH MPD XML text, appending findings for each violation.
pub fn check_dash_mpd(text: &str, report: &mut Report) {
    let mpd = match transmux::Mpd::parse(text) {
        Ok(mpd) => mpd,
        Err(err) => {
            report.push(Finding::new(
                Severity::Error,
                Location::new(1, 0),
                "dash-parse-error",
                alloc::format!("MPD XML failed to parse: {err}"),
            ));
            return;
        }
    };

    // dash-static-mpd-missing-duration — §5.3.1.2 Table 3
    if mpd.mpd_type == transmux::MpdType::Static && mpd.media_presentation_duration.is_none() {
        report.push(Finding::new(
            Severity::Error,
            Location::new(1, 0),
            "dash-static-mpd-missing-duration",
            "Static MPD is missing required @mediaPresentationDuration — §5.3.1.2 Table 3",
        ));
    }

    // dash-dynamic-mpd-no-availability-start — §5.3.1.2 Table 3
    if mpd.mpd_type == transmux::MpdType::Dynamic && mpd.availability_start_time.is_none() {
        report.push(Finding::new(
            Severity::Error,
            Location::new(1, 0),
            "dash-dynamic-mpd-no-availability-start",
            "Dynamic MPD is missing required @availabilityStartTime — §5.3.1.2 Table 3",
        ));
    }

    // Per-Period validation
    for (period_idx, period) in mpd.periods.iter().enumerate() {
        // dash-period-no-adaptation-sets — §5.3.2
        if period.adaptation_sets.is_empty() {
            report.push(Finding::new(
                Severity::Error,
                Location::new(period_idx + 1, 0),
                "dash-period-no-adaptation-sets",
                alloc::format!(
                    "Period '{}' has no AdaptationSets — §5.3.2",
                    period.id.as_deref().unwrap_or("(unnamed)")
                ),
            ));
            continue;
        }

        for (as_idx, aset) in period.adaptation_sets.iter().enumerate() {
            let as_key = alloc::format!("{period_idx}.{as_idx}");

            // dash-representation-id-duplicate — §5.3.5.2
            check_representation_id_duplicates(aset, &as_key, report);

            // dash-bandwidth-mismatch — §5.3.5.2: check bandwidth
            // consistency within an AdaptationSet
            check_bandwidth_consistency(aset, &as_key, report);

            // Per-Representation SegmentTimeline validation
            for (r_idx, repr) in aset.representations.iter().enumerate() {
                let r_key = alloc::format!("{as_key}.{r_idx}");
                if let Some(ref st) = repr.segment_template {
                    if let Some(ref timeline) = st.timeline {
                        validate_segment_timeline(timeline, st.timescale, &r_key, report);
                    }
                }
            }
        }
    }
}

/// Check for duplicate `@id` values across Representations in an AdaptationSet.
/// §5.3.5.2: `@id` must be unique within the Period.
fn check_representation_id_duplicates(
    aset: &transmux::AdaptationSet,
    as_key: &str,
    report: &mut Report,
) {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for repr in &aset.representations {
        if !seen.insert(&repr.id) {
            report.push(Finding::new(
                Severity::Error,
                Location::new(1, 0),
                "dash-representation-id-duplicate",
                alloc::format!(
                    "AdaptationSet {as_key}: duplicate Representation @id=\"{}\" — §5.3.5.2",
                    repr.id,
                ),
            ));
        }
    }
}

/// Check bandwidth consistency within an AdaptationSet — representations
/// within the same AdaptationSet are alternate encodings of the same content
/// and should have distinct, ordered bandwidths.
fn check_bandwidth_consistency(aset: &transmux::AdaptationSet, as_key: &str, report: &mut Report) {
    if aset.representations.len() < 2 {
        return;
    }

    // Warn if a Representation's bandwidth is <= half or >= double the
    // AdaptationSet's median — a strong signal of a mislabeled AdaptationSet.
    let mut bandwidths: Vec<u64> = aset.representations.iter().map(|r| r.bandwidth).collect();
    bandwidths.sort_unstable();
    let median = bandwidths[bandwidths.len() / 2];
    if median == 0 {
        return;
    }

    for repr in &aset.representations {
        if repr.bandwidth > 0 {
            let ratio = repr.bandwidth as f64 / median as f64;
            if !(DASH_BANDWIDTH_RATIO_LOW..=DASH_BANDWIDTH_RATIO_HIGH).contains(&ratio) {
                report.push(Finding::new(
                    Severity::Warning,
                    Location::new(1, 0),
                    "dash-bandwidth-mismatch",
                    alloc::format!(
                        "AdaptationSet {as_key}: Representation @id=\"{}\" bandwidth {} is {ratio:.1}x the set median {median} — possible misassignment",
                        repr.id, repr.bandwidth,
                    ),
                ));
            }
        }
    }
}

/// Validate SegmentTimeline for monotonic time progression — §5.3.9.6.
fn validate_segment_timeline(
    timeline: &transmux::SegmentTimeline,
    timescale: u64,
    r_key: &str,
    report: &mut Report,
) {
    let mut prev_end_time: Option<u64> = None;

    for (s_idx, s) in timeline.segments.iter().enumerate() {
        let start_time = s.t.unwrap_or_else(|| {
            prev_end_time.expect("first S element must have @t — §5.3.9.6.2 Table 21")
        });

        // Each S element's @t (explicit or computed) must be >= previous end time.
        if let Some(prev_end) = prev_end_time {
            if start_time < prev_end {
                let prev_str = timescale_str(prev_end, timescale);
                let t_str = timescale_str(start_time, timescale);
                report.push(Finding::new(
                    Severity::Error,
                    Location::new(s_idx + 1, 0),
                    "dash-segment-timeline-monotonic",
                    alloc::format!(
                        "SegmentTimeline {r_key}: <S t={t_str}> starts before previous segment ends (prev_end={prev_str}) — §5.3.9.6",
                    ),
                ));
            }
        }

        // Calculate this S element's end time (for the next S's implicit @t).
        // Each repeat adds @d to the time.
        let repeat_count: u64 = if s.r >= 0 {
            s.r as u64 + 1
        } else {
            // Negative @r is not valid per spec, but don't crash.
            1
        };
        prev_end_time = Some(start_time.saturating_add(s.d.saturating_mul(repeat_count)));
    }
}

/// Format a timescale value with the timescale for readability.
fn timescale_str(value: u64, timescale: u64) -> alloc::string::String {
    if timescale == 0 || timescale == 1 {
        alloc::format!("{value}")
    } else {
        let secs = value as f64 / timescale as f64;
        alloc::format!("{value} ({secs:.3}s)")
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Low bandwidth ratio threshold for the bandwidth-mismatch check.
const DASH_BANDWIDTH_RATIO_LOW: f64 = 0.4;

/// High bandwidth ratio threshold for the bandwidth-mismatch check.
const DASH_BANDWIDTH_RATIO_HIGH: f64 = 2.5;
