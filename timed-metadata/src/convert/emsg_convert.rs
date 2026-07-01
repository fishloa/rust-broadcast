//! emsg version 0 ↔ version 1 conversion.
//!
//! Both versions encode the same Movie-timeline instant T (ISO/IEC 23009-1
//! §5.10.3.3):
//!
//! ```text
//! T = presentation_time (v1)   ==   EPT + presentation_time_delta (v0)
//! ```
//!
//! where EPT is the carrying segment's earliest presentation time.
//!
//! **Presentation-time offset** (`InbandEventStream@presentationTimeOffset`,
//! PTO) is carried in [`SegmentTiming`] for Movie↔Period alignment but does
//! *not* enter into the delta↔presentation_time relationship within a single
//! Representation.  Both `presentation_time` (v1) and `presentation_time_delta`
//! (v0) are relative to the same `timescale`, and the conversion is
//! PTO-independent — a round-trip v0↔v1↔v0 with any PTO reproduces the same
//! box bytes (the PTO is unused in the arithmetic, only documented for
//! higher-layer Period-level adjustment).
use crate::error::{Error, Result};
use mp4_emsg::{EmsgBox, PresentationTime};

/// Segment-level timing parameters needed for emsg version conversion.
///
/// All time fields share a single `timescale` (ticks/second), which MUST equal
/// the emsg's `timescale`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentTiming {
    /// The carrying segment's earliest presentation time on the Movie
    /// timeline, in `timescale` ticks.
    pub earliest_presentation_time: u64,
    /// `InbandEventStream@presentationTimeOffset`: the Movie→Period mapping
    /// adjustment, in `timescale` ticks.  Documented here for higher-layer
    /// use; not incorporated into the delta conversion (the arithmetic within
    /// one Representation is PTO-independent).
    pub presentation_time_offset: u64,
    /// Ticks per second; MUST equal the emsg's `timescale`.
    pub timescale: u32,
}

/// Convert an emsg to **version 1** (absolute `presentation_time` on the
/// Movie timeline).
///
/// Returns the emsg unchanged if it is already version 1.
pub fn emsg_to_v1<'a>(emsg: &EmsgBox<'a>, timing: &SegmentTiming) -> Result<EmsgBox<'a>> {
    validate_timescale(emsg.timescale, timing.timescale)?;

    let t = movie_timeline_t(emsg, timing);

    Ok(EmsgBox {
        scheme_id_uri: emsg.scheme_id_uri,
        value: emsg.value,
        timescale: emsg.timescale,
        presentation_time: PresentationTime::Absolute(t),
        event_duration: emsg.event_duration,
        id: emsg.id,
        message_data: emsg.message_data,
    })
}

/// Convert an emsg to **version 0** (segment-relative
/// `presentation_time_delta`).
///
/// Returns the emsg unchanged if it is already version 0.
pub fn emsg_to_v0<'a>(emsg: &EmsgBox<'a>, timing: &SegmentTiming) -> Result<EmsgBox<'a>> {
    validate_timescale(emsg.timescale, timing.timescale)?;

    let t = movie_timeline_t(emsg, timing);

    let delta = t
        .checked_sub(timing.earliest_presentation_time)
        .ok_or(Error::EmsgPresentationTimeBeforeEpt)?;

    if delta > u64::from(u32::MAX) {
        return Err(Error::EmsgDeltaOverflow(delta));
    }

    Ok(EmsgBox {
        scheme_id_uri: emsg.scheme_id_uri,
        value: emsg.value,
        timescale: emsg.timescale,
        presentation_time: PresentationTime::Delta(delta as u32),
        event_duration: emsg.event_duration,
        id: emsg.id,
        message_data: emsg.message_data,
    })
}

/// Compute the Movie-timeline instant `T` from either emsg version.
fn movie_timeline_t(emsg: &EmsgBox<'_>, timing: &SegmentTiming) -> u64 {
    match emsg.presentation_time {
        PresentationTime::Absolute(pt) => pt,
        PresentationTime::Delta(d) => timing.earliest_presentation_time + u64::from(d),
        #[allow(unreachable_patterns)]
        _ => unreachable!("non_exhaustive PresentationTime"),
    }
}

/// Validate that the emsg timescale matches the SegmentTiming timescale.
fn validate_timescale(emsg: u32, timing: u32) -> Result<()> {
    if emsg != timing {
        return Err(Error::EmsgTimescaleMismatch { emsg, timing });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v0_to_v1_to_v0_round_trips() {
        let emsg = EmsgBox {
            scheme_id_uri: "urn:scte:scte35:2013:bin",
            value: "1",
            timescale: 90_000,
            presentation_time: PresentationTime::Delta(0),
            event_duration: 2_160_000,
            id: 1,
            message_data: &[0xFC, 0x30, 0x21],
        };
        let timing = SegmentTiming {
            earliest_presentation_time: 500_000,
            presentation_time_offset: 0,
            timescale: 90_000,
        };
        let v1 = emsg_to_v1(&emsg, &timing).unwrap();
        assert_eq!(v1.presentation_time, PresentationTime::Absolute(500_000));
        let v0_back = emsg_to_v0(&v1, &timing).unwrap();
        assert_eq!(v0_back, emsg);
    }

    #[test]
    fn v1_to_v0_to_v1_round_trips() {
        let emsg = EmsgBox {
            scheme_id_uri: "urn:scte:scte35:2013:bin",
            value: "2",
            timescale: 90_000,
            presentation_time: PresentationTime::Absolute(1_000_000),
            event_duration: 0,
            id: 42,
            message_data: &[],
        };
        let timing = SegmentTiming {
            earliest_presentation_time: 900_000,
            presentation_time_offset: 0,
            timescale: 90_000,
        };
        let v0 = emsg_to_v0(&emsg, &timing).unwrap();
        assert_eq!(v0.presentation_time, PresentationTime::Delta(100_000));
        let v1_back = emsg_to_v1(&v0, &timing).unwrap();
        assert_eq!(v1_back, emsg);
    }

    #[test]
    fn timescale_mismatch_returns_error() {
        let emsg = EmsgBox {
            scheme_id_uri: "urn:scte:scte35:2013:bin",
            value: "",
            timescale: 90_000,
            presentation_time: PresentationTime::Delta(0),
            event_duration: 0,
            id: 0,
            message_data: &[],
        };
        let timing = SegmentTiming {
            earliest_presentation_time: 0,
            presentation_time_offset: 0,
            timescale: 48_000,
        };
        assert!(matches!(
            emsg_to_v1(&emsg, &timing),
            Err(Error::EmsgTimescaleMismatch { .. })
        ));
        assert!(matches!(
            emsg_to_v0(&emsg, &timing),
            Err(Error::EmsgTimescaleMismatch { .. })
        ));
    }

    #[test]
    fn v0_event_before_ept_errors() {
        let emsg = EmsgBox {
            scheme_id_uri: "urn:scte:scte35:2013:bin",
            value: "",
            timescale: 90_000,
            presentation_time: PresentationTime::Delta(10),
            event_duration: 0,
            id: 0,
            message_data: &[],
        };
        let timing = SegmentTiming {
            earliest_presentation_time: 1_000,
            presentation_time_offset: 0,
            timescale: 90_000,
        };

        // v1 with presentation_time < EPT: v1→v0 should fail.
        let v1_before = EmsgBox {
            presentation_time: PresentationTime::Absolute(500),
            ..emsg
        };
        assert!(matches!(
            emsg_to_v0(&v1_before, &timing),
            Err(Error::EmsgPresentationTimeBeforeEpt)
        ));
    }
}
