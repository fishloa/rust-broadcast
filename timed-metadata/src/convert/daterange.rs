//! SCTE-35 → HLS `EXT-X-DATERANGE` (RFC 8216 / hls-bis §4.4.5.1).
use crate::anchor::TimeAnchor;
use crate::daterange::{DateRange, Scte35Attr, Scte35Cue};
use crate::error::{Error, Result};
use crate::event::{EventKind, SourcePayload, TimedEvent};
use alloc::string::ToString;

/// Convert a SCTE-35-sourced [`TimedEvent`] to a [`DateRange`].
///
/// `START-DATE` comes from `ev.at` via `anchor` when present; otherwise from the
/// anchor's own UTC (the insertion-point time the caller supplied). The original
/// splice bytes are carried verbatim into the `SCTE35-OUT`/`IN` attribute.
pub fn scte35_to_daterange(ev: &TimedEvent, anchor: &TimeAnchor) -> Result<DateRange> {
    let raw = match &ev.source {
        SourcePayload::Scte35 { raw } => raw.clone(),
        SourcePayload::Emsg { .. } => {
            return Err(Error::AttrParse("event is not SCTE-35-sourced".to_string()))
        }
    };

    let cue = match ev.kind {
        EventKind::BreakStart => Scte35Cue::Out,
        EventKind::BreakEnd => Scte35Cue::In,
        _ => Scte35Cue::Cmd,
    };

    let start_date = match ev.at {
        Some(t) => anchor.rfc3339(t),
        None => crate::anchor::format_rfc3339_ms(anchor.utc_epoch_ms),
    };

    let planned_duration = ev.duration.map(|d| d.as_seconds_f64());

    Ok(DateRange {
        id: ev.id.map(|i| i.to_string()).unwrap_or_default(),
        start_date,
        class: None,
        duration: None,
        planned_duration,
        scte35: Some(Scte35Attr { cue, raw }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anchor::TimeAnchor;
    use crate::daterange::Scte35Cue;
    use crate::event::{EventKind, MediaDuration, SourcePayload, TimedEvent};
    use alloc::{string::ToString, vec};

    #[test]
    fn break_start_maps_to_scte35_out_with_duration() {
        let ev = TimedEvent {
            id: Some(2002),
            kind: EventKind::BreakStart,
            at: None,
            duration: Some(MediaDuration(2_160_000)), // 24s
            source: SourcePayload::Scte35 {
                raw: vec![0xFC, 0x30, 0x21],
            },
        };
        // anchor: epoch 0 == pts 0; with at=None, START-DATE uses anchor.utc_epoch_ms.
        let anchor = TimeAnchor {
            pts_90k: 0,
            utc_epoch_ms: 0,
        };
        let dr = scte35_to_daterange(&ev, &anchor).unwrap();
        assert_eq!(dr.id, "2002");
        assert_eq!(dr.planned_duration, Some(24.0));
        let s = dr.scte35.unwrap();
        assert_eq!(s.cue, Scte35Cue::Out);
        assert_eq!(s.raw, vec![0xFC, 0x30, 0x21]); // verbatim
        assert_eq!(dr.start_date, "1970-01-01T00:00:00.000Z");
    }
}
