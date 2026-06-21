//! SNDU → MPEG-2 TS packet mapping and reassembly (RFC 4326 §3, §4.3, §6, §7).
//!
//! A ULE Encapsulator maps SNDUs into the 184-byte payload of MPEG-2 TS
//! packets on a single PID (TS Logical Channel). An SNDU may be carried whole,
//! fragmented across several packets, or packed two-or-more-to-a-packet. This
//! module provides:
//!
//! - [`UleReceiver`] — a de-fragmenting reassembler. Feed it the 184-byte
//!   payload of each TS packet on the ULE PID (with that packet's PUSI flag)
//!   and it yields each complete SNDU's bytes, validated to length and ready to
//!   hand to [`crate::Sndu::parse`].
//!
//! The receiver follows the §7 reassembly rules: it idles until a PUSI=1
//! packet, locates the first SNDU via the 1-byte Payload Pointer (§6.1),
//! accumulates bytes across PUSI=0 continuations, and stops a packet's walk at
//! an End Indicator / 0xFF padding (§4.3).

use alloc::vec::Vec;

use crate::sndu::{is_end_indicator, BASE_HEADER_LEN, END_INDICATOR_LENGTH, PADDING_BYTE};

/// The number of TS-packet payload bytes when AFC = `01` (payload only): the
/// 188-byte packet minus its 4-byte header (RFC 4326 §3).
pub const TS_PAYLOAD_LEN: usize = 184;

/// A de-fragmenting ULE receiver (RFC 4326 §7).
///
/// Stateful across TS packets on one PID. Hand it each packet's payload via
/// [`UleReceiver::push`]; it returns the complete SNDUs that finished in that
/// packet. The receiver owns a reassembly buffer for the partial SNDU spanning
/// packet boundaries.
#[derive(Debug, Default, Clone)]
pub struct UleReceiver {
    /// Bytes of an SNDU accumulated so far (header-first); empty when idle.
    partial: Vec<u8>,
    /// Total expected SNDU length once known (header + Length region), else 0.
    expected: usize,
    /// `true` once we have seen a valid SNDU start (not in the Idle State).
    started: bool,
}

impl UleReceiver {
    /// Create an idle receiver.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset to the Idle State (e.g. after a CC discontinuity, §7.3).
    pub fn reset(&mut self) {
        self.partial.clear();
        self.expected = 0;
        self.started = false;
    }

    /// `true` if the receiver is mid-SNDU (a fragment is buffered).
    pub fn in_reassembly(&self) -> bool {
        self.started && !self.partial.is_empty()
    }

    /// Feed one TS packet's payload (`payload` = the bytes after the 4-byte TS
    /// header, length [`TS_PAYLOAD_LEN`] in practice) and its `pusi` flag.
    /// Returns every SNDU that completed within this packet, as owned byte
    /// vectors (header..CRC inclusive).
    ///
    /// On a malformed Payload Pointer or an inconsistent length the partial
    /// SNDU is dropped and the receiver re-enters the Idle State (§7), but the
    /// already-completed SNDUs from this packet are still returned.
    pub fn push(&mut self, payload: &[u8], pusi: bool) -> Vec<Vec<u8>> {
        let mut out = Vec::new();

        if pusi {
            // PUSI=1: a 1-byte Payload Pointer follows the TS header. It counts
            // the bytes (excluding itself) up to the first new SNDU start.
            if payload.is_empty() {
                return out;
            }
            let pp = payload[0] as usize;
            let pp_region = &payload[1..];
            if pp > pp_region.len() {
                // Bad PP: discard any partial and idle.
                self.reset();
                return out;
            }
            // The first `pp` bytes complete a continuing SNDU (if any).
            if self.in_reassembly() {
                self.feed_continuation(&pp_region[..pp], &mut out);
            } else {
                // Idle: bytes before the pointer belong to no SNDU; skip them.
                self.partial.clear();
                self.expected = 0;
            }
            self.started = true;
            // Walk packed SNDUs starting at the pointer.
            self.walk_new_sndus(&pp_region[pp..], &mut out);
        } else {
            // PUSI=0: pure continuation of the SNDU in progress.
            if self.in_reassembly() {
                self.feed_continuation(payload, &mut out);
            }
            // else: a continuation with nothing in progress — discard (idle).
        }
        out
    }

    /// Append continuation bytes to the partial SNDU, emitting it if it
    /// completes. `chunk` is consumed fully (a continuation never starts a new
    /// SNDU — packing only happens at a PUSI=1 pointer or right after a
    /// completed SNDU within the same packet, handled by `walk_new_sndus`).
    fn feed_continuation(&mut self, chunk: &[u8], out: &mut Vec<Vec<u8>>) {
        self.partial.extend_from_slice(chunk);
        self.maybe_finish(out);
    }

    /// Walk a region that begins at an SNDU start (a packing region): parse the
    /// Length, consume whole SNDUs, and buffer the trailing partial for the
    /// next packet. Stops at an End Indicator or 0xFF padding (§4.3).
    fn walk_new_sndus(&mut self, mut region: &[u8], out: &mut Vec<Vec<u8>>) {
        loop {
            if region.is_empty() {
                return;
            }
            // End Indicator / padding: no more SNDUs in this packet.
            if region[0] == PADDING_BYTE {
                // Either a 0xFFFF End Indicator or stray 0xFF stuffing.
                if is_end_indicator(region) {
                    // remainder is padding; nothing buffered.
                }
                return;
            }
            if region.len() < BASE_HEADER_LEN {
                // Header straddles the packet boundary — buffer it.
                self.partial.clear();
                self.partial.extend_from_slice(region);
                self.expected = 0;
                return;
            }
            let first = u16::from_be_bytes([region[0], region[1]]);
            let length = (first & 0x7FFF) as usize;
            if (first & 0x7FFF) == END_INDICATOR_LENGTH && (first & 0x8000) != 0 {
                // Explicit End Indicator caught even if not 0xFF-leading.
                return;
            }
            let total = BASE_HEADER_LEN + length;
            if region.len() >= total {
                // A whole SNDU fits — emit it and continue packing.
                out.push(region[..total].to_vec());
                region = &region[total..];
            } else {
                // SNDU continues into the next packet — buffer the head.
                self.partial.clear();
                self.partial.extend_from_slice(region);
                self.expected = total;
                return;
            }
        }
    }

    /// If the buffered partial now holds a complete SNDU, emit it and clear.
    fn maybe_finish(&mut self, out: &mut Vec<Vec<u8>>) {
        if self.expected == 0 && self.partial.len() >= BASE_HEADER_LEN {
            // We had buffered only a fragment of the header; now compute length.
            let first = u16::from_be_bytes([self.partial[0], self.partial[1]]);
            let length = (first & 0x7FFF) as usize;
            self.expected = BASE_HEADER_LEN + length;
        }
        if self.expected != 0 && self.partial.len() >= self.expected {
            let total = self.expected;
            out.push(self.partial[..total].to_vec());
            let rest: Vec<u8> = self.partial[total..].to_vec();
            self.partial.clear();
            self.expected = 0;
            // Any bytes past the completed SNDU are a packed follow-on SNDU.
            if !rest.is_empty() {
                self.walk_new_sndus(&rest, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sndu::Sndu;
    use crate::type_field::TypeField;

    fn make_sndu(pdu: &[u8]) -> Vec<u8> {
        let s = Sndu::new(TypeField::EtherType(0x0800), None, pdu);
        let mut b = alloc::vec![0u8; s.serialized_len()];
        s.serialize_into(&mut b).unwrap();
        b
    }

    // An SNDU fragmented across two TS packet payloads reassembles correctly.
    #[test]
    fn fragmented_across_two_packets() {
        let pdu: Vec<u8> = (0u8..40).collect();
        let sndu = make_sndu(&pdu);
        assert!(sndu.len() > 20);

        let mut rx = UleReceiver::new();
        // Packet 1: PUSI=1, PP=0, carries the first 20 SNDU bytes.
        let mut p1 = alloc::vec![0x00u8];
        p1.extend_from_slice(&sndu[..20]);
        let done = rx.push(&p1, true);
        assert!(done.is_empty(), "still reassembling");
        assert!(rx.in_reassembly());

        // Packet 2: PUSI=0 continuation, rest of the SNDU + padding.
        let mut p2 = sndu[20..].to_vec();
        p2.extend_from_slice(&[0xFF, 0xFF]);
        let done = rx.push(&p2, false);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0], sndu);
        // And it parses.
        assert_eq!(Sndu::parse(&done[0]).unwrap().pdu(), &pdu[..]);
    }

    // Two SNDUs packed into one TS packet are both extracted.
    #[test]
    fn two_packed_sndus_one_packet() {
        let a = make_sndu(&[0xAA; 5]);
        let b = make_sndu(&[0xBB; 7]);

        let mut payload = alloc::vec![0x00u8]; // PP = 0
        payload.extend_from_slice(&a);
        payload.extend_from_slice(&b);
        payload.extend_from_slice(&[0xFF, 0xFF, 0xFF]); // End Indicator + pad

        let mut rx = UleReceiver::new();
        let done = rx.push(&payload, true);
        assert_eq!(done.len(), 2, "both packed SNDUs extracted");
        assert_eq!(done[0], a);
        assert_eq!(done[1], b);
    }

    // A continuing SNDU completes mid-packet, then a packed SNDU starts (PP>0).
    #[test]
    fn continuation_then_packed_with_pp() {
        let a = make_sndu(&[0x11; 30]); // will be fragmented
        let b = make_sndu(&[0x22; 4]); // packed after a completes

        let mut rx = UleReceiver::new();
        // Packet 1: PUSI=1 PP=0, first 15 bytes of `a`.
        let mut p1 = alloc::vec![0x00u8];
        p1.extend_from_slice(&a[..15]);
        assert!(rx.push(&p1, true).is_empty());

        // Packet 2: PUSI=1, PP = (rest of a) so the pointer lands on `b`.
        let rest_a = a.len() - 15;
        let mut p2 = alloc::vec![rest_a as u8];
        p2.extend_from_slice(&a[15..]); // completes a
        p2.extend_from_slice(&b); // packed b
        p2.push(0xFF);
        let done = rx.push(&p2, true);
        assert_eq!(done.len(), 2);
        assert_eq!(done[0], a);
        assert_eq!(done[1], b);
    }

    #[test]
    fn idle_until_pusi() {
        let mut rx = UleReceiver::new();
        // A continuation arriving while idle is discarded.
        let done = rx.push(&[0x01, 0x02, 0x03], false);
        assert!(done.is_empty());
        assert!(!rx.in_reassembly());
    }

    #[test]
    fn bad_payload_pointer_resets() {
        let mut rx = UleReceiver::new();
        // PP claims 200 but only a few bytes follow.
        let done = rx.push(&[200u8, 0x00, 0x10, 0x08, 0x00], true);
        assert!(done.is_empty());
        assert!(!rx.in_reassembly());
    }
}
