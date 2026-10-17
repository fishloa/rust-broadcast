//! Per-track media ends and the effective (minimum) media end.
//!
//! A TS mux sends video well ahead of its play time and audio close to real
//! time, so the amount of media that can actually be played is bounded by the
//! *least advanced* rendered track, not by video alone. The set keeps the end
//! of the last delivered sample per track and reduces them to one position:
//!
//! * a track becomes **active** on its first delivery;
//! * a track announced with [`TrackSet::register`] that has not delivered yet
//!   **blocks** the reduction (no effective end) until it delivers or has been
//!   silent for `silence_timeout_us`;
//! * a track silent for `silence_timeout_us` is **excluded**, so a dead track
//!   cannot starve playback forever;
//! * the effective end is the earliest end over the non-excluded tracks; when
//!   every track is excluded it falls back to the latest known end.
//!
//! All position comparisons are wrap-safe (`broadcast_common::clock33`).

use crate::config::{Micros, Ticks27};
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

#[derive(Clone, Copy, Debug)]
struct Track {
    id: u32,
    registered: bool,
    end: Option<Ticks27>,
    /// Arrival time of the last delivery, or of the registration while none.
    seen_at: Micros,
}

/// The reduction result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Effective {
    pub(crate) end: Ticks27,
    /// The track that bounds `end`.
    pub(crate) limiting: u32,
}

/// Per-track ends.
#[derive(Clone, Debug, Default)]
pub(crate) struct TrackSet {
    v: Vec<Track>,
}

impl TrackSet {
    pub(crate) fn len(&self) -> usize {
        self.v.len()
    }

    pub(crate) fn clear(&mut self) {
        self.v.clear();
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

    /// Record a delivery. `None` when the track cap was hit; otherwise whether
    /// this was the track's first delivery since the last (re)start.
    pub(crate) fn deliver(&mut self, id: u32, now: Micros, end: Ticks27) -> Option<bool> {
        let t = self.slot(id, now)?;
        let first = t.end.is_none();
        t.end = Some(end);
        t.seen_at = t.seen_at.max(now);
        if first {
            t.seen_at = now;
        }
        Some(first)
    }

    /// Forget every end (a discontinuity); registrations survive and wait
    /// again from `now`.
    pub(crate) fn rebaseline(&mut self, now: Micros) {
        self.v.retain(|t| t.registered);
        for t in &mut self.v {
            t.end = None;
            t.seen_at = now;
        }
    }

    pub(crate) fn effective(&self, now: Micros, silence_us: Micros) -> Option<Effective> {
        let silent = |t: &Track| now.saturating_sub(t.seen_at) >= silence_us;
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
        if min.is_some() {
            return min;
        }
        let mut max: Option<Effective> = None;
        for t in &self.v {
            if let Some(end) = t.end {
                if max.is_none_or(|m| before(m.end, end)) {
                    max = Some(Effective {
                        end,
                        limiting: t.id,
                    });
                }
            }
        }
        max
    }
}
