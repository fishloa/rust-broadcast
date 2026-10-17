//! Delivery-jitter estimator, usable alone.
//!
//! Sample definition: RFC 5481 s4.2 / s7.1.2 packet delay variation. For each
//! media-time-carrying chunk the relative transit
//! `D = arrival - media` (the constant clock offset cancels) is pushed into a
//! trailing-window minimum; `PDV = D - Dmin`. The RFC 3550 s6.4.1 / A.8
//! interarrival jitter `J` is computed alongside, for reporting only.
//! Everything else (per-interval maxima, forgetting histogram, stall
//! max-hold) is implementation-defined; see the crate docs.

use crate::config::{JitterConfig, Micros, TICKS_PER_US, Ticks27};
use crate::error::Error;
use crate::histogram::ForgettingHistogram;
use crate::stall_memory::{MaxHold, StallMemorySnapshot};
use crate::window::MinWindow;
use broadcast_common::clock33::pcr_signed_distance;

/// What happened to one fed sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SampleOutcome {
    /// Folded into the estimate.
    Accepted,
    /// A media-time jump beyond `discontinuity_us`: the baseline was reset
    /// and this sample starts the new one.
    Discontinuity,
    /// The arrival clock went backwards: the sample is ignored.
    NonMonotonicArrival,
    /// The media time repeats the previous one (a duplicated PCR carries no
    /// new media time): the sample is ignored.
    Duplicate,
}

impl SampleOutcome {
    /// Stable label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Discontinuity => "discontinuity",
            Self::NonMonotonicArrival => "non-monotonic arrival",
            Self::Duplicate => "duplicate",
        }
    }
}
broadcast_common::impl_spec_display!(SampleOutcome);

/// Current estimator outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct JitterEstimate {
    /// Latest `D - Dmin`.
    pub pdv_last_us: Micros,
    /// Histogram quantile of interval maxima (ordinary-jitter upper tail).
    pub baseline_us: Micros,
    /// Max-hold with linear decay (stall memory).
    pub stall_peak_us: Micros,
    /// RFC 3550 A.8 `J`, reporting only.
    pub rfc3550_jitter_us: Micros,
    /// Samples accepted since construction.
    pub samples: u64,
}

#[derive(Clone, Copy, Debug)]
struct Origin {
    arrival_us: u64,
    media_us: i128,
}

/// Burst-aware PDV estimator (sliding-minimum window, per-interval maximum,
/// forgetting histogram, stall max-hold, RFC 3550 `J`).
#[derive(Clone, Debug)]
pub struct DeliveryJitter {
    cfg: JitterConfig,
    window: MinWindow,
    hist: ForgettingHistogram,
    hold: MaxHold,
    origin: Option<Origin>,
    prev_raw: Option<u64>,
    unrolled_ticks: i128,
    last_arrival: Option<u64>,
    istart: Option<u64>,
    imax: Option<u64>,
    recovering: bool,
    base: u64,
    pdv_last: u64,
    j16: i64,
    prev_d: Option<i64>,
    samples: u64,
}

/// RFC 3550 A.8 integer-form scaling: the estimate is kept x16, rounded by 8.
const J_SHIFT: u32 = 4;
const J_ROUND: i64 = 8;

fn sat_i64(v: i128) -> i64 {
    i64::try_from(v).unwrap_or(if v < 0 { i64::MIN } else { i64::MAX })
}

impl DeliveryJitter {
    /// Build an estimator.
    ///
    /// # Errors
    /// Any [`JitterConfig::validate`] failure.
    pub fn new(cfg: JitterConfig) -> Result<Self, Error> {
        cfg.validate()?;
        Ok(Self {
            window: MinWindow::new(cfg.min_window_us, cfg.max_samples)?,
            hist: ForgettingHistogram::new(
                cfg.buckets,
                cfg.bucket_us,
                cfg.forget_q15,
                cfg.quantile_permille,
            ),
            hold: MaxHold::new(cfg.hold_us, cfg.decay_us),
            cfg,
            origin: None,
            prev_raw: None,
            unrolled_ticks: 0,
            last_arrival: None,
            istart: None,
            imax: None,
            recovering: false,
            base: 0,
            pdv_last: 0,
            j16: 0,
            prev_d: None,
            samples: 0,
        })
    }

    /// Feed one media-time-carrying chunk. `media_27mhz` is the raw (wrapped)
    /// 27 MHz media time (the PCR; or the media end in PCR-less mode), unrolled
    /// here with `clock33::pcr_signed_distance`. `_bytes` is reserved (the
    /// estimator is time-based; the C-ABI mirror carries it).
    pub fn on_chunk(
        &mut self,
        arrival_us: Micros,
        media_27mhz: Ticks27,
        _bytes: u32,
    ) -> SampleOutcome {
        if self.last_arrival.is_some_and(|l| arrival_us < l) {
            return SampleOutcome::NonMonotonicArrival;
        }
        if self.prev_raw == Some(media_27mhz) {
            return SampleOutcome::Duplicate;
        }
        self.hold.anchor(arrival_us);
        let mut outcome = SampleOutcome::Accepted;
        if let (Some(prev), Some(last)) = (self.prev_raw, self.last_arrival) {
            let media_delta =
                i128::from(pcr_signed_distance(prev, media_27mhz)) / i128::from(TICKS_PER_US);
            let arrival_delta = i128::from(arrival_us - last);
            let disc = i128::from(self.cfg.discontinuity_us);
            if media_delta < -disc || media_delta - arrival_delta > disc {
                self.reset_baseline();
                outcome = SampleOutcome::Discontinuity;
            }
        }
        match self.prev_raw {
            Some(prev) => {
                self.unrolled_ticks += i128::from(pcr_signed_distance(prev, media_27mhz));
            }
            None => self.unrolled_ticks = 0,
        }
        let media_us = self.unrolled_ticks.div_euclid(i128::from(TICKS_PER_US));
        let origin = *self.origin.get_or_insert(Origin {
            arrival_us,
            media_us,
        });
        self.prev_raw = Some(media_27mhz);
        self.last_arrival = Some(arrival_us);

        let rel_arrival = arrival_us - origin.arrival_us;
        let d = sat_i64(i128::from(rel_arrival) - (media_us - origin.media_us));
        let dmin = self.window.push(rel_arrival, d);
        let pdv = u64::try_from(i128::from(d) - i128::from(dmin)).unwrap_or(0);

        if let Some(pd) = self.prev_d {
            let diff = sat_i64((i128::from(d) - i128::from(pd)).abs());
            // RFC 3550 A.8 integer form: `jitter += d - ((jitter + 8) >> 4)`, reported `>> 4`.
            self.j16 = self
                .j16
                .saturating_add(diff - (self.j16.saturating_add(J_ROUND) >> J_SHIFT));
        }
        self.prev_d = Some(d);

        self.hold.observe(arrival_us, pdv);
        self.fold_interval(arrival_us, pdv);
        self.pdv_last = pdv;
        self.samples = self.samples.saturating_add(1);
        outcome
    }

    fn fold_interval(&mut self, arrival_us: u64, pdv: u64) {
        match self.istart {
            None => self.istart = Some(arrival_us),
            Some(start) if arrival_us >= start.saturating_add(self.cfg.interval_us) => {
                if let Some(m) = self.imax.take() {
                    if m > self.cfg.outlier_us {
                        // a stall interval: max-hold only; its aftermath is not baseline either
                        self.recovering = true;
                    } else if self.recovering {
                        // the interval after a stall holds the tail of a catch-up burst
                        // (D still above the minimum while the backlog drains): skip it once
                        self.recovering = false;
                    } else {
                        self.hist.add(m);
                        self.base = self.hist.quantile_us();
                    }
                }
                let whole = (arrival_us - start) / self.cfg.interval_us;
                self.istart =
                    Some(start.saturating_add(whole.saturating_mul(self.cfg.interval_us)));
            }
            Some(_) => {}
        }
        self.imax = Some(self.imax.map_or(pdv, |m| m.max(pdv)));
    }

    /// Current outputs (stall peak evaluated at the latest arrival).
    #[must_use]
    pub fn estimate(&self) -> JitterEstimate {
        JitterEstimate {
            pdv_last_us: self.pdv_last,
            baseline_us: self.base,
            stall_peak_us: self.stall_peak_at(self.last_arrival.unwrap_or(0)),
            rfc3550_jitter_us: u64::try_from(self.j16 >> J_SHIFT).unwrap_or(0),
            samples: self.samples,
        }
    }

    /// Histogram baseline (ordinary-jitter upper tail).
    #[must_use]
    pub fn baseline_us(&self) -> Micros {
        self.base
    }

    /// Decayed stall-memory value at `now_us`.
    #[must_use]
    pub fn stall_peak_at(&self, now_us: Micros) -> Micros {
        self.hold.value(now_us)
    }

    /// Anchor a restored snapshot at the clock reading `now_us`.
    pub(crate) fn anchor(&mut self, now_us: Micros) {
        self.hold.anchor(now_us);
    }

    /// Entries dropped by the minimum-window cap.
    #[must_use]
    pub fn window_overflows(&self) -> u32 {
        self.window.overflows()
    }

    /// PCR discontinuity / new connection: drops the minimum window and the
    /// histogram, **keeps the stall memory**.
    pub fn reset_baseline(&mut self) {
        self.window.clear();
        self.hist.clear();
        self.base = 0;
        self.istart = None;
        self.imax = None;
        self.recovering = false;
        self.origin = None;
        self.prev_raw = None;
        self.unrolled_ticks = 0;
        self.prev_d = None;
        self.j16 = 0;
    }

    /// Forget everything including the stall memory (new channel).
    pub fn reset(&mut self) {
        self.reset_baseline();
        self.hold.clear();
        self.pdv_last = 0;
    }

    /// Stall memory for session persistence, aged to the latest arrival.
    #[must_use]
    pub fn snapshot(&self) -> StallMemorySnapshot {
        self.hold.snapshot(self.last_arrival.unwrap_or(0))
    }

    /// Snapshot aged to an explicit clock reading.
    #[must_use]
    pub(crate) fn snapshot_at(&self, now_us: Micros) -> StallMemorySnapshot {
        self.hold.snapshot(now_us)
    }

    /// Install a stall memory (see [`StallMemorySnapshot`] for aging).
    pub fn restore(&mut self, m: StallMemorySnapshot) {
        self.hold.restore(m);
    }
}
