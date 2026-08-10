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
pub const PTS_WRAP: u64 = broadcast_common::clock33::WRAP_33BIT;

/// A stateful conversion session.
#[derive(Debug, Default)]
pub struct Timeline {
    anchor: Option<TimeAnchor>,
    unroller: PtsUnroller,
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
            unroller: PtsUnroller::default(),
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
            let abs = self.unroller.unroll(pts33);
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

/// Per-signal 33-bit PTS unroller: turns a repeating 90 kHz wire counter into
/// an absolute, ever-growing tick value.
///
/// Thin stateful wrapper around
/// [`broadcast_common::clock33::unwrap_delta`] — the actual wrap-correction
/// math lives there (shared with `transmux`'s demux-edge unroller and
/// `media-doctor`/`compliance-probe`'s wrap-aware comparisons), so a fix
/// there reaches every 33-bit clock consumer in the workspace instead of
/// just this one. Used both by [`Timeline`] and by the caption diff-based
/// boundary tracker (`crate::webvtt::cue::DiffState`) — a second reason not
/// to hand-roll the (raw, epoch) bookkeeping a third time.
///
/// SCTE-35 cues and caption commit events never legitimately reorder across
/// the 33-bit origin before any sample has been observed (there is no prior
/// sample to wrap from), so `unroll` never sees a negative accumulator in
/// practice; the `max(0)` below is a defensive clamp against malformed input
/// rather than a real code path.
#[derive(Debug, Default)]
pub(crate) struct PtsUnroller {
    /// `(previous raw 33-bit value, previous unwrapped accumulator)`.
    state: Option<(u64, i128)>,
}

impl PtsUnroller {
    pub(crate) fn unroll(&mut self, raw33: u64) -> u64 {
        let unwrapped = match self.state {
            Some((prev_raw, prev_unwrapped)) => {
                broadcast_common::clock33::unwrap_delta(prev_unwrapped, prev_raw, raw33)
            }
            None => raw33 as i128,
        };
        self.state = Some((raw33, unwrapped));
        unwrapped.max(0) as u64
    }
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
        // A near-max previous value then a small next value crosses one wrap.
        let mut u = PtsUnroller {
            state: Some(((1u64 << 33) - 10, ((1u64 << 33) - 10) as i128)),
        };
        assert_eq!(u.unroll(5), 5 + (1u64 << 33));
    }

    #[test]
    fn wrap_unroll_forward_delta_keeps_epoch() {
        // A normal forward delta within range must NOT bump the epoch.
        let mut u = PtsUnroller {
            state: Some((1_000, 1_000)),
        };
        assert_eq!(u.unroll(2_000), 2_000);
        // First call (no prior pts) returns the raw value.
        let mut u2 = PtsUnroller::default();
        assert_eq!(u2.unroll(42), 42);
    }

    /// MUTATION VERIFIED: the previous `unroll_pts` (a forward-only epoch
    /// counter) could not distinguish a small backward reorder that
    /// straddles the wrap origin from a huge forward jump — see
    /// `broadcast_common::clock33::unwrap_delta`'s doc comment for the exact
    /// case. Reproduced here at the `PtsUnroller` level: a splice/caption
    /// event at raw tick `2`, then one at raw tick `2^33 - 3` (a legitimate
    /// 5-tick backward step across the origin), must unroll to a small
    /// value, not to `~2^33`.
    #[test]
    fn wrap_unroll_backward_reorder_across_origin_does_not_leap_forward() {
        let mut u = PtsUnroller::default();
        assert_eq!(u.unroll(2), 2);
        let second = u.unroll((1u64 << 33) - 3);
        assert!(
            second < 1000,
            "expected a small value from a 5-tick backward reorder, got {second}"
        );
    }
}
