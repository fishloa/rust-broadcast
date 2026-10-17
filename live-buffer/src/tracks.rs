//! Per-track media ends and the effective (minimum) media end.
//!
//! A TS mux sends video well ahead of its play time and audio close to real
//! time, so the amount of media that can actually be played is bounded by the
//! *least advanced* rendered track, not by video alone. The set keeps the end
//! of the last delivered sample per track and reduces them to one position:
//!
//! * a track becomes **active** on its first delivery;
//! * a track announced with [`BufferController::register_track`](crate::BufferController::register_track) that has not delivered yet
//!   **blocks** the reduction (no effective end) until it delivers or has been
//!   silent for `silence_timeout_us`;
//! * a track removed with
//!   [`BufferController::unregister_track`](crate::BufferController::unregister_track)
//!   (a replaced or dead track) leaves the reduction at once; if no track is
//!   left there is no effective end until one delivers;
//! * a track is **excluded** when it is silent *relative to the other tracks*:
//!   its own last delivery (or, if it never delivered, its registration) lies
//!   at least `silence_timeout_us` before the most recent delivery on ANY
//!   track. A dead track therefore cannot starve playback while the others
//!   keep delivering, but the clock is the arrival time of the latest delivery,
//!   not wall time: when nothing at all is delivered (a global stall such as a
//!   reconnect or server restart) nobody becomes excluded, however long the
//!   gap, and the stall is not mistaken for the tracks being dead. Before any
//!   delivery (or after a rebaseline) nothing is excluded either;
//! * the effective end is the earliest end over the non-excluded tracks, so a
//!   global stall keeps the MIN of the tracks' last ends (there is no
//!   "all silent" fallback: the most recently delivering track is never
//!   excluded).
//!
//! **Delivery granularity.** Each track also remembers the largest *step* its
//! end advanced between two distinct delivery instants (a burst of several
//! calls at one arrival time is one step), as a max-hold with a hold and a
//! linear decay ([`MaxHold`], bounded memory: a few scalars per track,
//! at most [`MAX_TRACKS`] tracks). A step counts only when it looks like a
//! delivery cadence: it advanced forward, the instants are less than the
//! silence timeout apart, it is no more than the arrival interval plus
//! `nodata_after_us` (a catch-up burst or a forward jump is not a cadence;
//! the PDV and the stall memory cover those), and the
//! caller reports no global data gap (a chunk gap of `nodata_after_us` or more)
//! inside the interval (a gap is PDV, which the stall memory already covers).
//! The policy keeps the target at least the largest held step plus the margin.
//!
//! All position comparisons are wrap-safe (`broadcast_common::clock33`).

use crate::config::{Micros, TICKS_PER_US, Ticks27};
use crate::stall_memory::MaxHold;
use alloc::vec::Vec;
use broadcast_common::clock33::pcr_signed_distance;

/// Most tracks tracked at once; deliveries for further tracks are ignored.
pub const MAX_TRACKS: usize = 16;

/// The track `BufferController::on_media` reports as.
pub const DEFAULT_TRACK: u32 = 0;

/// Wrap-safe: is `a` strictly before `b`?
fn before(a: Ticks27, b: Ticks27) -> bool {
    pcr_signed_distance(a, b) > 0
}

/// Delivery-instant bookkeeping for the granularity measurement.
#[derive(Clone, Copy, Debug)]
struct Step {
    /// End as of the previous distinct delivery instant, and that instant.
    prev_end: Ticks27,
    prev_at: Micros,
    /// End as of the current delivery instant, and that instant.
    cur_end: Ticks27,
    cur_at: Micros,
}

/// Rules deciding whether an end advance is a delivery step.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StepRules {
    /// Instants this far apart or more are not a cadence (the silence timeout).
    pub(crate) max_interval_us: Micros,
    /// Allowance of media advance over the arrival interval (`nodata_after_us`): more is a catch-up or a jump, not a cadence.
    pub(crate) max_excess_us: Micros,
    /// Arrival time of the chunk that ended the latest global data gap.
    pub(crate) gap_end_at: Option<Micros>,
}

#[derive(Clone, Debug)]
struct Track {
    id: u32,
    registered: bool,
    end: Option<Ticks27>,
    /// Arrival time of the last delivery, or of the registration while none.
    seen_at: Micros,
    step: Option<Step>,
    /// Largest delivery step (microseconds) with hold and decay.
    gran: MaxHold,
}

/// The reduction result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Effective {
    pub(crate) end: Ticks27,
    /// The track that bounds `end`.
    pub(crate) limiting: u32,
}

impl Track {
    /// Fold one delivery into the granularity memory.
    fn observe_step(&mut self, now: Micros, end: Ticks27, rules: StepRules) {
        let Some(s) = self.step.as_mut() else {
            self.step = Some(Step {
                prev_end: end,
                prev_at: now,
                cur_end: end,
                cur_at: now,
            });
            return;
        };
        if now > s.cur_at {
            s.prev_end = s.cur_end;
            s.prev_at = s.cur_at;
        }
        s.cur_end = end;
        s.cur_at = now;
        let advance = pcr_signed_distance(s.prev_end, end);
        let interval = now.saturating_sub(s.prev_at);
        if advance <= 0 || interval == 0 || interval >= rules.max_interval_us {
            return;
        }
        if rules.gap_end_at.is_some_and(|g| g > s.prev_at) {
            return;
        }
        let step_us = u64::try_from(advance).unwrap_or(0) / TICKS_PER_US;
        if step_us > interval.saturating_add(rules.max_excess_us) {
            return;
        }
        self.gran.observe(now, step_us);
    }
}

/// Per-track ends.
#[derive(Clone, Debug)]
pub(crate) struct TrackSet {
    v: Vec<Track>,
    gran_hold_us: Micros,
    gran_decay_us: Micros,
    /// Arrival time of the most recent delivery on any track; the clock the
    /// silence rule measures against. `None` until a delivery.
    last_delivery: Option<Micros>,
}

impl TrackSet {
    pub(crate) fn new(gran_hold_us: Micros, gran_decay_us: Micros) -> Self {
        Self {
            v: Vec::new(),
            gran_hold_us,
            gran_decay_us,
            last_delivery: None,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.v.len()
    }

    pub(crate) fn clear(&mut self) {
        self.v.clear();
        self.last_delivery = None;
    }

    fn slot(&mut self, id: u32, now: Micros) -> Option<&mut Track> {
        let i = match self.v.iter().position(|t| t.id == id) {
            Some(i) => i,
            None => {
                if self.v.len() >= MAX_TRACKS {
                    return None;
                }
                self.v.push(Track {
                    id,
                    registered: false,
                    end: None,
                    seen_at: now,
                    step: None,
                    gran: MaxHold::new(self.gran_hold_us, self.gran_decay_us),
                });
                self.v.len() - 1
            }
        };
        self.v.get_mut(i)
    }

    pub(crate) fn register(&mut self, id: u32, now: Micros) {
        if let Some(t) = self.slot(id, now) {
            t.registered = true;
        }
    }

    /// Forget `id` entirely (registration and end). Unknown id: no-op.
    pub(crate) fn unregister(&mut self, id: u32) {
        self.v.retain(|t| t.id != id);
    }

    /// Record a delivery. `None` when the track cap was hit; otherwise whether
    /// this was the track's first delivery since the last (re)start.
    pub(crate) fn deliver(
        &mut self,
        id: u32,
        now: Micros,
        end: Ticks27,
        rules: StepRules,
    ) -> Option<bool> {
        let t = self.slot(id, now)?;
        let first = t.end.is_none();
        t.end = Some(end);
        t.seen_at = t.seen_at.max(now);
        if first {
            t.seen_at = now;
            t.step = None;
        }
        t.observe_step(now, end, rules);
        self.last_delivery = Some(self.last_delivery.map_or(now, |l| l.max(now)));
        Some(first)
    }

    /// Largest held delivery step over the non-excluded tracks at `now`.
    pub(crate) fn granularity_us(&self, now: Micros, silence_us: Micros) -> Micros {
        self.v
            .iter()
            .filter(|t| {
                !self
                    .last_delivery
                    .is_some_and(|l| l.saturating_sub(t.seen_at) >= silence_us)
            })
            .map(|t| t.gran.value(now))
            .max()
            .unwrap_or(0)
    }

    /// Forget every end (a discontinuity); registrations survive and wait
    /// again from `now`.
    pub(crate) fn rebaseline(&mut self, now: Micros) {
        self.v.retain(|t| t.registered);
        self.last_delivery = None;
        for t in &mut self.v {
            t.end = None;
            t.step = None;
            t.seen_at = now;
        }
    }

    /// Earliest end over the non-excluded tracks (see the module docs for the
    /// relative-silence rule). `None` while a non-excluded track has no end.
    pub(crate) fn effective(&self, silence_us: Micros) -> Option<Effective> {
        let silent = |t: &Track| {
            self.last_delivery
                .is_some_and(|l| l.saturating_sub(t.seen_at) >= silence_us)
        };
        let mut min: Option<Effective> = None;
        for t in self.v.iter().filter(|t| !silent(t)) {
            let end = t.end?;
            if min.is_none_or(|m| before(end, m.end)) {
                min = Some(Effective {
                    end,
                    limiting: t.id,
                });
            }
        }
        min
    }
}
