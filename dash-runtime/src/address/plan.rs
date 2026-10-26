//! The lazy Segment plan of one Representation within one Period.
//!
//! Formulas (all integer, `i128` microseconds; ticks via `@timescale`, which
//! defaults to 1):
//!
//! * `$Number$` with `@duration`: `MPDstart(n) = (n - startNumber) * d`,
//!   `n = startNumber + k - 1` (5.3.9.5.3; the printed `(kstart-1)` variant
//!   is a misprint settled by the real captures, DESIGN section 11 item 1).
//!   The last segment of a Period is `PeriodEnd - start` (A.3.3 case 1);
//!   `@endNumber` caps the count; neither `@duration` nor a timeline is one
//!   segment starting at 0 (A.3.3 case 3).
//! * Timeline: `$Time$` is the raw `S@t` (decision D3: the standard's three
//!   statements disagree when `@presentationTimeOffset != 0`); the
//!   Period-relative start is `(time - PTO) / timescale` (5.3.9.6.1).
//! * Availability (dynamic only): [`crate::timing::availability`].

use transmux::SegmentIndexBox;

use super::AddressError;
use super::base_index::{self, IndexedSegment};
use super::list;
use super::template::{self, TemplateFault, TemplateUse};
use super::timeline::TimelineIndex;
use crate::error::Unsupported;
use crate::mpd::inherit::{ListInfo, ResolvedMulti, SegmentInfo, TemplateInfo};
use crate::mpd::model::{BaseInfo, ByteRange, UrlType};
use crate::timing::availability::{
    init_available_from, segment_available_from_ticks, segment_available_until_ticks, ticks_to_us,
};

const MICROS: i128 = 1_000_000;

fn us_to_ticks_ceil(us: i128, timescale: u64) -> u64 {
    let n = us.max(0).saturating_mul(i128::from(timescale));
    let t = n.div_euclid(MICROS) + i128::from(n.rem_euclid(MICROS) != 0);
    u64::try_from(t).unwrap_or(u64::MAX)
}

/// What a dynamic MPD adds to the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DynamicContext {
    /// `MPD@availabilityStartTime` as Unix microseconds.
    pub availability_start_us: i128,
    /// The effective `@timeShiftBufferDepth` (Representation level over MPD
    /// level), microseconds; `None` means unbounded.
    pub time_shift_buffer_depth_us: Option<i128>,
    /// Sum of `@availabilityTimeOffset` over every level that built the URL,
    /// microseconds; zero unless the caller opted into early requests.
    pub availability_time_offset_us: i128,
    /// `FetchTime + minimumUpdatePeriod` in UTC microseconds: where a trailing
    /// negative `@r` runs to when the Period has no known end (A.3.2).
    pub horizon_utc_us: Option<i128>,
}

impl DynamicContext {
    /// A context with no ATO and no horizon.
    #[must_use]
    pub fn new(availability_start_us: i128, time_shift_buffer_depth_us: Option<i128>) -> Self {
        Self {
            availability_start_us,
            time_shift_buffer_depth_us,
            availability_time_offset_us: 0,
            horizon_utc_us: None,
        }
    }
}

/// Everything [`SegmentPlan::build`] needs.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PlanInput<'a> {
    /// `Representation@id` (`$RepresentationID$`).
    pub representation_id: &'a str,
    /// `Representation@bandwidth` (`$Bandwidth$`).
    pub bandwidth: u64,
    /// The resolved Segment information.
    pub info: &'a SegmentInfo,
    /// PeriodStart in microseconds from the presentation start.
    pub period_start_us: i128,
    /// The Period end in the same domain, when known.
    pub period_end_us: Option<i128>,
    /// Dynamic-MPD context; `None` for a static MPD.
    pub dynamic: Option<DynamicContext>,
    /// The parsed `sidx` and the anchor offset (SegmentBase only).
    pub index: Option<(&'a SegmentIndexBox, u64)>,
}

impl<'a> PlanInput<'a> {
    /// A static plan starting at 0 with no known end.
    #[must_use]
    pub fn new(representation_id: &'a str, bandwidth: u64, info: &'a SegmentInfo) -> Self {
        Self {
            representation_id,
            bandwidth,
            info,
            period_start_us: 0,
            period_end_us: None,
            dynamic: None,
            index: None,
        }
    }
}

/// One Media Segment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SegmentRef {
    /// Zero-based position in the plan.
    pub index: u64,
    /// `$Number$`.
    pub number: u64,
    /// `$Time$` value in ticks.
    pub time_ticks: u64,
    /// Duration in ticks.
    pub duration_ticks: u64,
    /// Ticks per second.
    pub timescale: u64,
    /// Start relative to the Period start, microseconds
    /// (`(time - PTO) / timescale` for a timeline).
    pub start_us: i128,
    /// Duration in microseconds.
    pub duration_us: i128,
    /// The URL reference to resolve against the BaseURL chain; empty means the
    /// BaseURL itself.
    pub reference: String,
    /// The byte range, when the segment is a part of a file.
    pub byte_range: Option<ByteRange>,
    /// Availability start (dynamic MPD).
    pub available_from_us: Option<i128>,
    /// Availability end (dynamic MPD with a time-shift buffer).
    pub available_until_us: Option<i128>,
}

/// An Initialization or Index Segment address.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResourceRef {
    /// URL reference (empty: the BaseURL itself).
    pub reference: String,
    /// Byte range.
    pub byte_range: Option<ByteRange>,
    /// Availability start (dynamic MPD).
    pub available_from_us: Option<i128>,
    /// Availability end: that of the last Media Segment.
    pub available_until_us: Option<i128>,
}

#[derive(Debug, Clone)]
struct Fixed {
    number: u64,
    time: u64,
    start_ticks: i128,
    duration: u64,
    reference: String,
    range: Option<ByteRange>,
}

#[derive(Debug, Clone)]
enum Kind {
    Duration {
        d: u64,
        start_number: u64,
        count: Option<u64>,
        last_duration: u64,
    },
    Timeline(TimelineIndex),
    Fixed(Vec<Fixed>),
}

#[derive(Debug, Clone)]
struct Avail {
    ast_us: i128,
    ato_us: i128,
    tsb_us: Option<i128>,
}

#[derive(Debug, Clone)]
struct MediaTemplate {
    template: String,
    representation_id: String,
    bandwidth: u64,
}

/// The ordered Media Segments of one Representation in one Period.
#[derive(Debug, Clone)]
pub struct SegmentPlan {
    kind: Kind,
    timescale: u64,
    pto: u64,
    period_start_us: i128,
    avail: Option<Avail>,
    media: Option<MediaTemplate>,
    init: Option<(String, Option<ByteRange>)>,
    index: Option<(String, Option<ByteRange>)>,
    len_cap: Option<u64>,
}

fn fault(template: &str, f: TemplateFault) -> AddressError {
    AddressError::InvalidTemplate {
        template: template.to_owned(),
        fault: f,
    }
}

fn reject_sequences(used: TemplateUse) -> Result<(), AddressError> {
    if used.sub_number {
        Err(AddressError::Unsupported(Unsupported::SegmentSequence))
    } else {
        Ok(())
    }
}

fn url_ref(u: &UrlType) -> (String, Option<ByteRange>) {
    (u.source_url.clone().unwrap_or_default(), u.range)
}

fn period_end_media_ticks(
    multi: &ResolvedMulti,
    input: &PlanInput<'_>,
    period_duration_ticks: Option<u64>,
) -> Option<u64> {
    let pto = multi.presentation_time_offset;
    if let Some(pd) = period_duration_ticks {
        return Some(pd.saturating_add(pto));
    }
    let dynamic = input.dynamic.as_ref()?;
    let horizon = dynamic.horizon_utc_us?;
    let rel = horizon
        .saturating_sub(dynamic.availability_start_us)
        .saturating_sub(input.period_start_us);
    Some(us_to_ticks_ceil(rel, multi.timescale).saturating_add(pto))
}

impl SegmentPlan {
    /// Build the plan.
    ///
    /// # Errors
    /// [`AddressError`]: the Representation is to be dropped (or, for
    /// [`AddressError::IndexRequired`], planned again once the `sidx` is
    /// fetched).
    pub fn build(input: &PlanInput<'_>) -> Result<Self, AddressError> {
        let avail = input.dynamic.as_ref().map(|d| Avail {
            ast_us: d.availability_start_us,
            ato_us: d.availability_time_offset_us,
            tsb_us: d.time_shift_buffer_depth_us,
        });
        let mut plan = match input.info {
            SegmentInfo::Template(t) => Self::from_template(input, t)?,
            SegmentInfo::List(l) => Self::from_list(input, l)?,
            SegmentInfo::Base(b) => Self::from_base(input, b)?,
        };
        plan.avail = avail;
        plan.period_start_us = input.period_start_us;
        Ok(plan)
    }

    fn period_duration_us(input: &PlanInput<'_>) -> Option<i128> {
        input
            .period_end_us
            .map(|e| e.saturating_sub(input.period_start_us).max(0))
    }

    fn from_template(input: &PlanInput<'_>, t: &TemplateInfo) -> Result<Self, AddressError> {
        let media = t
            .media
            .as_deref()
            .ok_or(AddressError::MissingMediaTemplate)?;
        let used = template::scan_media(media).map_err(|f| fault(media, f))?;
        reject_sequences(used)?;
        let mut init = t.multi.base.initialization.as_ref().map(url_ref);
        if let Some(tpl) = &t.initialization {
            let used = template::scan_initialization(tpl).map_err(|f| fault(tpl, f))?;
            reject_sequences(used)?;
            init = Some((
                template::substitute(tpl, input.representation_id, None, None, input.bandwidth),
                None,
            ));
        }
        let mut index = t.multi.base.representation_index.as_ref().map(url_ref);
        if let Some(tpl) = &t.index {
            let used = template::scan(tpl).map_err(|f| fault(tpl, f))?;
            reject_sequences(used)?;
            if !used.number && !used.time {
                index = Some((
                    template::substitute(tpl, input.representation_id, None, None, input.bandwidth),
                    None,
                ));
            }
        }
        let m = &t.multi;
        let ts = m.timescale;
        let pd_us = Self::period_duration_us(input);
        let pd_ticks = pd_us.map(|d| us_to_ticks_ceil(d, ts));
        let (kind, len_cap) = if let Some(tl) = &m.timeline {
            let end = period_end_media_ticks(m, input, pd_ticks);
            let idx = TimelineIndex::build(tl, m.start_number, end)?;
            let cap = m.end_number.map(|en| idx.count_up_to_number(en));
            (Kind::Timeline(idx), cap)
        } else if let Some(d) = m.duration {
            let mut count = pd_us.map(|dur| {
                let num = dur.saturating_mul(i128::from(ts));
                let den = i128::from(d).saturating_mul(MICROS);
                let c = num.div_euclid(den) + i128::from(num.rem_euclid(den) != 0);
                u64::try_from(c).unwrap_or(u64::MAX)
            });
            if let Some(en) = m.end_number {
                let cap = if en < m.start_number {
                    0
                } else {
                    en - m.start_number + 1
                };
                count = Some(count.map_or(cap, |c| c.min(cap)));
            }
            let last_duration = match (count, pd_ticks) {
                (Some(c), Some(pd)) if c > 0 => {
                    let full = (c - 1).saturating_mul(d);
                    pd.saturating_sub(full).clamp(1, d)
                }
                _ => d,
            };
            (
                Kind::Duration {
                    d,
                    start_number: m.start_number,
                    count,
                    last_duration,
                },
                None,
            )
        } else {
            let seg = Fixed {
                number: m.start_number,
                time: 0,
                start_ticks: 0,
                duration: pd_ticks.unwrap_or(0),
                reference: String::new(),
                range: None,
            };
            (Kind::Fixed(vec![seg]), None)
        };
        Ok(Self {
            kind,
            timescale: ts,
            pto: m.presentation_time_offset,
            period_start_us: 0,
            avail: None,
            media: Some(MediaTemplate {
                template: media.to_owned(),
                representation_id: input.representation_id.to_owned(),
                bandwidth: input.bandwidth,
            }),
            init,
            index,
            len_cap,
        })
    }

    fn from_list(input: &PlanInput<'_>, l: &ListInfo) -> Result<Self, AddressError> {
        let m = &l.multi;
        let ts = m.timescale;
        let pd_ticks = Self::period_duration_us(input).map(|d| us_to_ticks_ceil(d, ts));
        let timeline = match &m.timeline {
            Some(tl) => {
                let end = period_end_media_ticks(m, input, pd_ticks);
                Some(TimelineIndex::build(tl, m.start_number, end)?)
            }
            None => None,
        };
        let segs = list::expand(l, timeline.as_ref(), pd_ticks)?;
        let pto = i128::from(m.presentation_time_offset);
        let fixed = segs
            .into_iter()
            .map(|s| Fixed {
                number: s.number,
                time: s.time,
                start_ticks: if timeline.is_some() {
                    i128::from(s.time) - pto
                } else {
                    i128::from(s.time)
                },
                duration: s.duration,
                reference: s.reference,
                range: s.range,
            })
            .collect();
        Ok(Self {
            kind: Kind::Fixed(fixed),
            timescale: ts,
            pto: m.presentation_time_offset,
            period_start_us: 0,
            avail: None,
            media: None,
            init: m.base.initialization.as_ref().map(url_ref),
            index: m.base.representation_index.as_ref().map(url_ref),
            len_cap: None,
        })
    }

    fn from_base(input: &PlanInput<'_>, b: &BaseInfo) -> Result<Self, AddressError> {
        let init = b.initialization.as_ref().map(url_ref);
        let index = b
            .representation_index
            .as_ref()
            .map(url_ref)
            .or_else(|| b.index_range.map(|r| (String::new(), Some(r))));
        let pto = b.presentation_time_offset.unwrap_or(0);
        let base_ts = b.timescale.unwrap_or(1);
        if let Some(r) = &b.index_range {
            let (sidx, anchor) = input.index.ok_or(AddressError::IndexRequired)?;
            let _ = base_index::anchor_after(r)?;
            let ts = u64::from(sidx.timescale).max(1);
            let segs: Vec<IndexedSegment> = base_index::segments(sidx, anchor)?;
            let fixed = segs
                .into_iter()
                .enumerate()
                .map(|(i, s)| Fixed {
                    number: i as u64 + 1,
                    time: s.time,
                    start_ticks: i128::from(s.time) - i128::from(pto),
                    duration: s.duration,
                    reference: String::new(),
                    range: Some(s.range),
                })
                .collect();
            return Ok(Self {
                kind: Kind::Fixed(fixed),
                timescale: ts,
                pto,
                period_start_us: 0,
                avail: None,
                media: None,
                init,
                index,
                len_cap: None,
            });
        }
        let pd_ticks = Self::period_duration_us(input).map(|d| us_to_ticks_ceil(d, base_ts));
        let seg = Fixed {
            number: 1,
            time: 0,
            start_ticks: 0,
            duration: pd_ticks.unwrap_or(0),
            reference: String::new(),
            range: None,
        };
        Ok(Self {
            kind: Kind::Fixed(vec![seg]),
            timescale: base_ts,
            pto,
            period_start_us: 0,
            avail: None,
            media: None,
            init,
            index,
            len_cap: None,
        })
    }

    /// `@timescale` in effect.
    #[must_use]
    pub fn timescale(&self) -> u64 {
        self.timescale
    }

    /// `@presentationTimeOffset` in ticks.
    #[must_use]
    pub fn presentation_time_offset(&self) -> u64 {
        self.pto
    }

    /// PeriodStart in microseconds.
    #[must_use]
    pub fn period_start_us(&self) -> i128 {
        self.period_start_us
    }

    /// Whether availability data is attached (dynamic MPD).
    #[must_use]
    pub fn is_dynamic(&self) -> bool {
        self.avail.is_some()
    }

    /// Number of segments; `None` when the plan is open-ended (a duration
    /// template of a Period with no known end).
    #[must_use]
    pub fn len(&self) -> Option<u64> {
        let raw = match &self.kind {
            Kind::Duration { count, .. } => *count,
            Kind::Timeline(t) => Some(t.len()),
            Kind::Fixed(v) => Some(v.len() as u64),
        };
        match (raw, self.len_cap) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, None) => a,
            (None, b) => b,
        }
    }

    /// Whether the plan has no segment.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == Some(0)
    }

    fn reference(&self, number: u64, time: u64, fixed: Option<&str>) -> String {
        match (&self.media, fixed) {
            (Some(m), _) => template::substitute(
                &m.template,
                &m.representation_id,
                Some(number),
                Some(time),
                m.bandwidth,
            ),
            (None, Some(r)) => r.to_owned(),
            (None, None) => String::new(),
        }
    }

    /// The segment at zero-based `index`.
    #[must_use]
    pub fn get(&self, index: u64) -> Option<SegmentRef> {
        if self.len().is_some_and(|n| index >= n) {
            return None;
        }
        let (number, time, start_ticks, duration, reference, range) = match &self.kind {
            Kind::Duration {
                d,
                start_number,
                count,
                last_duration,
            } => {
                let start = index.checked_mul(*d)?;
                let dur = if count.is_some_and(|c| index + 1 == c) {
                    *last_duration
                } else {
                    *d
                };
                let number = start_number.saturating_add(index);
                (
                    number,
                    start,
                    i128::from(start),
                    dur,
                    self.reference(number, start, None),
                    None,
                )
            }
            Kind::Timeline(t) => {
                let s = t.segment(index)?;
                (
                    s.number,
                    s.time,
                    i128::from(s.time) - i128::from(self.pto),
                    s.duration,
                    self.reference(s.number, s.time, None),
                    None,
                )
            }
            Kind::Fixed(v) => {
                let f = v.get(usize::try_from(index).ok()?)?;
                (
                    f.number,
                    f.time,
                    f.start_ticks,
                    f.duration,
                    self.reference(f.number, f.time, Some(&f.reference)),
                    f.range,
                )
            }
        };
        let start_us = ticks_to_us(start_ticks, self.timescale);
        let duration_us = ticks_to_us(i128::from(duration), self.timescale);
        let (available_from_us, available_until_us) = match &self.avail {
            Some(a) => (
                Some(segment_available_from_ticks(
                    a.ast_us,
                    self.period_start_us,
                    start_ticks,
                    i128::from(duration),
                    self.timescale,
                    a.ato_us,
                )),
                segment_available_until_ticks(
                    a.ast_us,
                    self.period_start_us,
                    start_ticks,
                    i128::from(duration),
                    self.timescale,
                    a.tsb_us,
                ),
            ),
            None => (None, None),
        };
        Some(SegmentRef {
            index,
            number,
            time_ticks: time,
            duration_ticks: duration,
            timescale: self.timescale,
            start_us,
            duration_us,
            reference,
            byte_range: range,
            available_from_us,
            available_until_us,
        })
    }

    /// The segments in order. For an open-ended plan this never ends; bound it
    /// with `take`.
    pub fn iter(&self) -> impl Iterator<Item = SegmentRef> + '_ {
        let mut i = 0u64;
        std::iter::from_fn(move || {
            let r = self.get(i)?;
            i += 1;
            Some(r)
        })
    }

    /// The Initialization Segment address, with its availability window.
    #[must_use]
    pub fn init(&self) -> Option<ResourceRef> {
        self.init
            .as_ref()
            .map(|(reference, range)| self.resource(reference, *range))
    }

    /// The Index Segment address (the `sidx` of a SegmentBase Representation,
    /// or a `RepresentationIndex`).
    #[must_use]
    pub fn index(&self) -> Option<ResourceRef> {
        self.index
            .as_ref()
            .map(|(reference, range)| self.resource(reference, *range))
    }

    fn resource(&self, reference: &str, range: Option<ByteRange>) -> ResourceRef {
        let from = self
            .avail
            .as_ref()
            .map(|a| init_available_from(a.ast_us, self.period_start_us));
        let until = self
            .len()
            .and_then(|n| n.checked_sub(1))
            .and_then(|last| self.get(last))
            .and_then(|s| s.available_until_us);
        ResourceRef {
            reference: reference.to_owned(),
            byte_range: range,
            available_from_us: from,
            available_until_us: until,
        }
    }

    /// Index of the last segment whose Period-relative start is at or before
    /// `period_time_us`; `None` before the first segment.
    #[must_use]
    pub fn index_at_or_before(&self, period_time_us: i128) -> Option<u64> {
        let n = self.len();
        let (mut lo, mut hi) = (0u64, n.unwrap_or(u64::MAX / 2));
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.get(mid) {
                Some(s) if s.start_us <= period_time_us => lo = mid + 1,
                Some(_) => hi = mid,
                None => hi = mid,
            }
        }
        lo.checked_sub(1)
    }

    /// Availability start of the segment at `index`.
    #[must_use]
    pub fn available_from_us(&self, index: u64) -> Option<i128> {
        self.get(index)?.available_from_us
    }

    /// Availability end of the segment at `index`.
    #[must_use]
    pub fn available_until_us(&self, index: u64) -> Option<i128> {
        self.get(index)?.available_until_us
    }

    /// The first index, and the number of segments, for which `pred` holds on
    /// a monotone-in-index predicate over `[0, bound)`: the count of leading
    /// indices where `pred` is true.
    pub(crate) fn leading_true(&self, bound: u64, pred: impl Fn(&SegmentRef) -> bool) -> u64 {
        let (mut lo, mut hi) = (0u64, bound);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.get(mid) {
                Some(s) if pred(&s) => lo = mid + 1,
                _ => hi = mid,
            }
        }
        lo
    }

    pub(crate) fn constant_duration(&self) -> Option<u64> {
        match &self.kind {
            Kind::Duration { d, .. } => Some(*d),
            _ => None,
        }
    }

    pub(crate) fn avail_parts(&self) -> Option<(i128, i128)> {
        self.avail.as_ref().map(|a| (a.ast_us, a.ato_us))
    }
}
