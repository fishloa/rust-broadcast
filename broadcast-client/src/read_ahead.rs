//! Read-ahead backpressure for the TS-over-HTTP source (issue #4085).
//!
//! A sans-IO session cannot stop a socket; the host can. This tracker measures
//! how far the media delivered so far runs ahead of the player, in media time
//! and in bytes of media still ahead of the playback position, and reports the
//! two transitions a host uses to stop and restart reading one connection.

use alloc::collections::VecDeque;
use core::time::Duration;

use broadcast_common::clock33::{PCR_HZ, pcr_signed_distance};
use broadcast_common::timescale::Timescale;
use broadcast_common::units::MICROS_PER_SEC;

use crate::buffer::{negative_origin, pts_to_27mhz_from, raw_27mhz};

/// 27 MHz ticks in one microsecond.
const TICKS_PER_MICRO: u64 = PCR_HZ / MICROS_PER_SEC;
/// Resume once a measure falls under `cap / RESUME_DIVISOR`; half the cap keeps
/// a pause and a resume from flapping around the cap.
const RESUME_DIVISOR: u32 = 2;
/// Presentation time one queued byte entry covers. Consecutive samples in one
/// slice share an entry, so the queue length is the media-time window divided by
/// this slice, never the sample or file count.
const READ_AHEAD_SLICE: Duration = Duration::from_millis(100);
/// Absolute ceiling on queued slices, about 400 s of media at the slice above.
/// A configured time cap normally shrinks the queue far below this; the ceiling
/// is what keeps a byte-only cap's queue bounded too.
const MAX_READ_AHEAD_SLICES: usize = 4096;
/// Smallest useful queue: a cap shorter than two slices still keeps one entry
/// per boundary plus the merge target.
const MIN_READ_AHEAD_SLICES: usize = 2;

/// One presentation-time slice of delivered samples still ahead of the player.
struct Slice {
    end: u64,
    bytes: u64,
}

fn limit_ticks(limit: Duration) -> u64 {
    let micros = limit.as_micros().min(u128::from(u64::MAX));
    u64::try_from(micros).map_or(u64::MAX, |micros| micros.saturating_mul(TICKS_PER_MICRO))
}

/// The read-ahead measure and its hysteresis state.
pub(crate) struct ReadAhead {
    /// Time cap in 27 MHz ticks; `None` leaves the time measure uncapped.
    max_time: Option<u64>,
    /// Byte cap on bytes still ahead of the playback position.
    max_bytes: Option<u64>,
    /// Presentation-time width of one queued entry, in 27 MHz ticks.
    slice_ticks: u64,
    /// Most entries the queue may hold.
    max_slices: usize,
    /// Shift that moves a negative first PTS onto the clock, as the playout gate does.
    origin: Option<i128>,
    /// First delivered presentation time, used until a playback position is known.
    first_media: Option<u64>,
    /// Latest delivered presentation end.
    latest_media: Option<u64>,
    /// Last playback position, in 27 MHz ticks.
    position: Option<u64>,
    /// Accepted bytes not yet attributed to a delivered sample.
    pending: u64,
    /// Delivered sample bytes still ahead of `position`, oldest end first.
    ahead: VecDeque<Slice>,
    /// `pending` plus the bytes in [`Self::ahead`].
    bytes_ahead: u64,
    paused: bool,
}

impl ReadAhead {
    pub(crate) fn new(max_time: Option<Duration>, max_bytes: Option<usize>) -> Self {
        let slice_ticks = limit_ticks(READ_AHEAD_SLICE);
        let max_slices = match max_time {
            Some(cap) => usize::try_from(limit_ticks(cap) / slice_ticks)
                .unwrap_or(MAX_READ_AHEAD_SLICES)
                .saturating_add(1)
                .clamp(MIN_READ_AHEAD_SLICES, MAX_READ_AHEAD_SLICES),
            None => MAX_READ_AHEAD_SLICES,
        };
        Self {
            max_time: max_time.map(limit_ticks),
            max_bytes: max_bytes.map(|bytes| bytes as u64),
            slice_ticks,
            max_slices,
            origin: None,
            first_media: None,
            latest_media: None,
            position: None,
            pending: 0,
            ahead: VecDeque::new(),
            bytes_ahead: 0,
            paused: false,
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.max_time.is_some() || self.max_bytes.is_some()
    }

    pub(crate) fn is_paused(&self) -> bool {
        self.paused
    }

    pub(crate) fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Bytes the session accepted. They count at once, so a stream that never
    /// produces a timed sample is still held; bytes attributed to a delivered
    /// sample move into the presentation-time queue.
    pub(crate) fn on_bytes(&mut self, len: usize) {
        if self.enabled() {
            let len = len as u64;
            self.pending = self.pending.saturating_add(len);
            self.bytes_ahead = self.bytes_ahead.saturating_add(len);
        }
    }

    /// The player's latest position. Moving it drops every queued entry whose
    /// presentation end is at or behind it; an unchanged position drops
    /// nothing. Before the first position is reported nothing is dropped.
    pub(crate) fn on_position(&mut self, position: Option<u64>) {
        if !self.enabled() {
            return;
        }
        let Some(position) = position else {
            return;
        };
        if self.position == Some(position) {
            return;
        }
        self.position = Some(position);
        while let Some(front) = self.ahead.front() {
            if pcr_signed_distance(position, front.end) > 0 {
                break;
            }
            let bytes = front.bytes;
            self.ahead.pop_front();
            self.bytes_ahead = self.bytes_ahead.saturating_sub(bytes);
        }
    }

    /// A delivered sample: `pts` and `end` are presentation start and end in
    /// the track's timescale. Every byte accepted since the previous sample is
    /// attributed to this one, container overhead included: the demuxer has no
    /// finer mapping, and it keeps `pending` at most one feed's tail rather
    /// than letting overhead accumulate past the cap.
    pub(crate) fn on_sample(&mut self, pts: i64, end: i64, timescale: Timescale) {
        if !self.enabled() {
            return;
        }
        let raw = raw_27mhz(pts, timescale);
        let origin = match self.origin {
            Some(origin) => origin,
            None => {
                let origin = negative_origin(raw);
                self.origin = Some(origin);
                origin
            }
        };
        let start = pts_to_27mhz_from(pts, timescale, origin);
        let end = pts_to_27mhz_from(end, timescale, origin);
        self.first_media.get_or_insert(start);
        self.latest_media = Some(match self.latest_media {
            Some(latest) if pcr_signed_distance(latest, end) > 0 => end,
            Some(latest) => latest,
            None => end,
        });

        let take = self.pending;
        if take == 0 {
            return;
        }
        self.pending = 0;
        self.push_slice(end, take);
    }

    fn push_slice(&mut self, end: u64, bytes: u64) {
        if let Some(last) = self.ahead.back_mut()
            && end / self.slice_ticks <= last.end / self.slice_ticks
        {
            last.bytes = last.bytes.saturating_add(bytes);
            if pcr_signed_distance(last.end, end) > 0 {
                last.end = end;
            }
            return;
        }
        self.ahead.push_back(Slice { end, bytes });
        self.trim_slices();
    }

    /// Keep the queue within its derived bound by merging the two oldest
    /// entries into one ending at the later of their ends: their bytes stay
    /// counted until the position passes both slices.
    fn trim_slices(&mut self) {
        while self.ahead.len() > self.max_slices {
            let Some(first) = self.ahead.pop_front() else {
                return;
            };
            let Some(second) = self.ahead.front_mut() else {
                self.ahead.push_front(first);
                return;
            };
            second.bytes = second.bytes.saturating_add(first.bytes);
            if pcr_signed_distance(second.end, first.end) > 0 {
                second.end = first.end;
            }
        }
    }

    fn time_lead(&self) -> Option<u64> {
        let latest = self.latest_media?;
        let anchor = self.position.or(self.first_media)?;
        let distance = pcr_signed_distance(anchor, latest);
        Some(if distance > 0 { distance as u64 } else { 0 })
    }

    pub(crate) fn pause_due(&self) -> bool {
        if self.paused || !self.enabled() {
            return false;
        }
        let time = self
            .max_time
            .is_some_and(|cap| self.time_lead().is_some_and(|lead| lead >= cap));
        let bytes = self.max_bytes.is_some_and(|cap| self.bytes_ahead >= cap);
        time || bytes
    }

    pub(crate) fn resume_due(&self) -> bool {
        if !self.paused {
            return false;
        }
        let time = self.max_time.is_none_or(|cap| {
            self.time_lead()
                .is_none_or(|lead| lead < cap / u64::from(RESUME_DIVISOR))
        });
        let bytes = self
            .max_bytes
            .is_none_or(|cap| self.bytes_ahead < cap / u64::from(RESUME_DIVISOR));
        time && bytes
    }

    /// Start over for a new connection: drop the media baseline and every byte
    /// measure, but keep the last playback position (the player still holds
    /// it). The pause itself is cleared by the caller without a `ResumeRead`.
    pub(crate) fn reset(&mut self) {
        self.origin = None;
        self.first_media = None;
        self.latest_media = None;
        self.pending = 0;
        self.ahead.clear();
        self.bytes_ahead = 0;
        self.paused = false;
    }

    /// Re-baseline after a discontinuity on the same connection. The pause and
    /// playback position survive, so a backward jump can resume and a forward
    /// one keeps its pause; the old timeline's bytes no longer describe what is
    /// ahead.
    pub(crate) fn reset_timeline(&mut self) {
        self.origin = None;
        self.first_media = None;
        self.latest_media = None;
        self.pending = 0;
        self.ahead.clear();
        self.bytes_ahead = 0;
    }
}
