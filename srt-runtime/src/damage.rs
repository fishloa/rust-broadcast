//! Turning the delivery engine's drops into the workspace's one damage report.
//!
//! SRT delivers payloads, not a byte stream with positions, so a loss is
//! reported against the packet sequence the reader counts
//! ([`DamageLocation::ordinal`](broadcast_core::damage::DamageLocation))
//! and, when the scheduler knew it, the play time the packet was due at
//! ([`MediaTimeRange`]).
//!
//! # Coalescing rule
//!
//! One report per contiguous run of dropped sequence numbers that share a
//! reason. A run ends when a sequence number is delivered, when the next
//! dropped number is not the successor of the previous one, or when the
//! reason changes. A stream that loses every other packet therefore reports
//! once per single-packet run, and one that loses a run of 200 consecutive
//! packets reports that run once — never one report per packet.

use alloc::vec::Vec;

use broadcast_core::damage::{Damage, DamageKind, MediaTimeRange, Severity};
use broadcast_core::timescale::Timescale;

use crate::arq::seq;
use crate::tsbpd::{DropReason, DroppedPacket};

/// The time base of an SRT timestamp: the wire carries microseconds
/// (`draft-sharabayko-srt-01` §3.1).
pub const DAMAGE_TIMESCALE: Timescale = broadcast_core::timescale!(1_000_000);

/// The structure name a report about a dropped data packet uses, in SRT's own
/// terms.
#[must_use]
pub const fn what(reason: DropReason) -> &'static str {
    match reason {
        DropReason::TooLate => "SRT data packet (too late)",
        DropReason::GapSkipped => "SRT data packet (unrecoverable gap)",
        DropReason::DropRequested => "SRT data packet (peer drop request)",
    }
}

/// One report per contiguous run of `dropped` that shares a reason.
///
/// `dropped` is in the order the scheduler reported it (increasing sequence
/// numbers within a release pass).
#[must_use]
pub fn reports(dropped: &[DroppedPacket]) -> Vec<Damage> {
    let mut out = Vec::new();
    let mut run: Option<(DroppedPacket, DroppedPacket)> = None;
    for &packet in dropped {
        match run {
            Some((first, last))
                if first.reason == packet.reason
                    && packet.seq_number == seq::seq_next(last.seq_number) =>
            {
                run = Some((first, packet));
            }
            Some((first, last)) => {
                out.push(report(first, last));
                run = Some((packet, packet));
            }
            None => run = Some((packet, packet)),
        }
    }
    if let Some((first, last)) = run {
        out.push(report(first, last));
    }
    out
}

/// One run's report: the first sequence number as the ordinal, and the play
/// time span the run was due at when the scheduler recorded it.
fn report(first: DroppedPacket, last: DroppedPacket) -> Damage {
    let mut damage = Damage::new(
        DamageKind::MediaUnitDropped,
        Severity::Degraded,
        what(first.reason),
    )
    .with_ordinal(u64::from(first.seq_number));
    if let Some(start) = first.due_us {
        let end = last.due_us.unwrap_or(start);
        damage = damage.at_media_time(MediaTimeRange::new(
            DAMAGE_TIMESCALE,
            i64::try_from(start).unwrap_or(i64::MAX),
            i64::try_from(end).unwrap_or(i64::MAX),
        ));
    }
    damage
}

/// A report for content this adapter dropped before the scheduler saw it:
/// a datagram over the configured size, an inbound datagram the bounded
/// channel had no room for, or a payload the application read too slowly.
///
/// Coalesced by cause: the caller emits the first occurrence of each cause
/// once, `count` times the channel capacity, rather than one report per
/// datagram (a flood of them would say nothing more).
#[must_use]
pub fn dropped_before_delivery(what: &'static str, count: u64) -> Damage {
    Damage::new(DamageKind::MediaUnitDropped, Severity::Degraded, what).with_ordinal(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(seq_number: u32, reason: DropReason, due_us: Option<u64>) -> DroppedPacket {
        DroppedPacket::new(seq_number, reason, due_us)
    }

    #[test]
    fn one_run_is_one_report() {
        let dropped = [
            p(10, DropReason::GapSkipped, None),
            p(11, DropReason::GapSkipped, None),
            p(12, DropReason::GapSkipped, None),
        ];
        let reports = reports(&dropped);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].location().ordinal(), Some(10));
        assert_eq!(reports[0].kind(), DamageKind::MediaUnitDropped);
        assert_eq!(reports[0].severity(), Severity::Degraded);
        assert_eq!(reports[0].what(), what(DropReason::GapSkipped));
    }

    #[test]
    fn a_non_adjacent_drop_starts_a_new_report() {
        let dropped = [
            p(10, DropReason::TooLate, None),
            p(11, DropReason::TooLate, None),
            p(20, DropReason::TooLate, None),
        ];
        let reports = reports(&dropped);
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].location().ordinal(), Some(10));
        assert_eq!(reports[1].location().ordinal(), Some(20));
    }

    #[test]
    fn a_reason_change_starts_a_new_report() {
        let dropped = [
            p(10, DropReason::TooLate, None),
            p(11, DropReason::DropRequested, None),
        ];
        let reports = reports(&dropped);
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[1].what(), what(DropReason::DropRequested));
    }

    #[test]
    fn the_run_carries_its_play_time_span_when_known() {
        let dropped = [
            p(4, DropReason::TooLate, Some(1_000)),
            p(5, DropReason::TooLate, Some(3_000)),
        ];
        let reports = reports(&dropped);
        assert_eq!(reports.len(), 1);
        let time = reports[0].location().media_time().expect("media time");
        assert_eq!(time.timescale().get(), 1_000_000);
        assert_eq!((time.start(), time.end()), (1_000, 3_000));
    }

    #[test]
    fn a_wrapped_sequence_run_stays_one_report() {
        let dropped = [
            p(u32::MAX, DropReason::GapSkipped, None),
            p(0, DropReason::GapSkipped, None),
        ];
        assert_eq!(reports(&dropped).len(), 1);
    }

    #[test]
    fn no_drops_no_reports() {
        assert!(reports(&[]).is_empty());
    }
}
