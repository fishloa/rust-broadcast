#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod conformance;
pub mod metric_names;
mod record;
pub mod scte35;
pub mod structural;
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub mod trunk_bridge;

pub use conformance::PcrSample;
pub use scte35::Scte35Sanity;
pub use structural::run_structural_checks;

use core::time::Duration;

use conformance::PcrTracker;
use dvb_conformance::{Config as ConformanceConfig, ConformanceMonitor, Stats as ConformanceStats};
use mpeg_ts::ts::TsPacket;
use record::{record_counter, record_gauge};

/// Drives every per-packet/per-section check this crate implements, and
/// records what it finds through the `metrics` facade. See the crate docs
/// for the full scope and its documented gaps.
pub struct Probe {
    conformance: ConformanceMonitor,
    pcr: PcrTracker,
}

impl Default for Probe {
    fn default() -> Self {
        Self::new()
    }
}

impl Probe {
    /// Build a probe with `dvb-conformance`'s default thresholds.
    #[must_use]
    pub fn new() -> Self {
        Self {
            conformance: ConformanceMonitor::new(),
            pcr: PcrTracker::new(),
        }
    }

    /// Build a probe with caller-supplied TR 101 290 thresholds (PAT/PMT max
    /// intervals, PCR repetition/discontinuity limits, SI repetition
    /// intervals, …) — passed straight through to
    /// [`dvb_conformance::ConformanceMonitor::with_config`].
    #[must_use]
    pub fn with_config(config: ConformanceConfig) -> Self {
        Self {
            conformance: ConformanceMonitor::with_config(config),
            pcr: PcrTracker::new(),
        }
    }

    /// Feed one already sync-aligned 188-byte TS packet at caller wall-clock
    /// arrival time `t` (monotonic non-decreasing across calls — the same
    /// contract [`dvb_conformance::ConformanceMonitor::feed`] documents).
    ///
    /// Records `compliance_probe_ts_packets_total`,
    /// `compliance_probe_tr101290_events_total`,
    /// `compliance_probe_tr101290_in_sync`, and — when this packet carries a
    /// PCR — `compliance_probe_pcr_drift_ppm`/`_jitter_ppm` for its PID.
    pub fn feed_ts_packet(&mut self, packet: &[u8; mpeg_ts::ts::TS_PACKET_SIZE], t: Duration) {
        record_counter!(crate::metric_names::TS_PACKETS_TOTAL);

        for ev in self.conformance.feed(packet, t) {
            record_counter!(
                crate::metric_names::TR101290_EVENTS_TOTAL,
                "indicator" => ev.indicator.name(),
                "priority" => ev.priority.name(),
            );
        }
        let in_sync = self.conformance.stats().in_sync;
        record_gauge!(
            crate::metric_names::TR101290_IN_SYNC,
            if in_sync { 1.0 } else { 0.0 }
        );

        let Ok(ts_packet) = TsPacket::parse(packet) else {
            return;
        };
        if !ts_packet.header.has_adaptation {
            return;
        }
        let Some(Ok(af)) = ts_packet.adaptation_field() else {
            return;
        };
        let Some(pcr) = af.pcr else {
            return;
        };
        if af.discontinuity_indicator {
            // The broadcaster has signalled that this PID's PCR series is
            // not continuous here (ISO/IEC 13818-1 §2.4.3.4) — measure a
            // fresh baseline instead of reporting drift across the jump
            // (W-CP-2).
            self.pcr.reset(ts_packet.header.pid);
        }
        if let Some(sample) = self.pcr.observe(ts_packet.header.pid, pcr.as_27mhz(), t) {
            let pid_label = alloc::format!("0x{:04X}", sample.pid);
            record_gauge!(
                crate::metric_names::PCR_DRIFT_PPM,
                sample.drift_ppm,
                "pid" => pid_label.clone()
            );
            record_gauge!(
                crate::metric_names::PCR_JITTER_PPM,
                sample.jitter_ppm,
                "pid" => pid_label
            );
        }
    }

    /// Check one raw `splice_info_section` against a reference "now" on the
    /// same 33-bit 90 kHz `pts_time` clock. See [`scte35::check_section`] —
    /// this is a thin passthrough kept on `Probe` for API symmetry with
    /// [`Probe::feed_ts_packet`].
    pub fn feed_scte35_section(&mut self, section: &[u8], now_pts: u64) -> Scte35Sanity {
        scte35::check_section(section, now_pts)
    }

    /// The underlying [`dvb_conformance::ConformanceMonitor`]'s diagnostic
    /// counters (packets fed, events raised, current sync state).
    #[must_use]
    pub fn conformance_stats(&self) -> ConformanceStats {
        self.conformance.stats()
    }
}
