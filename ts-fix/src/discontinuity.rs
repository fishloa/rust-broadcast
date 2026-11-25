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
use mpeg_ts::pid::Pid;

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
    pub pid: Pid,
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
/// For a capture that does not fit in memory use
/// [`PcrDiscontinuityScanner`], which is the same pass fed in pieces.
///
/// # Example
///
/// ```rust,no_run
/// use std::fs::File;
/// use std::io::{BufReader, Read};
/// use ts_fix::discontinuity::PcrDiscontinuityScanner;
///
/// const CHUNK: usize = 1 << 20;
///
/// let mut reader = BufReader::with_capacity(CHUNK, File::open("capture.ts")?);
/// let mut scanner = PcrDiscontinuityScanner::new();
/// let mut chunk = vec![0u8; CHUNK];
/// loop {
///     let read = reader.read(&mut chunk)?;
///     if read == 0 {
///         break;
///     }
///     for d in scanner.feed(&chunk[..read]) {
///         if !d.flagged {
///             println!(
///                 "unflagged PCR break on PID 0x{:04X} at packet {}",
///                 d.pid, d.packet_index
///             );
///         }
///     }
/// }
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// # Spec
///
/// ISO/IEC 13818-1 (= ITU-T H.222.0) §2.4.3.5; ETSI TR 101 290 v1.4.1 §5.2.2,
/// Table 5.0b, indicator 2.3b.
#[must_use]
pub fn detect_pcr_discontinuities(ts_bytes: &[u8]) -> Vec<PcrDiscontinuity> {
    PcrDiscontinuityScanner::new().feed(ts_bytes)
}

/// The streaming form of [`detect_pcr_discontinuities`]: feed the capture in
/// pieces of any size.
///
/// The per-packet rule it applies is the batch pass's: a PCR-bearing packet
/// with `discontinuity_indicator` set (ISO/IEC 13818-1 §2.4.3.5) is a
/// signalled break, and an unflagged jump beyond 100 ms is ETSI TR 101 290
/// v1.4.1 §5.2.2 Table 5.0b indicator 2.3b.
///
/// Complete 188-byte packets are examined as they arrive; a trailing partial
/// packet is carried to the next [`feed`](Self::feed). Anything left partial
/// when the input ends is ignored, exactly as the batch pass ignores it.
pub struct PcrDiscontinuityScanner {
    detector: PcrDiscDetector,
    packet_index: u64,
    leftover: [u8; TS_PACKET_SIZE],
    leftover_len: usize,
}

impl PcrDiscontinuityScanner {
    /// A scanner that has seen no byte yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            detector: PcrDiscDetector::new(),
            packet_index: 0,
            leftover: [0u8; TS_PACKET_SIZE],
            leftover_len: 0,
        }
    }

    /// Feed the next piece of the capture, returning every discontinuity the
    /// completed packets in it exposed.
    #[must_use]
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<PcrDiscontinuity> {
        let mut found = Vec::new();
        let mut offset = 0usize;

        if self.leftover_len > 0 {
            let want = TS_PACKET_SIZE - self.leftover_len;
            let take = want.min(bytes.len());
            self.leftover[self.leftover_len..self.leftover_len + take]
                .copy_from_slice(&bytes[..take]);
            self.leftover_len += take;
            offset = take;
            if self.leftover_len < TS_PACKET_SIZE {
                return found;
            }
            let packet = self.leftover;
            self.examine(&packet, &mut found);
            self.leftover_len = 0;
        }

        while offset + TS_PACKET_SIZE <= bytes.len() {
            self.examine(&bytes[offset..offset + TS_PACKET_SIZE], &mut found);
            offset += TS_PACKET_SIZE;
        }

        let rest = &bytes[offset..];
        self.leftover[..rest.len()].copy_from_slice(rest);
        self.leftover_len = rest.len();
        found
    }

    /// Examine one complete packet, appending what it exposed.
    fn examine(&mut self, packet: &[u8], found: &mut Vec<PcrDiscontinuity>) {
        let packet_index = self.packet_index;
        self.packet_index += 1;

        // Flagged: a direct, threshold-free read of discontinuity_indicator
        // on a PCR-bearing packet (a legal system-time-base change).
        if let Ok(pkt) = TsPacket::parse(packet)
            && let Some(Ok(af)) = pkt.adaptation_field()
            && af.pcr.is_some()
            && af.discontinuity_indicator
        {
            found.push(PcrDiscontinuity {
                pid: pkt.header.pid,
                packet_index,
                flagged: true,
            });
        }

        // Unflagged: TR 101 290 §5.2.2 indicator 2.3b, reused from
        // dvb-conformance. Must run on every packet (flagged or not) to keep
        // the detector's per-PID PCR state synced with the true input.
        if let Some(pid) = self.detector.feed(packet) {
            found.push(PcrDiscontinuity {
                pid,
                packet_index,
                flagged: false,
            });
        }
    }
}

impl Default for PcrDiscontinuityScanner {
    fn default() -> Self {
        Self::new()
    }
}
