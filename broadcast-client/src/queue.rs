//! Bounded event queue. When the host does not drain it and it exceeds its cap, the oldest
//! droppable events are discarded in O(1) amortised time: transient statuses (stalled,
//! reconnecting, live, establishing) and starvation notices coalesce to their latest, then the
//! oldest samples go and are counted. A dropped sample leaves one [`Event::Discontinuity`] in its place
//! per burst of drops so the player re-syncs; every other event (tracks, errors, buffer events,
//! terminal statuses, ...) is kept, so the queue is bounded by the cap plus the undroppable
//! events a stalled host leaves unread. A byte budget on the queued sample payloads sheds the
//! oldest samples the same way, so a few very large samples cannot outgrow the event count. The
//! droppable kept events (a discontinuity, an A/V sync announcement) have a ceiling of their own,
//! and a discontinuity queued right behind another coalesces into it, so a stream that keeps
//! resetting cannot grow them without bound either.

use alloc::collections::VecDeque;

use crate::events::{Event, Status};

const MIN_TOMBSTONES_BEFORE_COMPACTION: usize = 64;
/// Most [`Event::Discontinuity`] and [`Event::AvSync`] values the queue holds at once; past it the
/// oldest is dropped.
const KEEP_LANE_CEILING: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Sample,
    Status,
    Starved,
    Keep,
}

fn kind_of(e: &Event) -> Kind {
    match e {
        Event::Sample { .. } => Kind::Sample,
        Event::Starved(_) => Kind::Starved,
        Event::Status(s) => match s {
            Status::Ended | Status::NotFound | Status::WaitingForCredentials => Kind::Keep,
            _ => Kind::Status,
        },
        _ => Kind::Keep,
    }
}

/// `true` for a kept event that may still go once the keep lane is over its ceiling: a
/// discontinuity a player can recover from by the next one, and an A/V sync announcement.
fn is_droppable_keep(e: &Event) -> bool {
    matches!(e, Event::Discontinuity | Event::AvSync(_))
}

#[derive(Default)]
struct Lanes {
    samples: VecDeque<u64>,
    statuses: VecDeque<u64>,
    starved: VecDeque<u64>,
    keeps: VecDeque<u64>,
}

impl Lanes {
    fn lane(&mut self, kind: Kind) -> Option<&mut VecDeque<u64>> {
        match kind {
            Kind::Sample => Some(&mut self.samples),
            Kind::Status => Some(&mut self.statuses),
            Kind::Starved => Some(&mut self.starved),
            Kind::Keep => None,
        }
    }

    fn next_victim(&mut self) -> Option<u64> {
        if self.statuses.len() > 1 {
            return self.statuses.pop_front();
        }
        if self.starved.len() > 1 {
            return self.starved.pop_front();
        }
        self.samples.pop_front()
    }

    fn clear(&mut self) {
        self.samples.clear();
        self.statuses.clear();
        self.starved.clear();
        self.keeps.clear();
    }
}

pub(crate) struct EventQueue {
    slots: VecDeque<Option<Event>>,
    base: u64,
    live: usize,
    tombstones: usize,
    lanes: Lanes,
    last_discontinuity: Option<u64>,
    gap_end: Option<u64>,
    cap: usize,
    byte_budget: usize,
    bytes: usize,
    dropped: u64,
}

fn payload_len(e: &Event) -> usize {
    match e {
        Event::Sample { sample, .. } => sample.data.len(),
        _ => 0,
    }
}

impl EventQueue {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            slots: VecDeque::new(),
            base: 0,
            live: 0,
            tombstones: 0,
            lanes: Lanes::default(),
            last_discontinuity: None,
            gap_end: None,
            cap,
            byte_budget: usize::MAX,
            bytes: 0,
            dropped: 0,
        }
    }

    pub(crate) fn with_byte_budget(mut self, byte_budget: usize) -> Self {
        self.byte_budget = byte_budget;
        self
    }

    fn next_seq(&self) -> u64 {
        self.base.saturating_add(self.slots.len() as u64)
    }

    pub(crate) fn push_back(&mut self, e: Event) {
        if matches!(e, Event::Discontinuity) && self.newest_is_discontinuity() {
            return;
        }
        let seq = self.next_seq();
        if is_droppable_keep(&e) {
            self.lanes.keeps.push_back(seq);
        }
        if let Some(lane) = self.lanes.lane(kind_of(&e)) {
            lane.push_back(seq);
        }
        if matches!(e, Event::Discontinuity) {
            self.last_discontinuity = Some(seq);
        }
        self.bytes = self.bytes.saturating_add(payload_len(&e));
        self.slots.push_back(Some(e));
        self.live = self.live.saturating_add(1);
        while self.live > self.cap || self.bytes > self.byte_budget {
            let Some(victim) = self.lanes.next_victim() else {
                break;
            };
            self.drop_event(victim);
        }
        while self.lanes.keeps.len() > KEEP_LANE_CEILING {
            let Some(victim) = self.lanes.keeps.pop_front() else {
                break;
            };
            self.drop_event(victim);
        }
    }

    /// `true` when the newest live slot is a [`Event::Discontinuity`], so one queued now would
    /// only repeat it.
    fn newest_is_discontinuity(&self) -> bool {
        self.slots
            .iter()
            .rev()
            .find_map(Option::as_ref)
            .is_some_and(|e| matches!(e, Event::Discontinuity))
    }

    pub(crate) fn extend(&mut self, events: impl IntoIterator<Item = Event>) {
        for e in events {
            self.push_back(e);
        }
    }

    pub(crate) fn pop_front(&mut self) -> Option<Event> {
        loop {
            let slot = self.slots.pop_front()?;
            let seq = self.base;
            self.base = self.base.saturating_add(1);
            match slot {
                Some(e) => {
                    self.live = self.live.saturating_sub(1);
                    self.bytes = self.bytes.saturating_sub(payload_len(&e));
                    if let Some(lane) = self.lanes.lane(kind_of(&e))
                        && lane.front() == Some(&seq)
                    {
                        lane.pop_front();
                    }
                    if is_droppable_keep(&e) && self.lanes.keeps.front() == Some(&seq) {
                        self.lanes.keeps.pop_front();
                    }
                    return Some(e);
                }
                None => self.tombstones = self.tombstones.saturating_sub(1),
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.slots.clear();
        self.lanes.clear();
        self.base = 0;
        self.live = 0;
        self.bytes = 0;
        self.tombstones = 0;
        self.last_discontinuity = None;
        self.gap_end = None;
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.live == 0
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.live
    }

    pub(crate) fn dropped(&self) -> u64 {
        self.dropped
    }

    fn index_of(&self, seq: u64) -> Option<usize> {
        seq.checked_sub(self.base)
            .and_then(|i| usize::try_from(i).ok())
            .filter(|i| *i < self.slots.len())
    }

    fn drop_event(&mut self, seq: u64) {
        let Some(index) = self.index_of(seq) else {
            return;
        };
        let Some(event) = self.slots.get_mut(index).and_then(Option::take) else {
            return;
        };
        self.bytes = self.bytes.saturating_sub(payload_len(&event));
        let is_sample = kind_of(&event) == Kind::Sample;
        if is_sample {
            self.dropped = self.dropped.saturating_add(1);
        }
        let contiguous = self.gap_end.is_some_and(|end| end.saturating_add(1) == seq);
        let marker_present = self
            .last_discontinuity
            .is_some_and(|at| at < seq && at >= self.base);
        if is_sample && !contiguous && !marker_present {
            if let Some(slot) = self.slots.get_mut(index) {
                *slot = Some(Event::Discontinuity);
            }
            self.last_discontinuity = Some(seq);
            self.gap_end = Some(seq);
            return;
        }
        self.live = self.live.saturating_sub(1);
        self.tombstones = self.tombstones.saturating_add(1);
        if contiguous || is_sample {
            self.gap_end = Some(seq);
        }
        if matches!(event, Event::Discontinuity) {
            self.refresh_discontinuity();
        }
        self.trim_front();
        if self.tombstones > self.live.max(MIN_TOMBSTONES_BEFORE_COMPACTION) {
            self.compact();
        }
    }

    fn trim_front(&mut self) {
        while matches!(self.slots.front(), Some(None)) {
            self.slots.pop_front();
            self.base = self.base.saturating_add(1);
            self.tombstones = self.tombstones.saturating_sub(1);
        }
    }

    fn compact(&mut self) {
        let kept: VecDeque<Option<Event>> = self.slots.drain(..).filter(Option::is_some).collect();
        self.slots = kept;
        self.tombstones = 0;
        self.lanes.clear();
        self.last_discontinuity = None;
        self.gap_end = None;
        for (offset, slot) in self.slots.iter().enumerate() {
            let seq = self.base.saturating_add(offset as u64);
            let Some(e) = slot else {
                continue;
            };
            if matches!(e, Event::Discontinuity) {
                self.last_discontinuity = Some(seq);
            }
            if is_droppable_keep(e) {
                self.lanes.keeps.push_back(seq);
            }
            if let Some(lane) = self.lanes.lane(kind_of(e)) {
                lane.push_back(seq);
            }
        }
    }

    /// Points `last_discontinuity` at the newest live discontinuity, or clears it when the one it
    /// named has just been dropped.
    fn refresh_discontinuity(&mut self) {
        let base = self.base;
        self.last_discontinuity = self
            .slots
            .iter()
            .enumerate()
            .rev()
            .find_map(|(offset, slot)| match slot {
                Some(Event::Discontinuity) => Some(base.saturating_add(offset as u64)),
                _ => None,
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{BufferEvent, ErrorKind, Status};
    use crate::tshttp::StarveKind;
    use core::time::Duration as StdDuration;
    use transmux::{JoinDecision, RapKind, Sample, TrackId};

    const TRACK: TrackId = TrackId::new(1);
    const BURST: i64 = 100;
    const TINY_CAP: usize = 4;

    fn base_sample() -> Sample {
        static BASE: std::sync::OnceLock<Sample> = std::sync::OnceLock::new();
        BASE.get_or_init(|| {
            let data = test_fixtures::read("ts/h264-aac.ts");
            let media = test_harness::collect::media_from_ts(&data[..]).unwrap();
            media.tracks[0].samples[0].clone()
        })
        .clone()
    }

    fn sample_event(n: i64, join: Option<JoinDecision>) -> Event {
        let mut sample = base_sample();
        if let Some(t) = sample.timing.as_mut() {
            let dts = t.dts().unwrap_or(n);
            *t = t.with_composition(dts, n);
        } else {
            sample.timing = Some(transmux::ir::SampleTiming::timed(n, n));
        }
        Event::Sample {
            track: TRACK,
            sample,
            join,
        }
    }

    fn pts_of(e: &Event) -> Option<i64> {
        match e {
            Event::Sample { sample, .. } => sample.timing.and_then(|t| t.pts()),
            _ => None,
        }
    }

    fn drain(q: &mut EventQueue) -> Vec<Event> {
        std::iter::from_fn(|| q.pop_front()).collect()
    }

    fn discontinuities(events: &[Event]) -> usize {
        events
            .iter()
            .filter(|e| matches!(e, Event::Discontinuity))
            .count()
    }

    #[test]
    fn status_events_alone_cannot_grow_the_queue_past_the_cap() {
        let mut q = EventQueue::new(TINY_CAP);
        for n in 0..10_000u64 {
            q.push_back(Event::Status(Status::Stalled {
                stalled: StdDuration::from_secs(n),
            }));
        }
        assert!(q.len() <= TINY_CAP, "len {}", q.len());
        let events = drain(&mut q);
        assert_eq!(
            events.last(),
            Some(&Event::Status(Status::Stalled {
                stalled: StdDuration::from_secs(9_999),
            })),
            "the latest status survives"
        );
    }

    #[test]
    fn a_dropped_keyframe_leaves_a_discontinuity_in_its_place() {
        let mut q = EventQueue::new(3);
        q.push_back(sample_event(0, Some(JoinDecision::JoinPoint(RapKind::Idr))));
        for n in 1..4 {
            q.push_back(sample_event(n, Some(JoinDecision::Keep)));
        }
        let events = drain(&mut q);
        assert_eq!(events.first(), Some(&Event::Discontinuity), "{events:?}");
        let pts: Vec<i64> = events.iter().filter_map(pts_of).collect();
        assert_eq!(pts, vec![2, 3], "newest samples kept in order");
        assert!(q.dropped() >= 2);
    }

    #[test]
    fn one_marker_per_burst_of_drops() {
        let mut q = EventQueue::new(TINY_CAP);
        for n in 0..BURST {
            q.push_back(sample_event(n, None));
        }
        let events = drain(&mut q);
        assert_eq!(discontinuities(&events), 1, "{}", events.len());
        let pts: Vec<i64> = events.iter().filter_map(pts_of).collect();
        let kept = i64::try_from(pts.len()).unwrap();
        assert_eq!(pts, ((BURST - kept)..BURST).collect::<Vec<_>>());
        assert_eq!(
            q.dropped(),
            u64::try_from(BURST - kept).unwrap(),
            "every dropped sample is counted"
        );
    }

    #[test]
    fn a_second_burst_after_the_host_drained_gets_its_own_marker() {
        let mut q = EventQueue::new(3);
        for n in 0..4 {
            q.push_back(sample_event(n, None));
        }
        assert_eq!(q.pop_front(), Some(Event::Discontinuity));
        q.pop_front();
        for n in 4..7 {
            q.push_back(sample_event(n, None));
        }
        let events = drain(&mut q);
        assert_eq!(events.first(), Some(&Event::Discontinuity), "{events:?}");
        assert_eq!(discontinuities(&events), 1);
    }

    #[test]
    fn a_discontinuity_queued_behind_another_coalesces_into_it() {
        let mut q = EventQueue::new(16);
        q.push_back(Event::Status(Status::Live));
        for _ in 0..5 {
            q.push_back(Event::Discontinuity);
        }
        let events = drain(&mut q);
        assert_eq!(discontinuities(&events), 1, "{events:?}");
        assert_eq!(events.len(), 2, "one marker after the live status");
    }

    #[test]
    fn the_keep_lane_stops_at_its_ceiling_dropping_the_oldest() {
        let mut q = EventQueue::new(4 * KEEP_LANE_CEILING);
        q.push_back(Event::Tracks(Vec::new()));
        q.push_back(Event::Status(Status::Ended));
        for _ in 0..=KEEP_LANE_CEILING {
            q.push_back(Event::Discontinuity);
            q.push_back(Event::AvSync(transmux::AvSync::Provisional));
        }
        let events = drain(&mut q);
        let kept = events.iter().filter(|e| is_droppable_keep(e)).count();
        assert_eq!(
            kept, KEEP_LANE_CEILING,
            "the oldest discontinuity and A/V sync announcements were dropped"
        );
        assert!(
            events.iter().any(|e| matches!(e, Event::Tracks(_))),
            "an undroppable kept event survives the ceiling"
        );
        assert!(events.contains(&Event::Status(Status::Ended)));
    }

    #[test]
    fn tracks_errors_and_buffer_events_are_never_dropped() {
        let mut q = EventQueue::new(2);
        q.push_back(Event::Tracks(Vec::new()));
        q.push_back(Event::Error {
            kind: ErrorKind::BadKey,
            fatal: false,
        });
        q.push_back(Event::Buffer(BufferEvent::Start { at_27mhz: 1 }));
        for n in 0..BURST {
            q.push_back(sample_event(n, None));
            q.push_back(Event::Status(Status::Live));
        }
        let events = drain(&mut q);
        assert!(matches!(events[0], Event::Tracks(_)), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, Event::Error { .. })));
        assert!(events.iter().any(|e| matches!(e, Event::Buffer(_))));
    }

    #[test]
    fn terminal_statuses_survive_a_flood() {
        let mut q = EventQueue::new(2);
        q.push_back(Event::Status(Status::Ended));
        for n in 0..BURST {
            q.push_back(sample_event(n, None));
        }
        let events = drain(&mut q);
        assert!(events.contains(&Event::Status(Status::Ended)), "{events:?}");
    }

    #[test]
    fn memory_stays_bounded_behind_a_stuck_undroppable_event() {
        let mut q = EventQueue::new(TINY_CAP);
        q.push_back(Event::Tracks(Vec::new()));
        for n in 0..100_000 {
            q.push_back(sample_event(n % BURST, None));
        }
        assert!(q.len() <= TINY_CAP + 2, "len {}", q.len());
        assert!(
            q.slots.len() <= 4 * TINY_CAP + 128,
            "slots {}",
            q.slots.len()
        );
    }

    const PAYLOAD: usize = 1000;

    fn sized_sample_event(n: i64, len: usize) -> Event {
        let mut sample = base_sample();
        if let Some(t) = sample.timing.as_mut() {
            let dts = t.dts().unwrap_or(n);
            *t = t.with_composition(dts, n);
        } else {
            sample.timing = Some(transmux::ir::SampleTiming::timed(n, n));
        }
        sample.data = vec![0u8; len].into();
        Event::Sample {
            track: TRACK,
            sample,
            join: None,
        }
    }

    #[test]
    fn the_byte_budget_drops_the_oldest_samples_and_counts_them() {
        let mut q = EventQueue::new(usize::MAX).with_byte_budget(3 * PAYLOAD);
        for n in 0..10 {
            q.push_back(sized_sample_event(n, PAYLOAD));
        }
        let events = drain(&mut q);
        let kept: Vec<i64> = events.iter().filter_map(pts_of).collect();
        assert_eq!(kept, [7, 8, 9], "the newest samples that fit the budget");
        assert_eq!(q.dropped(), 7);
        assert_eq!(discontinuities(&events), 1, "one marker per burst of drops");
    }

    #[test]
    fn drained_bytes_free_the_budget() {
        let mut q = EventQueue::new(usize::MAX).with_byte_budget(2 * PAYLOAD);
        for round in 0..5 {
            q.push_back(sized_sample_event(round, PAYLOAD));
            q.push_back(sized_sample_event(round, PAYLOAD));
            assert_eq!(drain(&mut q).iter().filter_map(pts_of).count(), 2);
        }
        assert_eq!(q.dropped(), 0, "a host that keeps up never loses a sample");
    }

    #[test]
    fn a_sample_larger_than_the_whole_budget_is_dropped_and_other_events_survive() {
        let mut q = EventQueue::new(usize::MAX).with_byte_budget(PAYLOAD);
        q.push_back(Event::Status(Status::Ended));
        q.push_back(sized_sample_event(1, 2 * PAYLOAD));
        let events = drain(&mut q);
        assert_eq!(q.dropped(), 1);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Status(Status::Ended)))
        );
        assert!(events.iter().all(|e| pts_of(e).is_none()));
    }

    #[test]
    fn starvation_notices_coalesce_to_one_and_stay_bounded() {
        let mut q = EventQueue::new(TINY_CAP);
        for n in 0..10_000u64 {
            let kind = if n % 2 == 0 {
                StarveKind::Data
            } else {
                StarveKind::NoData
            };
            q.push_back(Event::Starved(kind));
        }
        assert!(q.len() <= TINY_CAP + 1, "len {}", q.len());
        let events = drain(&mut q);
        let notices: Vec<&Event> = events
            .iter()
            .filter(|e| matches!(e, Event::Starved(_)))
            .collect();
        assert!(notices.len() <= TINY_CAP, "{events:?}");
        assert_eq!(
            notices.last().copied(),
            Some(&Event::Starved(StarveKind::NoData)),
            "the latest notice survives"
        );
    }

    #[test]
    fn terminal_statuses_survive_beside_transient_statuses_and_samples() {
        let terminal = [
            Status::Ended,
            Status::NotFound,
            Status::WaitingForCredentials,
        ];
        for status in terminal {
            let mut q = EventQueue::new(TINY_CAP);
            q.push_back(Event::Status(status.clone()));
            for n in 0..BURST {
                q.push_back(Event::Status(Status::Live));
                q.push_back(sample_event(n, None));
            }
            let events = drain(&mut q);
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e, Event::Status(s) if *s == status)),
                "{status:?} was shed: {events:?}"
            );
        }
    }

    #[test]
    fn one_starved_notice_survives_while_samples_are_dropped() {
        let mut q = EventQueue::new(TINY_CAP);
        q.push_back(Event::Starved(StarveKind::NoData));
        for n in 0..BURST {
            q.push_back(sample_event(n, None));
        }
        let events = drain(&mut q);
        assert!(
            events.contains(&Event::Starved(StarveKind::NoData)),
            "the only starved notice was dropped: {events:?}"
        );
        assert!(q.dropped() > 0, "samples were dropped");
    }

    #[test]
    fn a_fully_drained_queue_leaves_every_lane_empty() {
        let mut q = EventQueue::new(8);
        for n in 0..1_000 {
            q.push_back(sample_event(n, None));
            q.push_back(Event::Status(Status::Live));
            q.push_back(Event::Starved(StarveKind::Data));
            if n % 10 == 0 {
                q.push_back(Event::Discontinuity);
                q.push_back(Event::AvSync(transmux::AvSync::Provisional));
            }
            while q.pop_front().is_some() {}
        }
        assert!(q.lanes.samples.is_empty(), "samples lane not empty");
        assert!(q.lanes.statuses.is_empty(), "statuses lane not empty");
        assert!(q.lanes.starved.is_empty(), "starved lane not empty");
        assert!(q.lanes.keeps.is_empty(), "keeps lane not empty");
        assert_eq!(q.live, 0);
        assert_eq!(q.slots.len(), 0);
    }

    #[test]
    fn each_burst_of_adjacent_drops_plants_exactly_one_marker() {
        let mut q = EventQueue::new(TINY_CAP);
        for n in 0..TINY_CAP as i64 + 1 {
            q.push_back(sample_event(n, None));
        }
        while q.pop_front().is_some() {}
        for n in 10..16 {
            q.push_back(sample_event(n, None));
        }
        let events = drain(&mut q);
        assert_eq!(
            discontinuities(&events),
            1,
            "one marker marks the burst: {events:?}"
        );
        let pts: Vec<i64> = events.iter().filter_map(pts_of).collect();
        assert_eq!(pts, vec![13, 14, 15], "the newest samples survive");
    }

    #[test]
    fn tombstones_stay_bounded_when_a_stuck_event_blocks_the_front() {
        let mut q = EventQueue::new(TINY_CAP);
        q.push_back(Event::Tracks(Vec::new()));
        for n in 0..10_000 {
            q.push_back(sample_event(n % BURST, None));
        }
        assert!(
            q.slots.len() <= q.live + MIN_TOMBSTONES_BEFORE_COMPACTION + 1,
            "slots {} live {}",
            q.slots.len(),
            q.live
        );
    }

    #[test]
    fn the_oldest_marker_dropped_by_the_keep_ceiling_is_replaced_by_the_next_gap() {
        let mut q = EventQueue::new(8);
        q.push_back(Event::Discontinuity);
        for _ in 0..KEEP_LANE_CEILING {
            q.push_back(Event::AvSync(transmux::AvSync::Provisional));
        }
        q.push_back(sample_event(1, None));
        let events = drain(&mut q);
        assert_eq!(
            discontinuities(&events),
            1,
            "the gap after the dropped marker gets its own marker ({} events)",
            events.len()
        );
    }

    #[test]
    fn a_younger_live_marker_is_not_duplicated_when_an_older_one_is_dropped() {
        let mut q = EventQueue::new(8);
        q.push_back(Event::Discontinuity);
        for _ in 0..KEEP_LANE_CEILING {
            q.push_back(Event::AvSync(transmux::AvSync::Provisional));
        }
        q.push_back(Event::Discontinuity);
        q.push_back(sample_event(1, None));
        let events = drain(&mut q);
        assert_eq!(
            discontinuities(&events),
            1,
            "the younger live marker already covers the gap ({} events)",
            events.len()
        );
    }
}
