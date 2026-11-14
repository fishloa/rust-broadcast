//! The queue a segmenter publishes through to a [`Trunk`](crate::trunk::Trunk)
//! without ever blocking: parts and finished segments wait here in stream
//! order and [`drain_publish_queue`] hands them to the [`SegmentWriter`] as far
//! as it will take them, force-expiring a stalled DVR pin when the queue has
//! waited too long or grown too deep.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use transmux::ll_hls::PartInfo;

use crate::ids::{PartIndex, SegmentSeq};
use crate::trunk::{PartEntry, SegmentEntry, SegmentWriter, TryPublishSegmentError};

/// Hard cap on the number of **segments** (not parts — see below) queued in
/// a segmenter's pending-publish queue — bounds memory even before
/// [`PENDING_PUBLISH_MAX_WAIT`] elapses, for a DVR pin that never advances at
/// all (a permanently stalled disk, say). Once that many segments have
/// accumulated behind the stuck one, whichever `ArchiveOverrun::StallIngest`
/// pin is blocking it is force-expired immediately, the same as if the
/// deadline had already elapsed — see a segmenter's `drain_pending`.
///
/// Counts queued [`PendingPublish::Segment`] entries only, never
/// [`PendingPublish::Part`] ones: a part always rides behind its own segment
/// (queued immediately before it — see
/// the segmenter's publish-in-stream-order step) and a stuck segment cannot
/// itself unstick a later part, so a part sitting in the queue reflects
/// nothing about how "deep" the real backlog is. Counting every queued item
/// (parts included, as this constant used to) made an LL-HLS route hit this
/// cap in well under [`PENDING_PUBLISH_MAX_WAIT`]'s bound at typical
/// part-target cadences (a new part every ~200ms keeps re-filling the queue
/// long before 30s of wall-clock elapses), force-expiring a `StallIngest`
/// pin — and terminating a DVR recording relying on it — for a stall no
/// worse than a few seconds. Counting segments alone leaves the 30s deadline
/// as the one bound that matters for a merely-slow pin, while still
/// bounding memory for a pin that never advances at all.
pub const PENDING_PUBLISH_QUEUE_CAP: usize = 8;

/// How long the oldest entry in a segmenter's pending-publish queue
/// may wait before this segmenter force-expires whichever
/// `ArchiveOverrun::StallIngest` pin is blocking it
/// ([`SegmentWriter::expire_stalled_pins`]) and retries — applying, from
/// this non-blocking caller, the same bound
/// `media_plane::trunk::SegmentWriter::publish_segment`'s own internal
/// blocking wait uses (issue #1082's T6/T7 findings: a stalled DVR pin — a
/// disk write that never returns — must not freeze live output past this
/// bound).
pub const PENDING_PUBLISH_MAX_WAIT: Duration = Duration::from_secs(30);

/// One item queued by the segmenter's non-blocking publish —
/// either a live part or a finished segment, in the exact order they were
/// meant to reach [`SegmentWriter`], so a later segment's parts can never
/// appear before an earlier segment still stuck in the queue.
#[non_exhaustive]
pub enum PendingPublish {
    Part(PartEntry),
    Segment(SegmentEntry),
}

/// Converts a closed part to the `Trunk`'s [`PartEntry`] shape.
pub fn part_entry(part: PartInfo) -> PartEntry {
    PartEntry::new(
        part.bytes,
        SegmentSeq::new(part.segment_seq),
        PartIndex::new(part.part_index.value()),
        Duration::from_secs_f64(part.duration),
        part.independent,
    )
}

/// Drain `pending` into `segment_writer` from the front for as long as items keep
/// going through, in order; see a segmenter's `drain_pending`.
pub fn drain_publish_queue(
    segment_writer: &mut SegmentWriter,
    pending: &mut VecDeque<PendingPublish>,
    pending_since: &mut Option<Instant>,
) {
    loop {
        let Some(item) = pending.pop_front() else {
            *pending_since = None;
            return;
        };
        match item {
            PendingPublish::Part(part) => {
                segment_writer.publish_part(part);
                // The front item just published — whatever stall episode
                // `pending_since` was tracking (if any) is over. Reset it
                // here, not only when the queue drains empty, so a LATER
                // item that stalls starts its own fresh clock instead of
                // inheriting a stale timestamp from an earlier, already-
                // resolved stall (which would let `PENDING_PUBLISH_MAX_WAIT`
                // appear to have already elapsed for a stall that just
                // started).
                *pending_since = None;
            }
            PendingPublish::Segment(entry) => {
                match segment_writer.try_publish_segment(entry) {
                    Ok(()) => {
                        *pending_since = None;
                    }
                    Err(TryPublishSegmentError::NonMonotonic(e)) => {
                        // Should not happen: `try_new`/`apply_track_change`
                        // seed numbering from `next_sequence_number`. Not a
                        // reason to tear down the whole session — log and
                        // drop this one entry, exactly this crate's general
                        // "a segmentation defect on one program must not
                        // take the route down" posture.
                        tracing::error!(
                            error = %e,
                            "dropping a segment with a non-monotonic sequence_number"
                        );
                        *pending_since = None;
                    }
                    Err(TryPublishSegmentError::WouldStall(entry)) => {
                        pending.push_front(PendingPublish::Segment(entry));
                        let now = Instant::now();
                        let since = *pending_since.get_or_insert(now);
                        let waited_too_long =
                            now.saturating_duration_since(since) >= PENDING_PUBLISH_MAX_WAIT;
                        // Segments only — see `PENDING_PUBLISH_QUEUE_CAP`'s
                        // own doc for why a queued part must not count
                        // toward this cap.
                        let queued_segments = pending
                            .iter()
                            .filter(|item| matches!(item, PendingPublish::Segment(_)))
                            .count();
                        let queue_too_deep = queued_segments >= PENDING_PUBLISH_QUEUE_CAP;
                        if (waited_too_long || queue_too_deep)
                            && segment_writer.expire_stalled_pins() > 0
                        {
                            // The blocking pin just gave up (Terminated, not
                            // silently gapped — see `expire_stalled_pins`'s
                            // own doc): retry immediately rather than
                            // waiting for the next call.
                            continue;
                        }
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trunk::{Trunk, TrunkConfig};
    use broadcast_common::Timestamp;
    use core::num::NonZeroUsize;
    use transmux::SegmentMeta;

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("non-zero")
    }

    fn segment(seq: u32) -> SegmentEntry {
        SegmentEntry::new(
            vec![0u8; 4],
            SegmentSeq::new(seq),
            Duration::from_secs(1),
            Timestamp::from_nanos(0),
            SegmentMeta {
                discontinuous: false,
            },
        )
    }

    /// Parts and segments queued in stream order reach the writer in that order
    /// and leave the queue empty with its stall clock reset.
    #[test]
    fn a_drained_queue_is_empty_with_no_stall_clock() {
        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(8))
                .with_sparse_capacity(nz(8))
                .with_segment_capacity(nz(8))
                .with_event_capacity(nz(8))
                .with_part_capacity(nz(8)),
        );
        let mut writer = trunk.segment_writer().expect("segment writer");
        let mut pending = VecDeque::from([PendingPublish::Segment(segment(1))]);
        let mut since = Some(Instant::now());
        drain_publish_queue(&mut writer, &mut pending, &mut since);
        assert!(pending.is_empty());
        assert!(since.is_none());
    }
}
