//! PCR-discontinuity "honor" repair (#562).
//!
//! Unlike [`super::pcr_restamp`] (which rewrites PCR values onto a continuous
//! timeline), honor mode leaves every timestamp byte untouched and instead
//! **marks** a genuine, unflagged PCR break by setting
//! `discontinuity_indicator` in the adaptation-field flags byte
//! (ISO/IEC 13818-1 §2.4.3.5) — turning an unsignalled defect into a legally
//! signalled system-time-base change. All other bytes — including the PCR
//! field itself — are passed through byte-identical.
//!
//! # Detection
//!
//! "Genuine, unflagged break" is determined by
//! [`super::pcr_conformance::PcrDiscDetector`], which reuses
//! `dvb_conformance::ConformanceMonitor`'s ETSI TR 101 290 §5.2.2 indicator
//! 2.3b (`PCR_discontinuity_indicator_error`) check verbatim — the 100 ms
//! threshold is never re-derived here. A packet that already carries
//! `discontinuity_indicator == 1` never raises 2.3b (it is a legal break), so
//! honor mode never touches an already-flagged packet.
//!
//! # Why this is always byte-safe
//!
//! Indicator 2.3b can only fire on a packet that itself carries a PCR (the
//! delta is computed between consecutive PCR values), so the flagged packet
//! always already has an adaptation field with a valid flags byte at offset
//! 5 — setting bit `0x80` there never changes `adaptation_field_length` or
//! shifts any other byte.
//!
//! # Spec
//!
//! ISO/IEC 13818-1 (= ITU-T H.222.0) §2.4.3.5 (`discontinuity_indicator`);
//! ETSI TR 101 290 v1.4.1 §5.2.2, Table 5.0b, indicator 2.3b.

use mpeg_ts::ts::TS_PACKET_SIZE;

use crate::ops::pcr_conformance::PcrDiscDetector;
use crate::ops::{Op, StreamModel};

/// Bit mask for `discontinuity_indicator` in the adaptation field flags byte
/// (ISO/IEC 13818-1 §2.4.3.5), at byte offset 5 of the 188-byte packet.
const AF_DISCONTINUITY: u8 = 0x80;

/// PCR-discontinuity honor operation — flags genuine unflagged breaks without
/// rewriting any timestamp.
pub(crate) struct PcrHonorOp {
    detector: PcrDiscDetector,
}

impl PcrHonorOp {
    pub(crate) fn new() -> Self {
        Self {
            detector: PcrDiscDetector::new(),
        }
    }
}

impl Op for PcrHonorOp {
    fn process(&mut self, packet: &[u8], _model: &mut StreamModel, out: &mut dyn FnMut(&[u8])) {
        if packet.len() != TS_PACKET_SIZE {
            out(packet);
            return;
        }

        // Feed the ORIGINAL bytes so the conformance state machine tracks the
        // true input timeline regardless of what this op does with the packet.
        let is_genuine_break = self.detector.feed(packet).is_some();
        if !is_genuine_break {
            out(packet);
            return;
        }

        // Genuine, unflagged break (TR 101 290 §5.2.2 indicator 2.3b): set
        // discontinuity_indicator. Only the flag bit changes; every other
        // byte — including the PCR value — is preserved.
        let mut buf = [0u8; TS_PACKET_SIZE];
        buf.copy_from_slice(packet);
        buf[5] |= AF_DISCONTINUITY;
        out(&buf);
    }

    fn flush(&mut self, _model: &mut StreamModel, _out: &mut dyn FnMut(&[u8])) {
        // Stateless across packets beyond the detector's per-PID PCR state,
        // which is internal to `dvb_conformance::ConformanceMonitor` and does
        // not need flushing.
    }
}
