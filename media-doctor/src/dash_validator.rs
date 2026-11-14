//! DASH MPD manifest validator (ISO/IEC 23009-1).
//!
//! Accepts an MPD XML text and a [`Report`](crate::Report), appending findings
//! for spec violations. Reuses [`broadcast_dash::Mpd::parse`] as the
//! structured parse layer — no regex-based or manual XML re-parsing.
//!
//! # Rule IDs
//!
//! | ID | Severity | Description | Clause |
//! |---|---|---|---|
//! | `dash-parse-error` | Error | MPD XML fails to parse | §5.3 |
//! | `dash-representation-id-duplicate` | Error | Duplicate `@id` across Representations | ISO/IEC 23009-1:2012 §5.3.5.2 Table 7 (@id shall be unique within a Period) |
//! | `dash-segment-timeline-monotonic` | Error | SegmentTimeline `t` values not monotonic | ISO/IEC 23009-1:2012 §5.3.9.6.2 (@t must be ≥ previous S end time, defaulting to 0 for first S) |
//! | `dash-static-mpd-missing-duration` | Error | Static MPD missing `mediaPresentationDuration` while neither `minimumUpdatePeriod` nor the last `Period@duration` is present | ISO/IEC 23009-1:2012 §5.3.1.2 Table 3 (`private/specs-verified/dash/23009-1/05-3-1-hierarchy.md`) |
//! | `dash-dynamic-mpd-no-availability-start` | Error | Dynamic MPD missing `availabilityStartTime` | ISO/IEC 23009-1:2012 §5.3.1.2 Table 3 (CM — must be present for @type='dynamic') |
//! | `dash-period-no-adaptation-sets` | Error | Period has no AdaptationSets | §5.3.2 |

use broadcast_common::timescale::Timescale;
use crate::report::{Finding, Location, Report, Severity};
use alloc::collections::BTreeMap;
use broadcast_dash::PresentationType;
use broadcast_dash::inherit::{SegmentInfo, resolve_segment_info};

/// The parse bounds this validator reads with: the hostile-input defaults
/// throughout, except that **nothing is ever too large to validate** — a bound
/// trip would report a well-formed manifest as unparseable. Per-rule numeric
/// over-range is a rule's own finding (`dash-segment-timeline-r-range`), not a
/// parse failure.
fn relaxed_limits() -> broadcast_dash::limits::Limits {
    let mut limits = broadcast_dash::limits::Limits::default();
    limits.max_bytes = usize::MAX;
    limits.max_depth = usize::MAX;
    limits.max_elements = usize::MAX;
    limits.max_attribute_bytes = usize::MAX;
    limits.max_attributes = usize::MAX;
    limits.max_periods = usize::MAX;
    limits.max_adaptation_sets = usize::MAX;
    limits.max_representations = usize::MAX;
    limits.max_timeline_entries = usize::MAX;
    limits.max_base_urls = usize::MAX;
    limits.max_segment_urls = usize::MAX;
    limits
}

/// Parse an MPD into dash-runtime's model, recording the findings a refused
/// document earns. `None` means a finding was already recorded.
fn parse_mpd(text: &str, report: &mut Report) -> Option<broadcast_dash::Mpd> {
    match broadcast_dash::Mpd::parse_with(text.as_bytes(), &relaxed_limits()) {
        Ok(mpd) => Some(mpd),
        Err(err) => {
            report.push(Finding::new(
                Severity::Error,
                Location::new(1, 0),
                "dash-parse-error",
                alloc::format!("MPD XML failed to parse: {err}"),
            ));
            None
        }
    }
}

/// Validate a DASH MPD XML text, appending findings for each violation.
pub fn check_dash_mpd(text: &str, report: &mut Report) {
    let mpd = match parse_mpd(text, report) {
        Some(mpd) => mpd,
        None => return,
    };

    // dash-static-mpd-missing-duration — ISO/IEC 23009-1:2012 §5.3.1.2 Table 3
    // CM (Conditionally Mandatory) — must be present for @type='static'.
    let last_period_duration = mpd.periods.last().and_then(|p| p.duration);
    if mpd.kind == PresentationType::Static
        && mpd.media_presentation_duration.is_none()
        && mpd.minimum_update_period.is_none()
        && last_period_duration.is_none()
    {
        report.push(Finding::new(
            Severity::Error,
            Location::new(1, 0),
            "dash-static-mpd-missing-duration",
            "Static MPD is missing required @mediaPresentationDuration — ISO/IEC 23009-1:2012 §5.3.1.2 Table 3 (CM: mandatory for @type='static')",
        ));
    }

    // dash-dynamic-mpd-no-availability-start — ISO/IEC 23009-1:2012 §5.3.1.2 Table 3
    // CM (Conditionally Mandatory) — must be present for @type='dynamic'.
    if mpd.kind == PresentationType::Dynamic && mpd.availability_start_time_us.is_none() {
        report.push(Finding::new(
            Severity::Error,
            Location::new(1, 0),
            "dash-dynamic-mpd-no-availability-start",
            "Dynamic MPD is missing required @availabilityStartTime — §5.3.1.2 Table 3",
        ));
    }

    // Per-Period validation
    for (period_idx, period) in mpd.periods.iter().enumerate() {
        // dash-period-no-adaptation-sets — §5.3.2. An ed.6 list Period names
        // an imported MPD instead (`ImportedMPD`), which is all it needs.
        if period.adaptation_sets.is_empty() && period.imported_mpd.is_none() {
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

        // dash-representation-id-duplicate — §5.3.5.2. Scoped to the
        // *Period*: `@id` must be unique across all its AdaptationSets.
        check_representation_id_duplicates(period, period_idx, report);

        for (as_idx, aset) in period.adaptation_sets.iter().enumerate() {
            let as_key = alloc::format!("{period_idx}.{as_idx}");

            // Per-Representation SegmentTimeline validation
            for (r_idx, repr) in aset.representations.iter().enumerate() {
                let r_key = alloc::format!("{as_key}.{r_idx}");
                if let Some((timeline, timescale)) = effective_timeline(period, aset, repr) {
                    validate_segment_timeline(timeline, timescale, &r_key, report);
                }
            }
        }
    }
}

/// The `SegmentTimeline` a Representation is addressed by, with the timescale
/// it is read in, each S validated once.
///
/// A `SegmentTimeline` nested in an AdaptationSet's `SegmentTemplate` governs
/// every Representation there that declares no timeline of its own (5.3.9.2.1),
/// and one timeline is declarable once, so it has one resolved timeline
/// identity whichever chain it is read through: it is checked once, against
/// its declaration, and each Representation without a timeline of its own is
/// then not checked again.
fn effective_timeline<'a>(
    period: &'a broadcast_dash::Period,
    aset: &'a broadcast_dash::AdaptationSet,
    repr: &'a broadcast_dash::Representation,
) -> Option<(&'a broadcast_dash::SegmentTimeline, Timescale)> {
    let declared = repr
        .segment
        .template
        .as_ref()
        .and_then(|t| t.multi.timeline.as_ref());
    if let Some(timeline) = declared {
        let Ok(SegmentInfo::Template(info)) = resolve_segment_info(period, aset, repr) else {
            return None;
        };
        return Some((timeline, info.multi.timescale));
    }
    let template = aset.segment.template.as_ref()?;
    let inherited = template.multi.timeline.as_ref()?;
    Some((inherited, template.multi.base.effective_timescale()))
}

/// Check for duplicate `@id` values across **every** Representation in one
/// Period.
///
/// ISO/IEC 23009-1:2012 §5.3.5.2 Table 7: `@id` "shall be unique within a
/// Period" — not within an AdaptationSet. Checking per AdaptationSet missed
/// the realistic error entirely: the same `@id` reused by two different
/// AdaptationSets of the same Period (audit MD-W12).
fn check_representation_id_duplicates(
    period: &broadcast_dash::Period,
    period_idx: usize,
    report: &mut Report,
) {
    // `@id` must be unique within a Period (ISO/IEC 23009-1 §5.3.5.2 Table 7)
    // *unless* the Representations are functionally identical, in which case
    // the duplicate is redundant rather than ambiguous — a Warning, not an
    // Error.
    let mut seen: BTreeMap<&str, &broadcast_dash::Representation> = BTreeMap::new();
    for (as_idx, aset) in period.adaptation_sets.iter().enumerate() {
        for repr in &aset.representations {
            let Some(previous) = seen.insert(&repr.id, repr) else {
                continue;
            };
            let identical = functionally_identical(previous, repr);
            report.push(Finding::new(
                if identical {
                    Severity::Warning
                } else {
                    Severity::Error
                },
                Location::new(1, 0),
                "dash-representation-id-duplicate",
                alloc::format!(
                    "Period {period_idx} (AdaptationSet {as_idx}): duplicate Representation @id=\"{}\" — ISO/IEC 23009-1 §5.3.5.2 requires @id to be unique within a Period{}",
                    repr.id,
                    if identical {
                        " (the two are functionally identical, so this is redundant rather than ambiguous)"
                    } else {
                        ""
                    },
                ),
            ));
        }
    }
}

/// Whether two Representations carrying the same `@id` are functionally
/// identical — same codec, bandwidth, geometry, framing and addressing, i.e.
/// the same content described twice (ISO/IEC 23009-1 §5.3.5.2's exception).
fn functionally_identical(
    a: &broadcast_dash::Representation,
    b: &broadcast_dash::Representation,
) -> bool {
    a.bandwidth == b.bandwidth
        && a.common.codecs == b.common.codecs
        && a.common.width == b.common.width
        && a.common.height == b.common.height
        && a.common.frame_rate == b.common.frame_rate
        && a.common.audio_sampling_rate == b.common.audio_sampling_rate
        && a.common.mime_type == b.common.mime_type
        && a.base_urls == b.base_urls
        && segment_template(a) == segment_template(b)
}

/// The `SegmentTemplate` a Representation declares itself (5.3.9.4).
fn segment_template(
    repr: &broadcast_dash::Representation,
) -> Option<&broadcast_dash::SegmentTemplate> {
    repr.segment.template.as_ref()
}

/// Validate SegmentTimeline for monotonic time progression.
///
/// ISO/IEC 23009-1:2012 §5.3.9.6.2 Table 17: `@t` defaults to 0 for the first
/// `S` element; for subsequent elements, `@t` defaults to the previous element's
/// end time. The explicit/computed `@t` must be ≥ the previous element's end
/// time (a gap is a legal discontinuity; backward time is not).
fn validate_segment_timeline(
    timeline: &broadcast_dash::SegmentTimeline,
    timescale: Timescale,
    r_key: &str,
    report: &mut Report,
) {
    let mut prev_end_time: Option<u64> = None;

    for (s_idx, s) in timeline.entries.iter().enumerate() {
        let start_time = s.t.unwrap_or_else(|| {
            // §5.3.9.6.2: If @t is not present, it defaults to 0 for the first S
            // element, and to the previous S's end time for subsequent elements.
            prev_end_time.unwrap_or(0)
        });

        // Each S element's @t (explicit or computed) must be >= previous end time.
        if let Some(prev_end) = prev_end_time
            && start_time < prev_end
        {
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

        // `@r` below -1 is not defined by the spec: Table 17 gives
        // `@r = -1` the meaning "repeat until the next S element or the end
        // of the Period", and every other negative value is out of range.
        if s.r < -1 {
            report.push(Finding::new(
                Severity::Error,
                Location::new(s_idx + 1, 0),
                "dash-segment-timeline-r-range",
                alloc::format!(
                    "SegmentTimeline {r_key}: <S r={}> is out of range — Table 17 defines                      @r >= -1 (with -1 meaning repeat until the next S element or the end                      of the Period)",
                    s.r,
                ),
            ));
        }

        // Calculate this S element's end time (for the next S's implicit @t).
        // Each repeat adds @d to the time (ISO/IEC 23009-1 §5.3.9.6.2
        // Table 17: the S element repeats `@r + 1` times).
        //
        // `@r = -1` is *legal* and means "repeat until the next S element or
        // the end of the Period" (§5.3.9.6.2, Table 17) — not a fixed count.
        // Its end time is therefore undetermined from this element alone, so
        // no end is recorded and the next S element's explicit `@t` (which
        // the spec requires in that case) governs instead of a guessed one.
        let repeat_count = u64::try_from(s.r).ok().map(|r| r.saturating_add(1));
        if let Some(repeat_count) = repeat_count {
            prev_end_time = Some(start_time.saturating_add(s.d.saturating_mul(repeat_count)));
        } else {
            prev_end_time = None;
        }
    }
}

/// Format a timescale value with the timescale for readability.
fn timescale_str(value: u64, timescale: Timescale) -> alloc::string::String {
    if timescale.get() == 1 {
        alloc::format!("{value}")
    } else {
        let secs = value as f64 / f64::from(timescale.get());
        alloc::format!("{value} ({secs:.3}s)")
    }
}
