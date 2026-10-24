//! Bounded event queue. When the host does not drain it and it exceeds its cap, the oldest
//! droppable events are discarded in O(1) amortised time: transient statuses (stalled,
//! reconnecting, live, establishing) and starvation notices coalesce to their latest, then the
//! oldest samples go and are counted. A dropped sample leaves one [`Event::Discontinuity`] in its place
//! per burst of drops so the player re-syncs; every other event (tracks, errors, buffer events,
//! terminal statuses, ...) is kept, so the queue is bounded by the cap plus the undroppable
//! events a stalled host leaves unread.

use alloc::collections::VecDeque;

use crate::events::{Event, Status};

const MIN_TOMBSTONES_BEFORE_COMPACTION: usize = 64;

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

#[derive(Default)]
struct Lanes {
    samples: VecDeque<u64>,
    statuses: VecDeque<u64>,
    starved: VecDeque<u64>,
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
    dropped: u64,
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
            dropped: 0,
        }
    }

    fn next_seq(&self) -> u64 {
        self.base.saturating_add(self.slots.len() as u64)
    }

    pub(crate) fn push_back(&mut self, e: Event) {
        let seq = self.next_seq();
        if let Some(lane) = self.lanes.lane(kind_of(&e)) {
            lane.push_back(seq);
        }
        if matches!(e, Event::Discontinuity) {
            self.last_discontinuity = Some(seq);
        }
        self.slots.push_back(Some(e));
        self.live = self.live.saturating_add(1);
        while self.live > self.cap {
            let Some(victim) = self.lanes.next_victim() else {
                break;
            };
            self.drop_event(victim);
        }
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
                    if let Some(lane) = self.lanes.lane(kind_of(&e))
                        && lane.front() == Some(&seq)
                    {
                        lane.pop_front();
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
            if let Some(lane) = self.lanes.lane(kind_of(e)) {
                lane.push_back(seq);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{BufferEvent, ErrorKind, Status};
    use broadcast_common::Unpackage;
    use transmux::{JoinDecision, RapKind, Sample, TrackId};

    const TRACK: TrackId = TrackId::new(1);
    const BURST: i64 = 100;
    const TINY_CAP: usize = 4;

    fn base_sample() -> Sample {
        static BASE: std::sync::OnceLock<Sample> = std::sync::OnceLock::new();
        BASE.get_or_init(|| {
            let data = test_fixtures::read("ts/h264_aac.ts");
            let media = transmux::TsDemux::new().unpackage(&data[..]).unwrap();
            media.tracks[0].samples[0].clone()
        })
        .clone()
    }

    fn sample_event(n: i64, join: Option<JoinDecision>) -> Event {
        let mut sample = base_sample();
        sample.pts = Some(n);
        Event::Sample {
            track: TRACK,
            sample,
            join,
        }
    }

    fn pts_of(e: &Event) -> Option<i64> {
        match e {
            Event::Sample { sample, .. } => sample.pts,
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
            q.push_back(Event::Status(Status::Stalled { secs: n }));
        }
        assert!(q.len() <= TINY_CAP, "len {}", q.len());
        let events = drain(&mut q);
        assert_eq!(
            events.last(),
            Some(&Event::Status(Status::Stalled { secs: 9_999 })),
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
}
