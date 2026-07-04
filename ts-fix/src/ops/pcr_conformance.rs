//! Shared PCR-discontinuity classification — a thin, stateful wrapper over
//! [`dvb_conformance::ConformanceMonitor`]'s ETSI TR 101 290 v1.4.1 Table 5.0b
//! indicator 2.3b (`PCR_discontinuity_indicator_error`) check.
//!
//! Both [`super::pcr_restamp`] (Interpolate mode's sane/corrupt decision) and
//! [`super::pcr_honor`] (setting `discontinuity_indicator` on a genuine break)
//! need the *same* answer to "is this PCR delta a genuine, unflagged
//! discontinuity, per the TR 101 290 100 ms threshold?". ts-fix does **not**
//! re-derive that threshold — [`PcrDiscDetector`] feeds every packet through
//! the real conformance monitor and asks whether indicator 2.3b fired, so any
//! future threshold tuning in `dvb-conformance` is picked up here for free.
//!
//! # Synthetic clock
//!
//! [`dvb_conformance::ConformanceMonitor::feed`] takes a caller-supplied
//! wall-clock [`Duration`] per packet. Indicator 2.3b's decision is derived
//! purely from the **PCR value delta** between consecutive PCRs on the same
//! PID (`check_pcr`'s `delta_ms` — not from the caller's `t`); `t` only gates
//! the *other* PCR indicator (2.3a, `PCR_repetition_error`) and the
//! PAT/PMT/SI presence timers, neither of which this detector consults. ts-fix
//! repair ops process a raw byte stream with no independent wall clock, so a
//! monotonically increasing synthetic per-packet counter is fed instead — it
//! is sufficient because only 2.3b events are read back out.
//!
//! # Spec
//!
//! ETSI TR 101 290 v1.4.1 §5.2.2, Table 5.0b, indicator 2.3b. ISO/IEC 13818-1
//! (= ITU-T H.222.0) §2.4.3.5 (`discontinuity_indicator`).

use core::time::Duration;

use dvb_conformance::{ConformanceMonitor, Indicator};

/// Incremental TR 101 290 §5.2.2 indicator 2.3b classifier.
pub(crate) struct PcrDiscDetector {
    monitor: ConformanceMonitor,
    /// Synthetic per-packet clock (microseconds), monotonically increasing.
    tick_us: u64,
}

impl PcrDiscDetector {
    /// Create a detector using `dvb-conformance`'s default `Config`
    /// (100 ms PCR-discontinuity threshold, TR 101 290 v1.4.1 Table 5.0b
    /// note under indicator 2.3b).
    pub(crate) fn new() -> Self {
        Self {
            monitor: ConformanceMonitor::new(),
            tick_us: 0,
        }
    }

    /// Feed one **original, unmodified** 188-byte TS packet.
    ///
    /// Returns `Some(pid)` when this packet raised
    /// `Indicator::PcrDiscontinuityError` — a PCR delta on `pid` exceeding the
    /// TR 101 290 threshold with `discontinuity_indicator == 0` (a genuine,
    /// unflagged break; flagged breaks never raise this indicator by
    /// construction). Returns `None` otherwise (no PCR on this packet, delta
    /// within tolerance, or a legally flagged discontinuity).
    ///
    /// Must be called on every packet in stream order (including ones this
    /// caller ultimately treats specially, e.g. flagged discontinuities) so
    /// the monitor's per-PID PCR state stays in sync with the true input
    /// timeline.
    pub(crate) fn feed(&mut self, packet: &[u8]) -> Option<u16> {
        let t = Duration::from_micros(self.tick_us);
        self.tick_us = self.tick_us.saturating_add(1);
        let events = self.monitor.feed(packet, t);
        for event in events {
            if event.indicator == Indicator::PcrDiscontinuityError {
                return event.pid;
            }
        }
        None
    }
}
