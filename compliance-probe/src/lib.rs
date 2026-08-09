//! `compliance-probe` — a live TR 101 290 + PCR-drift + SCTE-35 cue-sanity
//! probe, meant to run continuously beside the stream it watches and export
//! what it finds through the [`metrics`] facade (issue #930,
//! `docs/IDEAS.md` item #4).
//!
//! # What this crate measures
//!
//! - **ETSI TR 101 290 priority 1/2/3 indicators** ([`dvb_conformance`],
//!   already shipped) — every indicator that crate implements, fed one TS
//!   packet at a time via [`Probe::feed_ts_packet`]. Continuity-count errors
//!   (indicator 1.4) are one of these indicators, not a separate metric —
//!   see [`crate::metric_names::TR101290_EVENTS_TOTAL`].
//! - **PCR interval-error ("drift") and jitter**, per PID — new in this
//!   crate; see [`conformance`] for the exact arithmetic and why it is
//!   named apart from the spec's `PCR_accuracy_error` (2.4).
//! - **SCTE-35 cue sanity** — well-formedness (wire path only), cues
//!   arriving, and future-vs-past `pts_time` judgement; see [`scte35`].
//! - **Container/stream structural checks**, delegated to
//!   `media-doctor`'s already-shipped [`media_doctor::Diagnostic`] harness;
//!   see [`structural`].
//!
//! # What this crate does **not** measure, and why
//!
//! - **TR 101 290 indicator 2.4, `PCR_accuracy_error`** — `dvb-conformance`
//!   itself never emits it (needs ±500 ns hardware arrival timing this
//!   sans-IO monitor cannot honestly provide). This crate's own
//!   `compliance_probe_pcr_drift_ppm`/`_jitter_ppm` gauges are a different,
//!   coarser, software-clock signal — see [`conformance`]'s module docs.
//! - **T-STD buffer-model sub-checks** `dvb-conformance` documents as
//!   partial (`Buffer_error` TBn overflow; `Empty_buffer_error` MBn;
//!   `Data_delay_error`'s still-picture 60 s threshold) — this crate adds no
//!   buffer-model logic of its own; whatever that crate emits is exactly
//!   what is exported, no more.
//! - **SCTE-35 malformed-section detection on the Trunk-cursor path**
//!   ([`Probe::drain_event_cursor`], `std` only) — an event already
//!   published into a `Trunk`'s event log was, by construction, already
//!   parsed successfully upstream. Only the wire path
//!   ([`Probe::feed_scte35_section`]) can observe malformedness.
//! - **A fabricated "now" for a SCTE-35 cue whose media time is not yet
//!   resolved** — an [`media_plane::EventAnchor::Segment`]/`Utc` entry is
//!   left exactly that, never guessed into a `Media` time; see
//!   [`trunk_bridge`] (`std` only).
//! - **This crate does not open a socket or serve `/metrics`.** It records
//!   through the `metrics` facade only; a host process (e.g. `multimux`,
//!   which already installs a process-wide
//!   `metrics_exporter_prometheus` recorder — see `multimux::prometheus`)
//!   renders the exposition. See [`crate::metric_names`] for the full list of metric
//!   names this crate records.
//!
//! # The clock you feed is part of the measurement
//!
//! [`Probe::feed_ts_packet`] takes an arrival timestamp, and **most TR 101 290
//! indicators are timeout-based** — PAT absent > 500 ms, PID absent > 5 s, SI
//! repetition intervals of 2/10/30 s, and the T-STD buffer model's
//! 1 Mbit/s TBsys drain. The clock is therefore not bookkeeping; it is an
//! *input to the result*, and feeding a dishonest one silently changes what
//! the probe reports.
//!
//! This is not hypothetical. `tests/wasm_analyzer_equivalence.rs` pins a
//! measured, reproducible instance: this repository's `demo/` WASM analyzer
//! anchors its clock on observed PCR values and falls back to `+1 ns` per
//! packet until the first PCR arrives — and `fixtures/ts/m6-single.ts`
//! contains **no PCR at all**, so that analyzer models 1264 packets as
//! spanning 1.264 µs (an implied ~1.5 Tbit/s). At that implied rate TBsys
//! cannot drain, so indicator 3.3 `Buffer_error` fires 35 times; under any
//! physically plausible bitrate it fires zero times. Same crate, same
//! [`dvb_conformance::Config`], same fixture — 838 events vs. 803, entirely
//! from the clock.
//!
//! Two consequences for a caller:
//!
//! - **Feed real arrival time**, not a packet-index-derived synthetic clock.
//!   [`Probe::drain_byte_tap`] does exactly this, using the
//!   [`broadcast_common::Timestamp`] the [`media_plane::ByteTap`] recorded
//!   with each item.
//! - **`Buffer_error`/`Empty_buffer_error`/`Data_delay_error` are statements
//!   about arrival timing**, so they are only as trustworthy as that clock.
//!   `Continuity_count_error`, by contrast, is clock-independent — the
//!   equivalence test asserts it is 803 on that fixture at every clock rate
//!   from frozen to 2 ms/packet.
//!
//! # Cost: one cursor, not a second copy of the data
//!
//! [`Probe::drain_byte_tap`] and [`Probe::drain_event_cursor`] (`std` only)
//! are the two `media-plane` attachment points this crate is built to use —
//! see [`trunk_bridge`] for why TR 101 290 specifically needs a
//! [`media_plane::ByteTap`] (a demuxed `Sample` has already discarded the
//! continuity counter and `transport_error_indicator` a demuxer's job is to
//! discard) rather than a `Trunk` sample cursor, and why SCTE-35 sanity
//! *can* use a real `Trunk` `EventCursor` (the event log already carries
//! `timed_metadata::TimedEvent`, `media-plane`'s own intended consumer for
//! exactly this). Both are bounded, non-blocking observers — see each type's
//! own docs for its `Lagged` accounting — so attaching this probe never
//! costs the ingest path a second copy of the stream or a blocking reader.

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
