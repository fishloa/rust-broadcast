//! PCR-discontinuity detection (#562) — a standalone analysis pass over a
//! buffer of concatenated 188-byte TS packets.
//!
//! [`detect_pcr_discontinuities`] finds every PCR jump on every PCR-bearing
//! PID and classifies each one as:
//!
//! - **flagged** — the packet carrying the post-break PCR has
//!   `discontinuity_indicator == 1` (ISO/IEC 13818-1 §2.4.3.5): a legal
//!   system-time-base change.
//! - **unflagged** — the jump exceeds the ETSI TR 101 290 v1.4.1 §5.2.2
//!   Table 5.0b indicator 2.3b (`PCR_discontinuity_indicator_error`)
//!   threshold with no `discontinuity_indicator` set: a genuine defect. The
//!   2.3b check is reused verbatim from
//!   [`dvb_conformance::ConformanceMonitor`] — the 100 ms threshold is never
//!   re-derived in this crate.
//!
//! This is a read-only query, independent of the repair operations
//! ([`crate::TsFixBuilder::restamp_pcr`] / `.honor_pcr_discontinuity()`) —
//! useful for auditing a stream before deciding how (or whether) to repair
//! it. The repair ops perform the same classification internally as they
//! stream packets through.

use alloc::vec::Vec;

use mpeg_ts::ts::{TS_PACKET_SIZE, TsPacket};

use crate::ops::pcr_conformance::PcrDiscDetector;

/// One detected PCR discontinuity.
///
/// `#[non_exhaustive]` — future fields (e.g. the measured delta) may be added
/// without a breaking change.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcrDiscontinuity {
    /// The PCR PID this discontinuity was observed on.
    pub pid: u16,
    /// 0-based index (into 188-byte packets) of the packet carrying the
    /// post-break PCR.
    pub packet_index: u64,
    /// `true` when `discontinuity_indicator` was set on this packet — a
    /// legal system-time-base change (ISO/IEC 13818-1 §2.4.3.5). `false`
    /// means the break is unflagged: a genuine defect per ETSI TR 101 290
    /// §5.2.2 indicator 2.3b.
    pub flagged: bool,
}

/// Scan `ts_bytes` (concatenated 188-byte TS packets) for PCR discontinuities
/// on every PCR-bearing PID.
///
/// Trailing bytes that do not form a complete 188-byte packet are ignored.
///
/// # Example
///
/// ```rust,no_run
/// use ts_fix::discontinuity::detect_pcr_discontinuities;
///
/// let bytes = std::fs::read("capture.ts").unwrap();
/// for d in detect_pcr_discontinuities(&bytes) {
///     if !d.flagged {
///         println!(
///             "unflagged PCR break on PID 0x{:04X} at packet {}",
///             d.pid, d.packet_index
///         );
///     }
/// }
/// ```
///
/// # Spec
///
/// ISO/IEC 13818-1 (= ITU-T H.222.0) §2.4.3.5; ETSI TR 101 290 v1.4.1 §5.2.2,
/// Table 5.0b, indicator 2.3b.
#[must_use]
pub fn detect_pcr_discontinuities(ts_bytes: &[u8]) -> Vec<PcrDiscontinuity> {
    let mut detector = PcrDiscDetector::new();
    let mut found = Vec::new();

    for (index, packet) in ts_bytes.chunks_exact(TS_PACKET_SIZE).enumerate() {
        let packet_index = index as u64;

        // Flagged: a direct, threshold-free read of discontinuity_indicator
        // on a PCR-bearing packet (a legal system-time-base change).
        if let Ok(pkt) = TsPacket::parse(packet) {
            if let Some(Ok(af)) = pkt.adaptation_field() {
                if af.pcr.is_some() && af.discontinuity_indicator {
                    found.push(PcrDiscontinuity {
                        pid: pkt.header.pid,
                        packet_index,
                        flagged: true,
                    });
                }
            }
        }

        // Unflagged: TR 101 290 §5.2.2 indicator 2.3b, reused from
        // dvb-conformance. Must run on every packet (flagged or not) to keep
        // the detector's per-PID PCR state synced with the true input.
        if let Some(pid) = detector.feed(packet) {
            found.push(PcrDiscontinuity {
                pid,
                packet_index,
                flagged: false,
            });
        }
    }

    found
}
