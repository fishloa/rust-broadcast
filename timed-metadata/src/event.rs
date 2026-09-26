//! Canonical timed-metadata event (the hub of the hub-and-spoke model).
use crate::error::Result;
use alloc::{string::String, vec::Vec};
use scte35_splice::{
    SpliceInfoSection,
    commands::AnyCommand,
    descriptors::{AnySpliceDescriptor, SegmentationTypeId},
};

/// A media-timeline instant in 90 kHz ticks, wrap-unrolled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MediaTime(pub u64);

/// A duration in 90 kHz ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MediaDuration(pub u64);

impl MediaDuration {
    /// The duration in seconds.
    pub fn as_seconds_f64(self) -> f64 {
        self.0 as f64 / crate::PTS_HZ as f64
    }
}

/// The abstracted meaning of an event, independent of carriage format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum EventKind {
    /// Start of an ad/break opportunity (SCTE-35 out-of-network).
    BreakStart,
    /// Return to network (SCTE-35 in-to-network).
    BreakEnd,
    /// Chapter / program boundary.
    Chapter,
    /// Meaning not determined from the source.
    Unspecified,
}

impl EventKind {
    /// Stable label for this variant.
    pub fn name(&self) -> &'static str {
        match self {
            EventKind::BreakStart => "break_start",
            EventKind::BreakEnd => "break_end",
            EventKind::Chapter => "chapter",
            EventKind::Unspecified => "unspecified",
        }
    }
}
broadcast_common::impl_spec_display!(EventKind);

/// The lossless original payload, carried verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum SourcePayload {
    /// A SCTE-35 `splice_info_section`, verbatim.
    Scte35 { raw: Vec<u8> },
    /// A DASH `emsg`: its scheme/value plus the verbatim `message_data`.
    Emsg {
        scheme_id_uri: String,
        value: String,
        raw: Vec<u8>,
    },
}

/// The canonical event passed between format adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TimedEvent {
    /// Event id (`splice_event_id` / emsg `id`).
    pub id: Option<u32>,
    /// Abstract meaning.
    pub kind: EventKind,
    /// Media-timeline instant; `None` = immediate / determined by insertion point.
    pub at: Option<MediaTime>,
    /// Event duration, if known.
    pub duration: Option<MediaDuration>,
    /// Lossless original.
    pub source: SourcePayload,
}

impl TimedEvent {
    /// Build from a parsed SCTE-35 section, retaining `raw` verbatim.
    ///
    /// Every `pts_time` is shifted by the section's `pts_adjustment` (SCTE 35
    /// §9.6.1: "the value of `pts_adjustment` shall be added to the
    /// `pts_time`... modulo 2^33") via [`broadcast_common::clock33::add`]
    /// before it becomes [`TimedEvent::at`] — the section itself is not
    /// mutated, only the value this crate derives from it.
    pub fn from_scte35(section: &SpliceInfoSection, raw: &[u8]) -> Result<Self> {
        let mut id = None;
        let mut kind = EventKind::Unspecified;
        let mut at = None;
        let mut duration = None;

        let adjust =
            |pts: u64| MediaTime(broadcast_common::clock33::add(pts, section.pts_adjustment));

        if let Some(clear) = &section.clear {
            match &clear.command {
                AnyCommand::SpliceInsert(si) => {
                    id = Some(si.splice_event_id);
                    kind = if si.splice_event_cancel_indicator {
                        // A cancel carries no out_of_network_indicator (§9.7.3);
                        // it is neither a break start nor a break end.
                        EventKind::Unspecified
                    } else if si.out_of_network_indicator {
                        EventKind::BreakStart
                    } else {
                        EventKind::BreakEnd
                    };
                    if let Some(st) = &si.splice_time {
                        at = st.pts_time.map(adjust);
                    }
                    if let Some(bd) = &si.break_duration {
                        duration = Some(MediaDuration(bd.duration));
                    }
                }
                AnyCommand::TimeSignal(ts) => {
                    at = ts.splice_time.pts_time.map(adjust);
                    // time_signal() carries no id/kind/duration of its own
                    // (§9.7.4): those ride in the descriptor loop, almost
                    // always a segmentation_descriptor (§10.3.3). Take the
                    // first one, per this crate's documented lossy-collapse
                    // policy for multi-descriptor loops.
                    for d in section.descriptors() {
                        if let Ok(AnySpliceDescriptor::Segmentation(seg)) = d {
                            if seg.segmentation_event_cancel_indicator {
                                continue;
                            }
                            id = Some(seg.segmentation_event_id);
                            kind = segmentation_kind(seg.segmentation_type_id);
                            duration = seg.segmentation_duration.map(MediaDuration);
                            break;
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(TimedEvent {
            id,
            kind,
            at,
            duration,
            source: SourcePayload::Scte35 { raw: raw.to_vec() },
        })
    }
}

/// Classify a `time_signal()` segmentation descriptor's `segmentation_type_id`
/// (SCTE 35 Table 23) into this crate's abstract [`EventKind`]. Program- and
/// chapter-boundary types map to [`EventKind::Chapter`]. Only the types that
/// actually leave (or return to) network programming for ad insertion map to
/// `BreakStart`/`BreakEnd`: `Break`, Provider/Distributor `Advertisement`,
/// Provider/Distributor `PlacementOpportunity` (**not** the `Overlay` variants
/// — an overlay is composited over the network feed, not a break away from
/// it), and Provider/Distributor `AdBlock`. Everything else — credits, promos,
/// unscheduled events, alternate content, and every `Overlay` placement
/// opportunity — is deliberately `Unspecified` rather than `BreakStart`/`End`:
/// a consumer that splices ads on `BreakStart` must not replace credits/promos
/// with an ad (`id`/`at`/`duration` are still populated; only `kind` is
/// unspecified for these). `NetworkStart`/`NetworkEnd` (Table 23 `0x50`/`0x51`)
/// have the *reversed* sense (a `NetworkStart` marks the return to network
/// programming, i.e. the end of a break) and are also left `Unspecified`
/// rather than guessed at; every other/reserved value is `Unspecified` too.
fn segmentation_kind(type_id: SegmentationTypeId) -> EventKind {
    use SegmentationTypeId as T;
    match type_id {
        T::ProgramStart | T::ProgramEnd | T::ChapterStart | T::ChapterEnd => EventKind::Chapter,
        T::BreakStart
        | T::ProviderAdvertisementStart
        | T::DistributorAdvertisementStart
        | T::ProviderPlacementOpportunityStart
        | T::DistributorPlacementOpportunityStart
        | T::ProviderAdBlockStart
        | T::DistributorAdBlockStart => EventKind::BreakStart,
        T::BreakEnd
        | T::ProviderAdvertisementEnd
        | T::DistributorAdvertisementEnd
        | T::ProviderPlacementOpportunityEnd
        | T::DistributorPlacementOpportunityEnd
        | T::ProviderAdBlockEnd
        | T::DistributorAdBlockEnd => EventKind::BreakEnd,
        _ => EventKind::Unspecified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;
    use broadcast_common::traits::Parse;
    use scte35_splice::SpliceInfoSection;

    // Real Unified Streaming splice (ID 2002): out-of-network, break_duration 2160000 (24s).
    fn splice_2002() -> Vec<u8> {
        let hex = "FC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D";
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn from_scte35_extracts_break_start_and_duration() {
        let raw = splice_2002();
        let section = SpliceInfoSection::parse(&raw).unwrap();
        let ev = TimedEvent::from_scte35(&section, &raw).unwrap();
        assert_eq!(ev.id, Some(2002));
        assert_eq!(ev.kind, EventKind::BreakStart); // out_of_network = true
        assert_eq!(ev.at, None); // pts_time None (program splice)
        assert_eq!(ev.duration, Some(MediaDuration(2_160_000)));
        assert!((ev.duration.unwrap().as_seconds_f64() - 24.0).abs() < 1e-9);
        match &ev.source {
            SourcePayload::Scte35 { raw: r } => assert_eq!(r, &raw), // verbatim, lossless
            _ => panic!("expected Scte35 payload"),
        }
    }

    #[test]
    fn event_kind_labels() {
        assert_eq!(EventKind::BreakStart.name(), "break_start");
        assert_eq!(alloc::format!("{}", EventKind::BreakEnd), "break_end");
    }

    /// Issue #1040: only the segmentation types that actually leave/return to
    /// network programming for ad insertion (Table 23's `Break`,
    /// `*Advertisement`, `*PlacementOpportunity` (non-overlay), `*AdBlock`)
    /// map to `BreakStart`/`BreakEnd`. Program/Chapter map to `Chapter`.
    /// Everything else -- including credits, promos, unscheduled/alternate
    /// content, and the `Overlay` placement-opportunity variants (an overlay
    /// is composited over the network feed, not a break away from it) -- is
    /// `Unspecified`, so a consumer that splices ads only on `BreakStart`
    /// cannot mistake a credit/promo boundary for an ad avail.
    #[test]
    fn segmentation_kind_classifies_by_group() {
        use SegmentationTypeId as T;
        let cases: &[(SegmentationTypeId, EventKind)] = &[
            // Program / Chapter -> Chapter.
            (T::ProgramStart, EventKind::Chapter),
            (T::ProgramEnd, EventKind::Chapter),
            (T::ChapterStart, EventKind::Chapter),
            (T::ChapterEnd, EventKind::Chapter),
            // The actual "leaves the network" break group -> BreakStart/End.
            (T::BreakStart, EventKind::BreakStart),
            (T::BreakEnd, EventKind::BreakEnd),
            (T::ProviderAdvertisementStart, EventKind::BreakStart),
            (T::DistributorAdvertisementEnd, EventKind::BreakEnd),
            (T::ProviderPlacementOpportunityStart, EventKind::BreakStart),
            (T::DistributorPlacementOpportunityEnd, EventKind::BreakEnd),
            (T::ProviderAdBlockStart, EventKind::BreakStart),
            (T::DistributorAdBlockEnd, EventKind::BreakEnd),
            // Everything else: Unspecified, NOT BreakStart/BreakEnd.
            (T::OpeningCreditStart, EventKind::Unspecified),
            (T::ClosingCreditEnd, EventKind::Unspecified),
            (T::ProviderPromoStart, EventKind::Unspecified),
            (T::DistributorPromoEnd, EventKind::Unspecified),
            (T::UnscheduledEventStart, EventKind::Unspecified),
            (T::AlternateContentOpportunityEnd, EventKind::Unspecified),
            // Overlay placement opportunities: composited OVER the network
            // feed, not a break away from it -- must not become BreakStart.
            (
                T::ProviderOverlayPlacementOpportunityStart,
                EventKind::Unspecified,
            ),
            (
                T::DistributorOverlayPlacementOpportunityEnd,
                EventKind::Unspecified,
            ),
            // NetworkStart/End have the reversed sense; also Unspecified.
            (T::NetworkStart, EventKind::Unspecified),
            (T::NetworkEnd, EventKind::Unspecified),
            (T::Reserved(0x60), EventKind::Unspecified),
        ];
        for (type_id, want) in cases {
            assert_eq!(
                segmentation_kind(*type_id),
                *want,
                "segmentation_type_id {type_id:?} ({}) must classify as {want:?}",
                type_id.name()
            );
        }
    }
}
