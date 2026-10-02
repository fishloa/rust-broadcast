//! PUSI-delimited payload reassembler — generic non-PSI PID payload accumulation.
//!
//! In MPEG-2 TS, a PID carrying packetised data that is **not** PSI section
//! data uses `payload_unit_start_indicator` (PUSI) to delimit units: the
//! packet whose PUSI == `1` starts a new unit. This is the mechanism used by
//! ISO/IEC 23009-1 §5.10.3.3.5 to carry a DASH `emsg` box on reserved PID
//! `0x0004` — the `emsg` box begins in the PUSI packet's payload, continues
//! across zero or more non-PUSI packets on the same PID, and its last packet
//! is padded with **adaptation-field stuffing** (not payload stuffing), so
//! the concatenation of the PID's payload bytes across a PUSI-delimited run
//! equals the box bytes exactly.
//!
//! [`PusiReassembler`] implements this generic reassembly: feed TS payload
//! bytes together with the PID and PUSI flag; it returns
//! `Option<Vec<u8>>` when a unit completes. This type is not `emsg`-specific
//! — it works for any PUSI-delimited non-PSI PID payload.

use alloc::vec::Vec;
use broadcast_common::pusi::PusiAccumulator;

/// Default maximum accumulated unit size (16 MiB), shared with `mpeg-pes`'s
/// `PesAssembler` through [`broadcast_common::pusi`] (audit r01-W13, #1074).
pub use broadcast_common::pusi::DEFAULT_MAX_UNIT_SIZE;

/// Generic PUSI-delimited payload reassembler: a PID filter over the shared
/// [`PusiAccumulator`].
///
/// Accumulates payload bytes across consecutive TS packets sharing the same
/// PID, using `payload_unit_start_indicator` to delimit unit boundaries.
///
/// # Example
///
/// ```ignore
/// let mut reassembler = PusiReassembler::new(0x0004);
///
/// // Feed pusi=true first packet
/// if let Some(unit) = reassembler.push(0x0004, true, &first_payload) {
///     // unit is complete (would only happen if a second PUSI follows)
/// }
///
/// // Feed continuation
/// reassembler.push(0x0004, false, &cont_payload);
///
/// // Drain the final unit
/// if let Some(unit) = reassembler.flush() {
///     // process complete unit
/// }
/// ```
#[derive(Debug, Clone)]
pub struct PusiReassembler {
    /// The PID we are listening to. Packets with a different PID are ignored.
    pid: u16,
    inner: PusiAccumulator,
}

impl PusiReassembler {
    /// Create a new reassembler for the given `pid` using the default maximum unit size.
    #[inline]
    pub fn new(pid: u16) -> Self {
        Self {
            pid,
            inner: PusiAccumulator::new(),
        }
    }

    /// Create a new reassembler with a custom maximum unit size.
    #[inline]
    pub fn with_max_unit_size(pid: u16, max: usize) -> Self {
        Self {
            pid,
            inner: PusiAccumulator::with_max_unit_size(max),
        }
    }

    /// Feed one TS packet's (pid, payload_unit_start_indicator, payload bytes).
    ///
    /// Packets whose `pid != self.pid` are silently ignored (returns `None`).
    ///
    /// **PUSI semantics**: When `pusi == true`, the packet marks the start of a
    /// *new* unit. If a unit was already in progress, it is **complete** — it
    /// is returned as `Some(unit_bytes)` (an empty unit is not returned), and
    /// a fresh unit begins with this packet's payload. When `pusi == false`,
    /// the payload is appended to the in-progress unit; payload before the
    /// first PUSI is ignored (ISO/IEC 13818-1 §2.4.3.2), and a unit past the
    /// size cap is discarded until the next PUSI.
    pub fn push(&mut self, pid: u16, pusi: bool, payload: &[u8]) -> Option<Vec<u8>> {
        if pid != self.pid {
            return None;
        }
        self.inner.feed(pusi, payload)
    }

    /// Return any in-progress (final) unit; the caller drains this at end of
    /// the PID's data (the last real box has no following PUSI to close it).
    ///
    /// Returns `None` when no data has been accumulated.
    pub fn flush(&mut self) -> Option<Vec<u8>> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// A synthetic "box" larger than a single TS payload (184 bytes).
    /// We use a valid big-endian size prefix so the test is realistic.
    const BIG_BOX_SIZE: u32 = 400;
    /// Build a synthetic box with a valid 4-byte big-endian size and 'test' type.
    fn synthetic_box_bytes(size: u32) -> Vec<u8> {
        let n = size as usize;
        let mut bytes = Vec::with_capacity(n);
        bytes.extend_from_slice(&size.to_be_bytes()); // size
        bytes.extend_from_slice(b"test"); // type
        // Fill the rest (starting at offset 8) with a counter pattern.
        for i in 8..n {
            bytes.push((i & 0xFF) as u8);
        }
        debug_assert_eq!(bytes.len(), n);
        bytes
    }

    #[test]
    fn spanning_across_two_packets() {
        let box_bytes = synthetic_box_bytes(BIG_BOX_SIZE);
        let pid = 0x0004u16;

        // Split at a boundary that fits in a single TS payload (≤ 184 bytes).
        let chunk1 = &box_bytes[..184];
        let chunk2 = &box_bytes[184..];

        let mut reasm = PusiReassembler::new(pid);

        // Push first chunk with PUSI = true.
        assert!(reasm.push(pid, true, chunk1).is_none());

        // Push second chunk (continuation).
        assert!(reasm.push(pid, false, chunk2).is_none());

        // Flush and verify.
        let reassembled = reasm.flush().expect("flush should return the unit");
        assert_eq!(reassembled, box_bytes);
    }

    #[test]
    fn boundary_two_units() {
        let unit_a = b"AAAA-first-unit-data";
        let unit_b = b"BBBB-second-unit-data";
        let pid = 0x0004u16;

        let mut reasm = PusiReassembler::new(pid);

        // Push unit A (PUSI start).
        assert!(reasm.push(pid, true, unit_a).is_none());

        // Push unit B (PUSI start) — this should close unit A and return it.
        let closed = reasm.push(pid, true, unit_b);
        assert_eq!(closed.as_deref(), Some(unit_a.as_slice()));

        // Flush should return unit B.
        let flushed = reasm.flush();
        assert_eq!(flushed.as_deref(), Some(unit_b.as_slice()));
    }

    #[test]
    fn different_pid_ignored() {
        let pid = 0x0004u16;
        let mut reasm = PusiReassembler::new(pid);

        // Packet with different PID should be ignored.
        assert!(reasm.push(0x0100, true, b"ignored").is_none());
        assert!(reasm.push(0x0100, false, b"more-ignored").is_none());
        assert!(reasm.flush().is_none());

        // Now feed a packet on our PID.
        assert!(reasm.push(pid, true, b"real-data").is_none());
        let flushed = reasm.flush();
        assert_eq!(flushed.as_deref(), Some(b"real-data".as_slice()));
    }

    #[test]
    fn flush_empty_returns_none() {
        let mut reasm = PusiReassembler::new(0x0004);
        assert!(reasm.flush().is_none());
    }

    #[test]
    fn single_packet_unit() {
        let pid = 0x0004u16;
        let mut reasm = PusiReassembler::new(pid);

        assert!(reasm.push(pid, true, b"single-packet-emsg").is_none());
        let flushed = reasm.flush();
        assert_eq!(flushed.as_deref(), Some(b"single-packet-emsg".as_slice()));
    }

    #[test]
    fn ignores_payload_before_first_pusi() {
        let pid = 0x0004u16;
        let mut reasm = PusiReassembler::new(pid);

        // Feed non-PUSI payloads before any PUSI — these should be ignored.
        assert!(reasm.push(pid, false, b"junk1").is_none());
        assert!(reasm.push(pid, false, b"junk2").is_none());

        // First PUSI starts a new unit.
        assert!(reasm.push(pid, true, b"real-data").is_none());

        let flushed = reasm.flush();
        // The flushed unit must be exactly "real-data", not "junk1junk2real-data".
        assert_eq!(flushed.as_deref(), Some(b"real-data".as_slice()));
    }

    #[test]
    fn caps_buffer_at_default_max_unit_size() {
        let pid = 0x0004u16;
        let mut reasm = PusiReassembler::new(pid);

        // Start with PUSI.
        assert!(reasm.push(pid, true, &[0u8; 10]).is_none());

        // Verify the default cap is DEFAULT_MAX_UNIT_SIZE.
        assert_eq!(reasm.inner.max_unit_size(), DEFAULT_MAX_UNIT_SIZE);

        // Flush and verify.
        let flushed = reasm.flush();
        assert_eq!(flushed.as_deref(), Some(&[0u8; 10][..]));
    }

    #[test]
    fn custom_cap_discards_oversized_unit() {
        let pid = 0x0004u16;
        let custom_cap = 1000usize;
        let mut reasm = PusiReassembler::with_max_unit_size(pid, custom_cap);

        // Start with PUSI.
        assert!(reasm.push(pid, true, &[0u8; 10]).is_none());

        // Try to feed enough non-PUSI continuation to exceed the custom cap.
        let chunk = vec![0x42u8; 600];
        reasm.push(pid, false, &chunk);

        // The second chunk should trigger the cap check and reset.
        reasm.push(pid, false, &chunk);

        // A new PUSI returns None (prior oversized unit was discarded).
        let completed = reasm.push(pid, true, b"new-unit");
        assert!(completed.is_none());

        // Flush the final unit.
        let flushed = reasm.flush();
        assert_eq!(flushed.as_deref(), Some(b"new-unit".as_slice()));
    }

    #[test]
    fn one_mib_unit_reassembles_with_default_cap() {
        let pid = 0x0004u16;
        let mut reasm = PusiReassembler::new(pid);
        let unit_size = 1024 * 1024usize;

        // Start with PUSI.
        assert!(reasm.push(pid, true, &[0u8; 10000]).is_none());

        // Feed 1 MiB in chunks, well within the default 16 MiB cap.
        let chunk = vec![0xAB; 256 * 1024];
        for _ in 0..3 {
            reasm.push(pid, false, &chunk);
        }

        // Flush and verify the unit is complete.
        let flushed = reasm.flush();
        assert!(flushed.is_some());
        assert!(flushed.as_ref().unwrap().len() > unit_size / 2);
    }
}
