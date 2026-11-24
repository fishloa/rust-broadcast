//! Read-ahead backpressure for the TS-over-HTTP source (issue #4085).
//!
//! A sans-IO session cannot stop a socket; the host can. This tracker measures
//! how far the media delivered so far runs ahead of the player, in media time
//! and in bytes accepted since the playback position last advanced, and reports
//! the two transitions a host uses to stop and restart reading one connection.

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

fn limit_ticks(limit: Duration) -> u64 {
    let micros = limit.as_micros().min(u128::from(u64::MAX));
    u64::try_from(micros).map_or(u64::MAX, |micros| micros.saturating_mul(TICKS_PER_MICRO))
}

/// The read-ahead measure and its hysteresis state.
pub(crate) struct ReadAhead {
    /// Time cap in 27 MHz ticks; `None` leaves the time measure uncapped.
    max_time: Option<u64>,
    /// Byte cap on bytes accepted since the playback position last changed.
    max_bytes: Option<u64>,
    /// Shift that moves a negative first PTS onto the clock, as the playout gate does.
    origin: Option<i128>,
    /// First delivered presentation time, used until a playback position is known.
    first_media: Option<u64>,
    /// Latest delivered presentation end.
    latest_media: Option<u64>,
    /// Last playback position, in 27 MHz ticks.
    position: Option<u64>,
    /// Bytes accepted since the playback position last changed.
    bytes: u64,
    paused: bool,
}

impl ReadAhead {
    pub(crate) fn new(max_time: Option<Duration>, max_bytes: Option<usize>) -> Self {
        Self {
            max_time: max_time.map(limit_ticks),
            max_bytes: max_bytes.map(|bytes| bytes as u64),
            origin: None,
            first_media: None,
            latest_media: None,
            position: None,
            bytes: 0,
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

    /// Bytes the session accepted. They count whether or not reads are paused.
    pub(crate) fn on_bytes(&mut self, len: usize) {
        if self.enabled() {
            self.bytes = self.bytes.saturating_add(len as u64);
        }
    }

    /// The player's latest position. Only a change starts a new byte window, so
    /// a host that reports the same position every tick does not reset the byte
    /// measure under a fast source.
    pub(crate) fn on_position(&mut self, position: Option<u64>) {
        if !self.enabled() {
            return;
        }
        let Some(position) = position else {
            return;
        };
        if self.position != Some(position) {
            self.position = Some(position);
            self.bytes = 0;
        }
    }

    /// A delivered sample: `pts` and `end` are presentation start and end in
    /// the track's timescale.
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
        let bytes = self.max_bytes.is_some_and(|cap| self.bytes >= cap);
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
            .is_none_or(|cap| self.bytes < cap / u64::from(RESUME_DIVISOR));
        time && bytes
    }

    /// Start over for a new connection: drop the media baseline and the byte
    /// window, but keep the last playback position (the player still holds it).
    /// The pause itself is cleared by the caller without a `ResumeRead`.
    pub(crate) fn reset(&mut self) {
        self.origin = None;
        self.first_media = None;
        self.latest_media = None;
        self.bytes = 0;
        self.paused = false;
    }

    /// Re-baseline the media measure after a discontinuity on the same
    /// connection. The pause, byte window and playback position survive, so a
    /// backward jump can resume and a forward one keeps its pause.
    pub(crate) fn reset_timeline(&mut self) {
        self.origin = None;
        self.first_media = None;
        self.latest_media = None;
    }
}
