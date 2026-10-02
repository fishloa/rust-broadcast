//! Per-PID PES reassembly from TS payloads.
//!
//! In PES-over-TS there is no `pointer_field`: a TS packet with
//! `payload_unit_start_indicator = 1` *begins* a PES packet, and continuation
//! packets (`PUSI = 0`) append to it. A PES therefore runs from one PUSI to the
//! next (the unbounded-video case, `PES_packet_length = 0`, is handled the same
//! way — flushed when the next unit starts or at end of stream).

use alloc::vec::Vec;
use broadcast_common::pusi::PusiAccumulator;

/// Reassembles PES packets for a single PID from successive TS payloads.
///
/// Feed each TS packet's payload with its `payload_unit_start_indicator`;
/// [`feed`](Self::feed) returns the **previous** completed PES's bytes when a new
/// unit starts. Call [`flush`](Self::flush) at end of stream for the last one.
/// The returned `Vec<u8>` is ready for [`crate::PesPacket::parse`].
///
/// A thin wrapper over the shared [`broadcast_common::pusi::PusiAccumulator`]
/// (the same rule `mpeg-ts`'s `PusiReassembler` uses), so a PES that never
/// sees another PUSI — legal for `PES_packet_length == 0` video — is bounded
/// by [`DEFAULT_MAX_UNIT_SIZE`](broadcast_common::pusi::DEFAULT_MAX_UNIT_SIZE)
/// rather than growing without limit (audit r01-W13, #1074). A PES past the
/// cap is discarded, and reassembly resumes at the next PUSI.
#[derive(Debug, Default)]
pub struct PesAssembler {
    inner: PusiAccumulator,
}

impl PesAssembler {
    /// New, empty assembler with the default 16 MiB per-PES cap.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// New assembler that discards any PES growing past `max` bytes.
    #[must_use]
    pub fn with_max_unit_size(max: usize) -> Self {
        Self {
            inner: PusiAccumulator::with_max_unit_size(max),
        }
    }

    /// Feed one TS packet's payload for this PID.
    ///
    /// `payload_unit_start` is the packet's `payload_unit_start_indicator`.
    /// Returns the bytes of the now-complete previous PES packet, if any.
    #[must_use]
    pub fn feed(&mut self, payload_unit_start: bool, payload: &[u8]) -> Option<Vec<u8>> {
        self.inner.feed(payload_unit_start, payload)
    }

    /// Take the final buffered PES at end of stream, if any.
    #[must_use]
    pub fn flush(&mut self) -> Option<Vec<u8>> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PesPacket;

    #[test]
    fn reassembles_across_packets_and_flushes() {
        let mut a = PesAssembler::new();
        // PES #1 split over 2 TS payloads.
        assert_eq!(
            a.feed(true, &[0x00, 0x00, 0x01, 0xE0, 0x00, 0x00, 0x80]),
            None
        );
        assert_eq!(
            a.feed(false, &[0x80, 0x05, 0x21, 0x00, 0x01, 0x00, 0x01]),
            None
        );
        assert_eq!(a.feed(false, &[0xAA, 0xBB]), None);
        // PES #2 starts → #1 emitted.
        let first = a
            .feed(
                true,
                &[0x00, 0x00, 0x01, 0xC0, 0x00, 0x00, 0x80, 0x00, 0x00, 0x11],
            )
            .expect("first PES emitted on next unit start");
        let p1 = PesPacket::parse(&first).unwrap();
        assert!(p1.stream_id.is_video());
        assert_eq!(p1.payload, &[0xAA, 0xBB]);
        // flush → #2.
        let second = a.flush().expect("second PES flushed");
        let p2 = PesPacket::parse(&second).unwrap();
        assert!(p2.stream_id.is_audio());
        assert!(a.flush().is_none());
    }

    /// A PES with no following PUSI (e.g. `PES_packet_length == 0` video) must
    /// not grow past the cap (audit r01-W13): the oversized PES is dropped and
    /// the next PUSI restarts cleanly.
    #[test]
    fn oversized_pes_is_discarded_not_buffered_forever() {
        let mut a = PesAssembler::with_max_unit_size(100);
        assert_eq!(a.feed(true, &[0xAA; 60]), None);
        assert_eq!(a.feed(false, &[0xBB; 60]), None); // 120 > 100: discarded
        assert_eq!(a.feed(false, &[0xCC; 10]), None); // ignored, not re-started
        assert_eq!(
            a.feed(true, &[0xDD; 3]),
            None,
            "nothing to emit: the PES was dropped"
        );
        assert_eq!(a.flush().as_deref(), Some([0xDD; 3].as_slice()));
    }

    #[test]
    fn ignores_continuation_before_first_start() {
        let mut a = PesAssembler::new();
        assert_eq!(a.feed(false, &[0xDE, 0xAD]), None); // mid-stream join
        assert!(a.flush().is_none());
    }
}
