//! The one report an RTMP session publishes for content it skipped.
//!
//! A session reads as well as it can: it keeps decoding past a message it
//! cannot frame, a command it does not know, an AMF0 value it cannot
//! interpret, and a media payload too large for an FLV tag. Every one of
//! those skips is published through `broadcast_core::damage::Damage` on the
//! session's own event channel ([`ServerEvent::Damage`] /
//! [`ClientEvent::Damage`]) in wire order with the events around it.
//!
//! What this session reports:
//!
//! - a message header that interrupts an in-progress message on the same
//!   chunk stream, and an `Abort` that discards one (`MediaUnitDropped`);
//! - a message whose declared length exceeds what this session will buffer,
//!   consumed without ever being held (`LimitReached`);
//! - a message on a stream id the session never opened, and media before
//!   `publish`/`play` (`MediaUnitDropped`);
//! - a message type, a user control event or a command name this engine does
//!   not decode (`MetadataUninterpreted`);
//! - an AMF0 ECMA-Array whose declared count disagrees with what it holds
//!   (`MetadataUninterpreted`);
//! - an aggregate sub-message whose framing does not hold, after the good
//!   ones before it were already delivered (`MediaUnitDropped`);
//! - a media payload that does not fit an FLV tag's 24-bit `DataSize`
//!   (`MediaUnitDropped`);
//! - input that ends inside a message (`InputTruncated`).
//!
//! What stays an error, and why: a chunk stream that cannot be framed at all
//! ([`RtmpError::ChunkStreamState`] on a csid with no prior header to inherit
//! from), a chunk size outside the protocol's range
//! ([`RtmpError::ChunkSizeOutOfRange`]), a handshake this engine does not
//! speak, and the unparsed-input backstop
//! ([`RtmpError::InputBudgetExceeded`]). Each of those means the framing
//! itself is gone, so no later message can be located — there is nothing to
//! continue past.
//!
//! The caller decides what a report means
//! ([`DamagePolicy`](broadcast_core::damage::DamagePolicy)): the default
//! publishes it and keeps reading; `Fail` ends the session with
//! [`RtmpError::Damage`] carrying the same report.
//!
//! [`ServerEvent::Damage`]: crate::server::ServerEvent::Damage
//! [`ClientEvent::Damage`]: crate::client::ClientEvent::Damage

use alloc::boxed::Box;
use alloc::vec::Vec;

use broadcast_core::damage::{Damage, DamageKind, DamageLocation, MediaTimeRange, Severity};
use broadcast_core::timescale::Timescale;

use crate::error::RtmpError;

/// Timescale of an RTMP timestamp: whole milliseconds (§5.1.2).
pub(crate) const MILLIS: Timescale = broadcast_core::timescale!(1_000);

/// A location naming a message's own timestamp, when the header carried one.
pub(crate) fn at_millis(timestamp_ms: u32) -> DamageLocation {
    let tick = i64::from(timestamp_ms);
    DamageLocation::new().with_media_time(MediaTimeRange::new(MILLIS, tick, tick))
}

/// A media message discarded before it could be delivered.
pub(crate) fn message_dropped(timestamp_ms: u32) -> Damage {
    Damage::new(
        DamageKind::MediaUnitDropped,
        Severity::Degraded,
        "RTMP message",
    )
    .with_location(at_millis(timestamp_ms))
}

/// A message the session declined to buffer because of a limit.
pub(crate) fn message_over_limit(timestamp_ms: u32) -> Damage {
    Damage::new(DamageKind::LimitReached, Severity::Degraded, "RTMP message")
        .with_location(at_millis(timestamp_ms))
}

/// A message a peer's `Abort` told the session to discard.
pub(crate) fn message_aborted(timestamp_ms: u32) -> Damage {
    Damage::new(
        DamageKind::MediaUnitDropped,
        Severity::Informational,
        "RTMP aborted message",
    )
    .with_location(at_millis(timestamp_ms))
}

/// Content that never belonged to the session's stream.
pub(crate) fn outside_stream(what: &'static str, timestamp_ms: u32) -> Damage {
    Damage::new(DamageKind::MediaUnitDropped, Severity::Informational, what)
        .with_location(at_millis(timestamp_ms))
}

/// A structure the session passed over without interpreting.
pub(crate) fn uninterpreted(what: &'static str) -> Damage {
    Damage::new(
        DamageKind::MetadataUninterpreted,
        Severity::Informational,
        what,
    )
}

/// Input that ended inside a message. The reader holds the partial message
/// but cannot name its offset in a forward-only stream, so no location is
/// fabricated. Only the `tokio` adapter can observe an end of input, so this
/// is compiled with it.
#[cfg(feature = "tokio")]
pub(crate) fn truncated() -> Damage {
    Damage::new(
        DamageKind::InputTruncated,
        Severity::Degraded,
        "RTMP message in progress at end of input",
    )
}

/// Hand one report to the caller's policy: append it to `events` through
/// `wrap`, or stop with the same report inside [`RtmpError::Damage`].
pub(crate) fn publish<E>(
    policy: &broadcast_core::damage::DamagePolicy,
    damage: Damage,
    events: &mut Vec<E>,
    wrap: fn(Box<Damage>) -> E,
) -> Result<(), RtmpError> {
    match policy.admit(damage) {
        Ok(damage) => {
            events.push(wrap(Box::new(damage)));
            Ok(())
        }
        Err(error) => Err(RtmpError::Damage(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_core::damage::DamagePolicy;

    #[test]
    fn every_constructor_carries_a_kind_and_a_severity() {
        let reports = [
            message_dropped(7),
            message_over_limit(7),
            message_aborted(7),
            outside_stream("RTMP pre-publish media", 7),
            uninterpreted("RTMP command"),
        ];
        for report in &reports {
            assert!(!report.what().is_empty());
            assert!(!report.kind().name().is_empty());
            assert!(!report.severity().name().is_empty());
        }
        assert_eq!(reports[0].kind(), DamageKind::MediaUnitDropped);
        assert_eq!(reports[1].kind(), DamageKind::LimitReached);
        assert_eq!(reports[2].severity(), Severity::Informational);
        assert_eq!(reports[4].kind(), DamageKind::MetadataUninterpreted);
    }

    #[test]
    fn a_message_report_names_its_timestamp_in_milliseconds() {
        let report = message_dropped(1_500);
        let time = report.location().media_time().expect("media time");
        assert_eq!(time.timescale(), MILLIS);
        assert_eq!((time.start(), time.end()), (1_500, 1_500));
        #[cfg(feature = "tokio")]
        assert_eq!(truncated().location().byte_range(), None);
    }

    #[test]
    fn the_default_policy_publishes_and_a_failing_one_stops() {
        let mut events: Vec<Box<Damage>> = Vec::new();
        publish(
            &DamagePolicy::new(),
            message_dropped(1),
            &mut events,
            core::convert::identity,
        )
        .expect("the default policy reports");
        assert_eq!(events.len(), 1);

        let refused = publish(
            &DamagePolicy::new().fail_on(Severity::Degraded),
            message_dropped(2),
            &mut events,
            core::convert::identity,
        )
        .expect_err("a failing policy stops");
        assert!(matches!(refused, RtmpError::Damage(_)));
        assert_eq!(events.len(), 1, "the refused report is not also published");
    }
}
