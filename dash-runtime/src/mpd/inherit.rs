//! Attribute inheritance down Period > AdaptationSet > Representation.
//!
//! ISO/IEC 23009-1 5.3.9.1: Segment information may be declared at any of the
//! three levels; an element inherits the attributes and children of the same
//! element above it, and the lower level wins attribute by attribute.
//! `@duration` and `SegmentTimeline` are exclusive (5.3.9.2.1), so a level
//! that introduces one does not inherit the other. A SegmentTemplate and a
//! SegmentList may not both appear along one chain.
//! The `RepresentationBase` attributes (5.3.7.2) of a Representation fall back
//! to its AdaptationSet's.

use thiserror::Error;

use super::model::{
    AdaptationSet, BaseInfo, CommonAttributes, MultiInfo, Period, Representation, SegmentTimeline,
    SegmentUrl,
};

/// `@timescale` default (5.3.9.2.2).
pub const DEFAULT_TIMESCALE: u64 = 1;
/// `@startNumber` default (5.3.9.5.3).
pub const DEFAULT_START_NUMBER: u64 = 1;
/// `@presentationTimeOffset` default (5.3.9.2.2).
pub const DEFAULT_PRESENTATION_TIME_OFFSET: u64 = 0;

/// Why a Representation's Segment information could not be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum InheritError {
    /// A SegmentTemplate and a SegmentList along one chain (5.3.9.1).
    #[error("SegmentTemplate and SegmentList both appear along one Period/AdaptationSet/Representation chain")]
    TemplateAndList,
    /// `@duration` and `SegmentTimeline` on one element (5.3.9.2.1).
    #[error("both @duration and SegmentTimeline are present on one level")]
    DurationAndTimeline,
}

/// The inherited multiple-Segment fields with defaults applied.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ResolvedMulti {
    /// Merged Table 16 fields (raw, defaults not applied).
    pub base: BaseInfo,
    /// `@timescale`, default 1.
    pub timescale: u64,
    /// `@presentationTimeOffset`, default 0.
    pub presentation_time_offset: u64,
    /// `@duration`.
    pub duration: Option<u64>,
    /// `@startNumber`, default 1.
    pub start_number: u64,
    /// `@endNumber`.
    pub end_number: Option<u64>,
    /// The effective `SegmentTimeline`.
    pub timeline: Option<SegmentTimeline>,
}

/// A resolved `SegmentTemplate`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct TemplateInfo {
    /// Merged numeric fields.
    pub multi: ResolvedMulti,
    /// `@media`.
    pub media: Option<String>,
    /// `@index`.
    pub index: Option<String>,
    /// `@initialization`.
    pub initialization: Option<String>,
    /// `@bitstreamSwitching`.
    pub bitstream_switching: Option<String>,
}

/// A resolved `SegmentList`.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ListInfo {
    /// Merged numeric fields.
    pub multi: ResolvedMulti,
    /// The `SegmentURL` entries (from the lowest level that lists any).
    pub urls: Vec<SegmentUrl>,
}

/// A Representation's effective Segment information (5.3.9.1): exactly one of
/// the three kinds.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SegmentInfo {
    /// `SegmentTemplate`.
    Template(TemplateInfo),
    /// `SegmentList`.
    List(ListInfo),
    /// `SegmentBase` or none: one Media Segment at the `BaseURL`, optionally
    /// indexed by a `sidx` at `@indexRange`.
    Base(BaseInfo),
}

fn merge_base(parent: &BaseInfo, child: &BaseInfo) -> BaseInfo {
    BaseInfo {
        timescale: child.timescale.or(parent.timescale),
        presentation_time_offset: child
            .presentation_time_offset
            .or(parent.presentation_time_offset),
        ept_delta: child.ept_delta.or(parent.ept_delta),
        pd_delta: child.pd_delta.or(parent.pd_delta),
        presentation_duration: child.presentation_duration.or(parent.presentation_duration),
        time_shift_buffer_depth: child
            .time_shift_buffer_depth
            .or(parent.time_shift_buffer_depth),
        index_range: child.index_range.or(parent.index_range),
        index_range_exact: child.index_range_exact.or(parent.index_range_exact),
        availability_time_offset: child
            .availability_time_offset
            .or(parent.availability_time_offset),
        availability_time_complete: child
            .availability_time_complete
            .or(parent.availability_time_complete),
        initialization: child
            .initialization
            .clone()
            .or_else(|| parent.initialization.clone()),
        representation_index: child
            .representation_index
            .clone()
            .or_else(|| parent.representation_index.clone()),
    }
}

fn merge_multi(parent: &MultiInfo, child: &MultiInfo) -> Result<MultiInfo, InheritError> {
    if child.duration.is_some() && child.timeline.is_some() {
        return Err(InheritError::DurationAndTimeline);
    }
    let (duration, timeline) = if child.timeline.is_some() {
        (None, child.timeline.clone())
    } else if child.duration.is_some() {
        (child.duration, None)
    } else {
        (parent.duration, parent.timeline.clone())
    };
    Ok(MultiInfo {
        base: merge_base(&parent.base, &child.base),
        duration,
        start_number: child.start_number.or(parent.start_number),
        end_number: child.end_number.or(parent.end_number),
        timeline,
        bitstream_switching: child
            .bitstream_switching
            .clone()
            .or_else(|| parent.bitstream_switching.clone()),
    })
}

fn finish_multi(m: MultiInfo) -> Result<ResolvedMulti, InheritError> {
    if m.duration.is_some() && m.timeline.is_some() {
        return Err(InheritError::DurationAndTimeline);
    }
    Ok(ResolvedMulti {
        timescale: m.base.timescale.unwrap_or(DEFAULT_TIMESCALE),
        presentation_time_offset: m
            .base
            .presentation_time_offset
            .unwrap_or(DEFAULT_PRESENTATION_TIME_OFFSET),
        base: m.base,
        duration: m.duration,
        start_number: m.start_number.unwrap_or(DEFAULT_START_NUMBER),
        end_number: m.end_number,
        timeline: m.timeline,
    })
}

/// Resolve the effective Segment information of `rep`.
///
/// # Errors
/// [`InheritError`] when the chain violates 5.3.9.1 / 5.3.9.2.1; the caller
/// drops the Representation.
pub fn resolve_segment_info(
    period: &Period,
    set: &AdaptationSet,
    rep: &Representation,
) -> Result<SegmentInfo, InheritError> {
    let layers = [&period.segment, &set.segment, &rep.segment];
    let has_template = layers.iter().any(|l| l.template.is_some());
    let has_list = layers.iter().any(|l| l.list.is_some());
    if has_template && has_list {
        return Err(InheritError::TemplateAndList);
    }
    if has_template {
        let mut merged = crate::mpd::model::SegmentTemplate::default();
        for t in layers.iter().filter_map(|l| l.template.as_ref()) {
            merged = crate::mpd::model::SegmentTemplate {
                multi: merge_multi(&merged.multi, &t.multi)?,
                media: t.media.clone().or(merged.media),
                index: t.index.clone().or(merged.index),
                initialization: t.initialization.clone().or(merged.initialization),
                bitstream_switching: t
                    .bitstream_switching
                    .clone()
                    .or(merged.bitstream_switching),
            };
        }
        return Ok(SegmentInfo::Template(TemplateInfo {
            multi: finish_multi(merged.multi)?,
            media: merged.media,
            index: merged.index,
            initialization: merged.initialization,
            bitstream_switching: merged.bitstream_switching,
        }));
    }
    if has_list {
        let mut multi = MultiInfo::default();
        let mut urls: Vec<SegmentUrl> = Vec::new();
        for l in layers.iter().filter_map(|l| l.list.as_ref()) {
            multi = merge_multi(&multi, &l.multi)?;
            if !l.urls.is_empty() {
                urls.clone_from(&l.urls);
            }
        }
        return Ok(SegmentInfo::List(ListInfo {
            multi: finish_multi(multi)?,
            urls,
        }));
    }
    let mut base = BaseInfo::default();
    for b in layers.iter().filter_map(|l| l.base.as_ref()) {
        base = merge_base(&base, &b.info);
    }
    Ok(SegmentInfo::Base(base))
}

/// The `RepresentationBase` attributes of `rep`, each falling back to `set`.
#[must_use]
pub fn effective_common(set: &AdaptationSet, rep: &Representation) -> CommonAttributes {
    let (s, r) = (&set.common, &rep.common);
    CommonAttributes {
        profiles: r.profiles.clone().or_else(|| s.profiles.clone()),
        width: r.width.or(s.width),
        height: r.height.or(s.height),
        sar: r.sar.clone().or_else(|| s.sar.clone()),
        frame_rate: r.frame_rate.clone().or_else(|| s.frame_rate.clone()),
        audio_sampling_rate: r
            .audio_sampling_rate
            .clone()
            .or_else(|| s.audio_sampling_rate.clone()),
        mime_type: r.mime_type.clone().or_else(|| s.mime_type.clone()),
        codecs: r.codecs.clone().or_else(|| s.codecs.clone()),
        max_playout_rate: r.max_playout_rate.or(s.max_playout_rate),
    }
}
