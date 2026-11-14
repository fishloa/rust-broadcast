//! Controller states, hints, events and statistics.
//!
//! Every enum has a stable `u32` code (`code()`), used by the C-ABI mirror;
//! codes are never renumbered. [`Event`] converts to the `#[repr(C)]`
//! [`EventRecord`] with [`Event::to_record`]; the `a` / `b` meaning per
//! kind is the table on [`EventRecord`].

use core::time::Duration;

use crate::jitter::JitterEstimate;
use crate::time::{Clock27, duration_micros};
use alloc::collections::VecDeque;

/// Event queue capacity (oldest non-critical event dropped first).
pub(crate) const EVENT_QUEUE_CAP: usize = 16;

/// Controller state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum State {
    /// Filling: first fill, or refilling after a discontinuity.
    Buffering,
    /// Rendering; the caller's clock runs at rate 1.
    Playing,
    /// Paused for an underrun or a voluntary re-prime, refilling.
    Starved,
}

impl State {
    /// Stable label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Buffering => "buffering",
            Self::Playing => "playing",
            Self::Starved => "starved",
        }
    }
    /// Stable ABI code.
    #[must_use]
    pub fn code(&self) -> u32 {
        match self {
            Self::Buffering => 1,
            Self::Playing => 2,
            Self::Starved => 3,
        }
    }
}
broadcast_common::impl_spec_display!(State);

/// Why the controller is not playing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Reason {
    /// First fill after construction/reset.
    Startup,
    /// Lead exhausted and no chunk for `nodata_after` (or the source ended).
    NoData,
    /// Lead exhausted while chunks were still arriving.
    DataArriving,
    /// Baseline reset (reconnect, sync loss, PCR discontinuity).
    Discontinuity,
    /// Voluntary re-prime (`reprime_floor`).
    Reprime,
}

impl Reason {
    /// Stable label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::NoData => "no data",
            Self::DataArriving => "data arriving",
            Self::Discontinuity => "discontinuity",
            Self::Reprime => "reprime",
        }
    }
    /// Stable ABI code.
    #[must_use]
    pub fn code(&self) -> u32 {
        match self {
            Self::Startup => 1,
            Self::NoData => 2,
            Self::DataArriving => 3,
            Self::Discontinuity => 4,
            Self::Reprime => 5,
        }
    }
}
broadcast_common::impl_spec_display!(Reason);

/// Out-of-band input the caller reports. Non-authoritative except where
/// stated: starvation is only ever declared from lead and the clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Hint {
    /// Continuity errors / TEI seen: recorded in `Stats::input_degraded` only.
    InputDegraded,
    /// The degradation ended.
    InputRecovered,
    /// Sync loss: re-baseline.
    SyncLoss,
    /// The caller saw a PCR discontinuity: re-baseline.
    PcrDiscontinuity,
    /// The connection ended: later pauses report `NoData`.
    SourceEnded,
    /// The connection failed: later pauses report `NoData`.
    SourceError,
    /// First chunk of a new connection follows (send BEFORE feeding it):
    /// re-baseline, keep the stall memory, resume at the target.
    Reconnected,
}

impl Hint {
    /// Stable label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::InputDegraded => "input degraded",
            Self::InputRecovered => "input recovered",
            Self::SyncLoss => "sync loss",
            Self::PcrDiscontinuity => "pcr discontinuity",
            Self::SourceEnded => "source ended",
            Self::SourceError => "source error",
            Self::Reconnected => "reconnected",
        }
    }
    /// Stable ABI code (`br_on_hint`).
    #[must_use]
    pub fn code(&self) -> u32 {
        match self {
            Self::InputDegraded => 1,
            Self::InputRecovered => 2,
            Self::SyncLoss => 3,
            Self::PcrDiscontinuity => 4,
            Self::SourceEnded => 5,
            Self::SourceError => 6,
            Self::Reconnected => 7,
        }
    }
    /// Inverse of [`code`](Self::code); `None` for an unknown code.
    #[must_use]
    pub fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            1 => Self::InputDegraded,
            2 => Self::InputRecovered,
            3 => Self::SyncLoss,
            4 => Self::PcrDiscontinuity,
            5 => Self::SourceEnded,
            6 => Self::SourceError,
            7 => Self::Reconnected,
            _ => return None,
        })
    }
}
broadcast_common::impl_spec_display!(Hint);

/// A decision for the caller. Media positions are in the **playback (PTS)
/// domain** as 27 MHz ticks (`PTS90k x 300`), i.e. the domain of `on_media`
/// and `on_tick`'s `playback`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Event {
    /// First fill complete. Set the renderer clock to `media_27mhz` (the
    /// oldest buffered media) and rate 1.
    StartPlaying {
        /// Position to start from.
        media_27mhz: Clock27,
        /// Lead at the decision.
        lead: Duration,
    },
    /// Underrun or re-prime. Set the renderer rate to 0 and hold the clock at
    /// `hold_at_27mhz`; keep queueing every chunk that arrives meanwhile.
    Pause {
        /// Position to freeze at (never ahead of the data).
        hold_at_27mhz: Clock27,
        /// Why.
        reason: Reason,
        /// Buffered lead at the pause (0 if the clock overran the data).
        depth: Duration,
        /// Resume target known now; it can rise when the first post-stall
        /// chunk reveals the stall depth.
        resume_target: Duration,
        /// For an underrun or re-prime with two or more tracks known: the
        /// track whose media end bounded the lead (the starving track).
        /// `None` otherwise (single track, discontinuity).
        track: Option<u32>,
    },
    /// Refilled. Re-seed the renderer clock to exactly `media_27mhz` and set
    /// rate 1 in one step.
    ///
    /// **Resume is also a "go".** After a `Hint::PcrDiscontinuity` (or
    /// `SyncLoss`/`Reconnected`) while `Playing` the controller emits `Pause`
    /// then, once refilled, `Resume`, never a second `StartPlaying`
    /// (`StartPlaying` happens once per controller until `reset`). A caller
    /// must therefore treat `StartPlaying` and `Resume` identically as "set the
    /// clock to `media_27mhz` and rate 1". If that differs from the clock at pause (after a
    /// discontinuity) it is a time jump on the caller's side: re-enqueue
    /// from there if the audio renderer flushes.
    Resume {
        /// Position to restart from.
        media_27mhz: Clock27,
        /// Lead at the decision.
        lead: Duration,
    },
    /// The target moved by at least one histogram bucket (or hit a bound).
    TargetChanged {
        /// Previously reported target.
        old: Duration,
        /// New target (never above `max`).
        new: Duration,
    },
    /// The estimated need exceeds the configured `max`; the target is clamped
    /// (the cap is a guarantee). Raised once per excursion.
    OverMax {
        /// Unclamped need.
        need: Duration,
        /// The cap.
        max: Duration,
    },
    /// The baseline was reset (discontinuity, reconnect, sync loss).
    Rebaselined,
    /// Events were lost because the caller did not poll in time.
    EventsDropped {
        /// How many.
        count: u32,
    },
}

/// Event kind codes (stable).
pub mod kind {
    /// [`Event::StartPlaying`](super::Event::StartPlaying)
    pub const START_PLAYING: u32 = 1;
    /// [`Event::Pause`](super::Event::Pause)
    pub const PAUSE: u32 = 2;
    /// [`Event::Resume`](super::Event::Resume)
    pub const RESUME: u32 = 3;
    /// [`Event::TargetChanged`](super::Event::TargetChanged)
    pub const TARGET_CHANGED: u32 = 4;
    /// [`Event::OverMax`](super::Event::OverMax)
    pub const OVER_MAX: u32 = 5;
    /// [`Event::Rebaselined`](super::Event::Rebaselined)
    pub const REBASELINED: u32 = 6;
    /// [`Event::EventsDropped`](super::Event::EventsDropped)
    pub const EVENTS_DROPPED: u32 = 7;
}

/// Plain-data, `#[repr(C)]` form of an [`Event`] for a C ABI.
///
/// | kind | `reason` | `media_27mhz` | `a` | `b` |
/// |---|---|---|---|---|
/// | 1 StartPlaying | 0 | start position | lead, µs | 0 |
/// | 2 Pause | [`Reason::code`] | hold position | depth, µs | resume target, µs |
/// | 3 Resume | 0 | restart position | lead, µs | 0 |
/// | 4 TargetChanged | 0 | 0 | old target, µs | new target, µs |
/// | 5 OverMax | 0 | 0 | need, µs | max, µs |
/// | 6 Rebaselined | 0 | 0 | 0 | 0 |
/// | 7 EventsDropped | 0 | 0 | count | 0 |
///
/// `a`/`b` are whole microseconds: the `#[repr(C)]` record is an
/// integer-only ABI, so a `Duration` crosses it by its microsecond value.
///
/// `track_valid` is 1 only for a `Pause` that names the starving track in
/// `track` (two or more tracks known); otherwise both are 0. Layout (ABI
/// version 2): 40 bytes, `kind@0 reason@4 media_27mhz@8 a@16 b@24
/// track@32 track_valid@36`.
///
/// There is no separate "Starved" event: a starvation is a `Pause` whose
/// `reason` is 2/3 and whose `a` is the lead at that instant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(C)]
#[non_exhaustive]
pub struct EventRecord {
    /// Event kind code (see [`kind`]).
    pub kind: u32,
    /// Pause reason code, else 0.
    pub reason: u32,
    /// Media position (PTS domain, 27 MHz), else 0.
    pub media_27mhz: u64,
    /// First value in whole microseconds (table above).
    pub a: u64,
    /// Second value in whole microseconds (table above).
    pub b: u64,
    /// The starving track when `track_valid == 1`, else 0.
    pub track: u32,
    /// 1 when `track` is meaningful, else 0.
    pub track_valid: u32,
}

impl Event {
    /// Convert to the plain-data record.
    #[must_use]
    pub fn to_record(&self) -> EventRecord {
        let mut r = EventRecord::default();
        match *self {
            Self::StartPlaying { media_27mhz, lead } => {
                r.kind = kind::START_PLAYING;
                r.media_27mhz = media_27mhz.ticks();
                r.a = duration_micros(lead);
            }
            Self::Pause {
                hold_at_27mhz,
                reason,
                depth,
                resume_target,
                track,
            } => {
                r.kind = kind::PAUSE;
                if let Some(t) = track {
                    r.track = t;
                    r.track_valid = 1;
                }
                r.reason = reason.code();
                r.media_27mhz = hold_at_27mhz.ticks();
                r.a = duration_micros(depth);
                r.b = duration_micros(resume_target);
            }
            Self::Resume { media_27mhz, lead } => {
                r.kind = kind::RESUME;
                r.media_27mhz = media_27mhz.ticks();
                r.a = duration_micros(lead);
            }
            Self::TargetChanged { old, new } => {
                r.kind = kind::TARGET_CHANGED;
                r.a = duration_micros(old);
                r.b = duration_micros(new);
            }
            Self::OverMax { need, max } => {
                r.kind = kind::OVER_MAX;
                r.a = duration_micros(need);
                r.b = duration_micros(max);
            }
            Self::Rebaselined => r.kind = kind::REBASELINED,
            Self::EventsDropped { count } => {
                r.kind = kind::EVENTS_DROPPED;
                r.a = u64::from(count);
            }
        }
        r
    }

    /// Stable label of the variant.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::StartPlaying { .. } => "start playing",
            Self::Pause { .. } => "pause",
            Self::Resume { .. } => "resume",
            Self::TargetChanged { .. } => "target changed",
            Self::OverMax { .. } => "over max",
            Self::Rebaselined => "rebaselined",
            Self::EventsDropped { .. } => "events dropped",
        }
    }

    /// Pause/Resume/StartPlaying must never be lost.
    fn is_critical(&self) -> bool {
        matches!(
            self,
            Self::StartPlaying { .. } | Self::Pause { .. } | Self::Resume { .. }
        )
    }
}
broadcast_common::impl_spec_display!(Event);

/// Controller statistics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Stats {
    /// Current state.
    pub state: State,
    /// Reason of the most recent non-playing period (`Startup` initially);
    /// meaningful only while `state != Playing`.
    pub reason: crate::events::Reason,
    /// Signed lead at the last tick (negative: the clock ran ahead of the media).
    pub lead: Duration,
    /// Current target.
    pub target: Duration,
    /// Underrun pauses (not the first fill, discontinuities or re-primes).
    pub stalls: u32,
    /// Voluntary re-primes.
    pub reprimes: u32,
    /// Stall-memory peak when the last stall ended.
    pub last_stall_depth: Duration,
    /// Longest pause (Pause to Resume).
    pub longest_freeze: Duration,
    /// Estimator outputs.
    pub est: JitterEstimate,
    /// Arrival/tick clock regressions seen (clamped or ignored).
    pub clock_regressions: u32,
    /// Baseline resets.
    pub discontinuities: u32,
    /// Hints received.
    pub hints_seen: u32,
    /// Events lost to queue overflow.
    pub events_dropped: u32,
    /// Between `InputDegraded` and `InputRecovered`.
    pub input_degraded: bool,
    /// Minimum-window entries dropped by the cap.
    pub window_overflows: u32,
    /// With two or more tracks known: the track currently bounding the media
    /// end (the one that would starve first); `None` otherwise.
    pub limiting_track: Option<u32>,
}

/// Bounded event queue: when full, the oldest non-critical event is dropped
/// (the oldest of all if every queued event is critical) and counted; the
/// loss is reported by one `EventsDropped` ahead of the survivors.
#[derive(Clone, Debug, Default)]
pub(crate) struct EventQueue {
    q: VecDeque<Event>,
    dropped_pending: u32,
    dropped_total: u32,
}

impl EventQueue {
    pub(crate) fn new() -> Self {
        Self {
            q: VecDeque::with_capacity(EVENT_QUEUE_CAP + 1),
            dropped_pending: 0,
            dropped_total: 0,
        }
    }

    pub(crate) fn push(&mut self, e: Event) {
        if self.q.len() >= EVENT_QUEUE_CAP {
            let victim = self.q.iter().position(|x| !x.is_critical());
            match victim {
                Some(i) => {
                    self.q.remove(i);
                }
                None => {
                    self.q.pop_front();
                }
            }
            self.dropped_pending = self.dropped_pending.saturating_add(1);
            self.dropped_total = self.dropped_total.saturating_add(1);
        }
        self.q.push_back(e);
    }

    pub(crate) fn poll(&mut self) -> Option<Event> {
        if self.dropped_pending > 0 {
            let count = core::mem::take(&mut self.dropped_pending);
            return Some(Event::EventsDropped { count });
        }
        self.q.pop_front()
    }

    pub(crate) fn dropped_total(&self) -> u32 {
        self.dropped_total
    }

    pub(crate) fn clear(&mut self) {
        self.q.clear();
        self.dropped_pending = 0;
    }
}
