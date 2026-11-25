#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;

mod container_codec;
// `check_dash_mpd` parses the MPD with `broadcast_dash::Mpd`, whose quick-xml
// parser is `std`-only.
#[cfg(all(feature = "std", feature = "dash"))]
mod dash_validator;
mod diagnostics;
mod hls_validator;
#[cfg(feature = "net")]
pub mod metrics_server;
#[cfg(feature = "std")]
pub mod mp4_stream;
#[cfg(feature = "std")]
pub mod payload;
mod playlist;
mod report;
#[cfg(feature = "net")]
pub mod udp;
mod watch;
#[cfg(feature = "metrics")]
mod watch_metrics;

pub use container_codec::{ContainerCodecChecker, check_container_codec, looks_like_isobmff};
#[cfg(all(feature = "std", feature = "dash"))]
pub use dash_validator::check_dash_mpd;
pub use diagnostics::cc_anomaly::CcAnomalyCheck;
pub use diagnostics::codec_common::{DeclaredStream, PmtStreamCollector, PmtStreamIndex};
pub use diagnostics::codec_signalling::CodecSignallingCheck;
pub use diagnostics::fps_cadence::FpsCadenceCheck;
pub use diagnostics::interlace::InterlaceCheck;
pub use diagnostics::param_sets::ParamSetsCheck;
pub use diagnostics::pat_pmt_version::PatPmtVersionCheck;
pub use diagnostics::pcr_check::PcrCheck;
pub use diagnostics::pts_check::PtsCheck;
pub use diagnostics::scte35_check::Scte35Check;
pub use diagnostics::sync_byte::SyncByteCheck;
pub use hls_validator::check_hls_playlist;
#[cfg(feature = "std")]
pub use mp4_stream::check_mp4_file;
pub use playlist::check_playlist;
pub use report::{Finding, Location, Report, Severity, StreamLocator};
pub use watch::{ConformanceSample, PidFlag, WatchSnapshot, WatchState};
#[cfg(feature = "metrics")]
pub use watch_metrics::{publish as publish_metrics, render as render_metrics};

/// A pluggable diagnostic check over a Transport Stream.
///
/// An implementor constructs a [`DiagnosticScan`] — the incremental pass over
/// one packet at a time — from the stream's PMT-declared set. The whole-buffer
/// [`run`](Self::run) is a default that drives that same scan, so a file on
/// disk is checked through the identical state machine a buffer is.
pub trait Diagnostic {
    /// Start a streaming pass over one transport stream.
    fn scan(&self, pmt: &PmtStreamIndex) -> Box<dyn DiagnosticScan>;

    /// Whether the scan reads the PMT-declared stream set. A diagnostic that
    /// returns `true` needs the pass-1 [`PmtStreamIndex`] filled in.
    fn needs_pmt_streams(&self) -> bool {
        false
    }

    /// Check a whole TS byte buffer (contiguous 188-byte packets, no framing
    /// gaps — `ts.len()` is a multiple of 188), appending findings to
    /// `report`.
    fn run(&self, ts: &[u8], report: &mut Report) {
        let pmt = if self.needs_pmt_streams() {
            PmtStreamIndex::from_ts(ts)
        } else {
            PmtStreamIndex::empty()
        };
        let mut scan = self.scan(&pmt);
        for packet in ts.chunks_exact(mpeg_ts::ts::TS_PACKET_SIZE) {
            scan.feed(packet, report);
        }
        scan.finish(report);
    }
}

/// One incremental pass of a [`Diagnostic`], fed one 188-byte packet at a time.
pub trait DiagnosticScan {
    /// Examine one TS packet.
    fn feed(&mut self, packet: &[u8], report: &mut Report);
    /// End of stream: flush any buffered state.
    fn finish(&mut self, _report: &mut Report) {}
}

/// Run every registered [`Diagnostic`] against a TS buffer.
pub fn run_all(ts: &[u8], diagnostics: &[&dyn Diagnostic], report: &mut Report) {
    let mut runner = DiagnosticRunner::new(diagnostics, &pmt_index_for(ts, diagnostics));
    for packet in ts.chunks_exact(mpeg_ts::ts::TS_PACKET_SIZE) {
        runner.feed(packet);
    }
    runner.finish(report);
}

/// The PMT-declared set `diagnostics` need: the stream's own when any of them
/// reads it, an empty index otherwise.
fn pmt_index_for(ts: &[u8], diagnostics: &[&dyn Diagnostic]) -> PmtStreamIndex {
    if diagnostics.iter().any(|d| d.needs_pmt_streams()) {
        PmtStreamIndex::from_ts(ts)
    } else {
        PmtStreamIndex::empty()
    }
}

/// A set of diagnostics driven together over a packet stream.
///
/// Construct it with the stream's [`PmtStreamIndex`] (built in a first pass by
/// [`PmtStreamCollector`]), then feed every packet once: each diagnostic keeps
/// its own state, so the input is read a bounded chunk at a time and never
/// held whole.
///
/// Each diagnostic's findings are kept in its own report and merged into the
/// caller's at [`finish`](Self::finish), so the result reads exactly as if the
/// diagnostics had run one after another over the whole stream — the input is
/// shared, the output order is not.
pub struct DiagnosticRunner {
    scans: Vec<Box<dyn DiagnosticScan>>,
    reports: Vec<Report>,
}

impl DiagnosticRunner {
    /// Build one scan per diagnostic.
    #[must_use]
    pub fn new(diagnostics: &[&dyn Diagnostic], pmt: &PmtStreamIndex) -> Self {
        Self {
            scans: diagnostics.iter().map(|d| d.scan(pmt)).collect(),
            reports: diagnostics.iter().map(|_| Report::new()).collect(),
        }
    }

    /// Feed one TS packet to every scan.
    pub fn feed(&mut self, packet: &[u8]) {
        for (scan, report) in self.scans.iter_mut().zip(self.reports.iter_mut()) {
            scan.feed(packet, report);
        }
    }

    /// End of stream: flush every scan, then append every diagnostic's
    /// findings to `report` in diagnostic order.
    pub fn finish(&mut self, report: &mut Report) {
        for (scan, findings) in self.scans.iter_mut().zip(self.reports.iter_mut()) {
            scan.finish(findings);
        }
        for findings in self.reports.drain(..) {
            report.append(findings);
        }
    }
}

#[cfg(feature = "cli")]
pub mod cli;
