//! PUSI-delimited unit accumulation — ISO/IEC 13818-1 (H.222.0) §2.4.3.2.
//!
//! A transport-stream PID that carries packetised data rather than PSI
//! sections delimits its units with `payload_unit_start_indicator` (PUSI): the
//! packet with PUSI = 1 *begins* a unit, and every following PUSI = 0 packet
//! on that PID appends to it (a PES packet, §2.4.3.6; a DASH `emsg` box on
//! PID `0x0004`, ISO/IEC 23009-1 §5.10.3.3.5). A unit therefore runs from one
//! PUSI to the next, or to end of stream.
//!
//! [`PusiAccumulator`] is that one rule, written once. `mpeg-pes`'s
//! `PesAssembler` and `mpeg-ts`'s `PusiReassembler` (which adds a PID filter)
//! both wrap it, so a gating or memory-cap fix lands in a single place (audit
//! r01-W13, #1074 — the two copies had diverged: one capped its buffer, the
//! other let a PES with no following PUSI grow without bound).

use alloc::vec::Vec;

/// Default maximum accumulated unit size: 16 MiB.
///
/// A PES packet with `PES_packet_length == 0` is unbounded by design
/// (ISO/IEC 13818-1 §2.4.3.7, video only), so a practical cap is what stops a
/// stream that never sends another PUSI from exhausting memory, while still
/// accommodating large payloads such as I-frames.
pub const DEFAULT_MAX_UNIT_SIZE: usize = 16 * 1024 * 1024;

/// Accumulates one PID's payload bytes into PUSI-delimited units.
///
/// Rules (all literal, all tested):
/// - payload before the first PUSI is ignored (a mid-stream join must not
///   prepend unrelated bytes to the first unit);
/// - a PUSI packet closes the in-progress unit (returned if non-empty) and
///   starts a new one with its own payload;
/// - a unit that would exceed the cap is discarded and nothing accumulates
///   until the next PUSI restarts it;
/// - [`flush`](Self::flush) returns the final unit, which has no following
///   PUSI to close it.
#[derive(Debug, Clone)]
pub struct PusiAccumulator {
    buf: Vec<u8>,
    /// `true` between a PUSI and the next PUSI / overflow / flush.
    started: bool,
    max_unit_size: usize,
}

impl Default for PusiAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl PusiAccumulator {
    /// New accumulator with the [`DEFAULT_MAX_UNIT_SIZE`] cap.
    #[must_use]
    pub fn new() -> Self {
        Self::with_max_unit_size(DEFAULT_MAX_UNIT_SIZE)
    }

    /// New accumulator that discards any unit growing past `max` bytes.
    #[must_use]
    pub fn with_max_unit_size(max: usize) -> Self {
        Self {
            buf: Vec::new(),
            started: false,
            max_unit_size: max,
        }
    }

    /// The configured cap, in bytes.
    #[must_use]
    pub fn max_unit_size(&self) -> usize {
        self.max_unit_size
    }

    /// Feed one TS packet's payload for this PID.
    ///
    /// Returns the bytes of the unit a PUSI just closed, if it was non-empty.
    #[must_use]
    pub fn feed(&mut self, pusi: bool, payload: &[u8]) -> Option<Vec<u8>> {
        if pusi {
            let completed = if self.started && !self.buf.is_empty() {
                Some(core::mem::take(&mut self.buf))
            } else {
                self.buf.clear();
                None
            };
            if payload.len() > self.max_unit_size {
                self.started = false;
            } else {
                self.started = true;
                self.buf.extend_from_slice(payload);
            }
            return completed;
        }
        if !self.started {
            return None;
        }
        if self.buf.len().saturating_add(payload.len()) > self.max_unit_size {
            self.buf.clear();
            self.started = false;
            return None;
        }
        self.buf.extend_from_slice(payload);
        None
    }

    /// Take the final unit at end of stream, if any, and reset.
    #[must_use]
    pub fn flush(&mut self) -> Option<Vec<u8>> {
        self.started = false;
        if self.buf.is_empty() {
            None
        } else {
            Some(core::mem::take(&mut self.buf))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn ignores_payload_before_first_pusi() {
        let mut a = PusiAccumulator::new();
        assert_eq!(a.feed(false, b"junk"), None);
        assert_eq!(a.feed(true, b"real"), None);
        assert_eq!(a.flush().as_deref(), Some(b"real".as_slice()));
    }

    #[test]
    fn pusi_closes_previous_unit_and_flush_returns_last() {
        let mut a = PusiAccumulator::new();
        assert_eq!(a.feed(true, b"AA"), None);
        assert_eq!(a.feed(false, b"aa"), None);
        assert_eq!(a.feed(true, b"BB").as_deref(), Some(b"AAaa".as_slice()));
        assert_eq!(a.flush().as_deref(), Some(b"BB".as_slice()));
        assert_eq!(a.flush(), None);
    }

    #[test]
    fn empty_unit_is_not_emitted() {
        let mut a = PusiAccumulator::new();
        assert_eq!(a.feed(true, b""), None);
        assert_eq!(
            a.feed(true, b"X"),
            None,
            "an empty unit is dropped, not returned"
        );
        assert_eq!(a.flush().as_deref(), Some(b"X".as_slice()));
    }

    #[test]
    fn overflow_discards_unit_until_next_pusi() {
        let mut a = PusiAccumulator::with_max_unit_size(10);
        assert_eq!(a.feed(true, &[1; 6]), None);
        // 6 + 6 > 10: the unit is discarded...
        assert_eq!(a.feed(false, &[2; 6]), None);
        // ...and later continuations are ignored, not accumulated.
        assert_eq!(a.feed(false, &[3; 2]), None);
        assert_eq!(
            a.feed(true, b"next"),
            None,
            "the oversized unit was discarded"
        );
        assert_eq!(a.flush().as_deref(), Some(b"next".as_slice()));
    }

    #[test]
    fn unit_exactly_at_cap_is_kept() {
        let mut a = PusiAccumulator::with_max_unit_size(10);
        assert_eq!(a.feed(true, &[1; 6]), None);
        assert_eq!(a.feed(false, &[2; 4]), None);
        assert_eq!(a.flush(), Some(vec![1, 1, 1, 1, 1, 1, 2, 2, 2, 2]));
    }

    #[test]
    fn oversized_first_payload_is_discarded() {
        let mut a = PusiAccumulator::with_max_unit_size(4);
        assert_eq!(a.feed(true, &[9; 5]), None);
        assert_eq!(a.feed(false, &[9; 1]), None);
        assert_eq!(a.flush(), None);
    }

    #[test]
    fn default_cap_is_16_mib() {
        assert_eq!(DEFAULT_MAX_UNIT_SIZE, 16_777_216);
        assert_eq!(PusiAccumulator::new().max_unit_size(), 16_777_216);
    }
}
