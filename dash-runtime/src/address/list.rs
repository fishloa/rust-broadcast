//! `SegmentList` entries (ISO/IEC 23009-1 5.3.9.3, Table 19).

use super::AddressError;
use super::timeline::TimelineIndex;
use crate::mpd::inherit::ListInfo;
use crate::mpd::model::{ByteRange, SegmentUrl};

/// One listed segment with its timing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListSegment {
    /// Segment number.
    pub number: u64,
    /// Start on the timeline in ticks: `i * @duration`, or the timeline's `t`.
    pub time: u64,
    /// Duration in ticks.
    pub duration: u64,
    /// `@media` reference (empty: the BaseURL itself).
    pub reference: String,
    /// `@mediaRange`.
    pub range: Option<ByteRange>,
    /// The entry's own `SegmentURL`.
    pub entry: SegmentUrl,
}

/// Expand a `SegmentList`. `period_duration_ticks` shortens the last segment
/// when the Period ends inside it (5.3.9.5.3).
///
/// # Errors
/// [`AddressError`] when entries cannot be timed or are empty.
pub fn expand(
    info: &ListInfo,
    timeline: Option<&TimelineIndex>,
    period_duration_ticks: Option<u64>,
) -> Result<Vec<ListSegment>, AddressError> {
    let urls = &info.urls;
    let multi = &info.multi;
    let mut out = Vec::with_capacity(urls.len());
    if let Some(t) = timeline {
        if t.len() != urls.len() as u64 {
            return Err(AddressError::ListTimelineMismatch {
                urls: urls.len(),
                segments: t.len(),
            });
        }
    } else if multi.duration.is_none() && urls.len() > 1 {
        return Err(AddressError::MissingDuration);
    }
    for (i, entry) in urls.iter().enumerate() {
        if entry.media.is_none() && entry.media_range.is_none() {
            return Err(AddressError::EmptySegmentUrl);
        }
        let idx = i as u64;
        let (number, time, duration) = match timeline.and_then(|t| t.segment(idx)) {
            Some(seg) => (seg.number, seg.time, seg.duration),
            None => {
                let d = multi.duration.unwrap_or(period_duration_ticks.unwrap_or(0));
                let start = idx.checked_mul(d).ok_or(AddressError::Overflow)?;
                let last = i + 1 == urls.len();
                let dur = match (last, multi.duration, period_duration_ticks) {
                    (true, Some(nominal), Some(total)) => {
                        total.saturating_sub(start).min(nominal)
                    }
                    _ => d,
                };
                (multi.start_number.saturating_add(idx), start, dur)
            }
        };
        out.push(ListSegment {
            number,
            time,
            duration,
            reference: entry.media.clone().unwrap_or_default(),
            range: entry.media_range,
            entry: entry.clone(),
        });
    }
    Ok(out)
}
