//! Stateful conversion session: holds the wall-clock anchor and unrolls 33-bit
//! PTS wrap across a stream of events.
use crate::anchor::TimeAnchor;
use crate::convert::{EmsgConfig, scte35_to_daterange, scte35_to_emsg};
use crate::daterange::DateRange;
use crate::error::{Error, Result};
use crate::event::{MediaTime, TimedEvent};
use alloc::vec::Vec;
use broadcast_common::traits::Parse;
use scte35_splice::SpliceInfoSection;

/// The 33-bit PTS modulus.
pub const PTS_WRAP: u64 = 1 << 33;

/// A stateful conversion session.
#[derive(Debug, Default)]
pub struct Timeline {
    anchor: Option<TimeAnchor>,
    last_pts: Option<u64>,
    epoch: u64,
}

impl Timeline {
    /// New session with no anchor.
    pub fn new() -> Self {
        Self::default()
    }
    /// New session with a wall-clock anchor.
    pub fn with_anchor(anchor: TimeAnchor) -> Self {
        Timeline {
            anchor: Some(anchor),
            last_pts: None,
            epoch: 0,
        }
    }
    /// Set / replace the anchor.
    pub fn set_anchor(&mut self, anchor: TimeAnchor) {
        self.anchor = Some(anchor);
    }

    /// Parse a SCTE-35 section; unroll its PTS into an absolute [`MediaTime`].
    pub fn push_scte35(&mut self, bytes: &[u8]) -> Result<TimedEvent> {
        let section = SpliceInfoSection::parse(bytes)?;
        let mut ev = TimedEvent::from_scte35(&section, bytes)?;
        if let Some(MediaTime(pts33)) = ev.at {
            let abs = unroll_pts(&mut self.last_pts, &mut self.epoch, pts33);
            ev.at = Some(MediaTime(abs));
        }
        Ok(ev)
    }

    /// Convert to a DATERANGE (requires an anchor).
    pub fn to_daterange(&self, ev: &TimedEvent) -> Result<DateRange> {
        let anchor = self.anchor.as_ref().ok_or(Error::MissingAnchor)?;
        scte35_to_daterange(ev, anchor)
    }

    /// Convert to a serialized SCTE-35 `emsg` box.
    pub fn to_emsg(&self, ev: &TimedEvent, cfg: &EmsgConfig) -> Result<Vec<u8>> {
        match &ev.source {
            crate::event::SourcePayload::Scte35 { raw } => scte35_to_emsg(raw, cfg),
            crate::event::SourcePayload::Emsg { .. } => Err(Error::AttrParse(
                alloc::string::String::from("event is not SCTE-35-sourced"),
            )),
        }
    }
}

/// Unroll a 33-bit PTS to an absolute monotonic value. On a backward jump of
/// more than half the range, advance one epoch.
pub(crate) fn unroll_pts(last_pts: &mut Option<u64>, epoch: &mut u64, pts33: u64) -> u64 {
    if let Some(prev) = *last_pts {
        if pts33 + (PTS_WRAP / 2) < prev {
            *epoch += 1;
        }
    }
    *last_pts = Some(pts33);
    *epoch * PTS_WRAP + pts33
}

#[cfg(test)]
mod tests {
    use super::*;

    fn splice_2002() -> alloc::vec::Vec<u8> {
        let hex = "FC302100000000000000FFF01005000007D27FEF7F7E0020F580C0000000000088B9661D";
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn push_scte35_returns_event() {
        let mut tl = Timeline::new();
        let ev = tl.push_scte35(&splice_2002()).unwrap();
        assert_eq!(ev.id, Some(2002));
    }

    #[test]
    fn to_daterange_without_anchor_errors() {
        let tl = Timeline::new();
        let ev = Timeline::new().push_scte35(&splice_2002()).unwrap();
        assert!(matches!(
            tl.to_daterange(&ev),
            Err(crate::Error::MissingAnchor)
        ));
    }

    #[test]
    fn wrap_unroll_adds_one_epoch() {
        // unroll(prev, cur) — a near-max prev then a small cur crosses one wrap.
        assert_eq!(
            unroll_pts(&mut Some((1u64 << 33) - 10), &mut 0u64, 5),
            5 + (1u64 << 33)
        );
    }

    #[test]
    fn wrap_unroll_forward_delta_keeps_epoch() {
        // A normal forward delta within range must NOT bump the epoch.
        let (mut last, mut epoch) = (Some(1_000u64), 0u64);
        assert_eq!(unroll_pts(&mut last, &mut epoch, 2_000), 2_000);
        assert_eq!(epoch, 0);
        // First call (no prior pts) returns the raw value, epoch unchanged.
        let (mut last2, mut epoch2) = (None, 0u64);
        assert_eq!(unroll_pts(&mut last2, &mut epoch2, 42), 42);
        assert_eq!(epoch2, 0);
    }
}
