//! Per-PID PES reassembly from TS payloads.
//!
//! In PES-over-TS there is no `pointer_field`: a TS packet with
//! `payload_unit_start_indicator = 1` *begins* a PES packet, and continuation
//! packets (`PUSI = 0`) append to it. A PES therefore runs from one PUSI to the
//! next (the unbounded-video case, `PES_packet_length = 0`, is handled the same
//! way — flushed when the next unit starts or at end of stream).

use alloc::vec::Vec;

/// Reassembles PES packets for a single PID from successive TS payloads.
///
/// Feed each TS packet's payload with its `payload_unit_start_indicator`;
/// [`feed`](Self::feed) returns the **previous** completed PES's bytes when a new
/// unit starts. Call [`flush`](Self::flush) at end of stream for the last one.
/// The returned `Vec<u8>` is ready for [`crate::PesPacket::parse`].
#[derive(Debug, Default)]
pub struct PesAssembler {
    buf: Vec<u8>,
    started: bool,
}

impl PesAssembler {
    /// New, empty assembler.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one TS packet's payload for this PID.
    ///
    /// `payload_unit_start` is the packet's `payload_unit_start_indicator`.
    /// Returns the bytes of the now-complete previous PES packet, if any.
    #[must_use]
    pub fn feed(&mut self, payload_unit_start: bool, payload: &[u8]) -> Option<Vec<u8>> {
        if payload_unit_start {
            let completed = if self.started && !self.buf.is_empty() {
                Some(core::mem::take(&mut self.buf))
            } else {
                None
            };
            self.started = true;
            self.buf.extend_from_slice(payload);
            completed
        } else {
            // Continuation: only meaningful once a unit has started.
            if self.started {
                self.buf.extend_from_slice(payload);
            }
            None
        }
    }

    /// Take the final buffered PES at end of stream, if any.
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

    #[test]
    fn ignores_continuation_before_first_start() {
        let mut a = PesAssembler::new();
        assert_eq!(a.feed(false, &[0xDE, 0xAD]), None); // mid-stream join
        assert!(a.flush().is_none());
    }
}
