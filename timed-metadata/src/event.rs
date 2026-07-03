//! Canonical timed-metadata event (the hub of the hub-and-spoke model).
use crate::error::Result;
use alloc::{string::String, vec::Vec};
use scte35_splice::{SpliceInfoSection, commands::AnyCommand};

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
    pub fn from_scte35(section: &SpliceInfoSection, raw: &[u8]) -> Result<Self> {
        let mut id = None;
        let mut kind = EventKind::Unspecified;
        let mut at = None;
        let mut duration = None;

        if let Some(clear) = &section.clear {
            if let AnyCommand::SpliceInsert(si) = &clear.command {
                id = Some(si.splice_event_id);
                kind = if si.out_of_network_indicator {
                    EventKind::BreakStart
                } else {
                    EventKind::BreakEnd
                };
                if let Some(st) = &si.splice_time {
                    at = st.pts_time.map(MediaTime);
                }
                if let Some(bd) = &si.break_duration {
                    duration = Some(MediaDuration(bd.duration));
                }
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
}
