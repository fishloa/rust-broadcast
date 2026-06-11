//! TS packet reassembly utilities.
//!
//! Reconstructs T2-MI packets from MPEG-2 TS payloads per ETSI TS 102 773 §6.1.1.

use std::collections::VecDeque;

use crate::crc::CRC_LEN;

/// Per-PID T2-MI packet reassembler.
///
/// Accepts TS payload slices with PUSI state and emits complete T2-MI packets.
#[derive(Default)]
pub struct PacketReassembler {
    buf: bytes::BytesMut,
    synced: bool,
    pending: VecDeque<bytes::Bytes>,
}

/// Total bytes in a T2-MI header.
const HEADER_LEN: usize = 6;

impl PacketReassembler {
    /// Create a new empty reassembler.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a TS payload slice with its PUSI state.
    ///
    /// The reassembler is single-stream: the caller demultiplexes by PID
    /// (typically 0x0006 data piping, or whatever the PMT assigns) and feeds
    /// only the T2-MI PID's payloads — one `PacketReassembler` per PID.
    ///
    /// Per §6.1.1:
    /// - If PUSI is set, byte 0 is `pointer_field` indicating offset to next T2-MI packet.
    /// - T2-MI packets are packed back-to-back; a new one can start mid-payload.
    /// - If PUSI is clear, continuation bytes extend the current packet.
    pub fn feed(&mut self, payload: &[u8], pusi: bool) {
        if payload.is_empty() {
            return;
        }

        if pusi {
            let ptr = payload[0] as usize;

            // Bytes 1..boundary belong to the packet in progress (if synced).
            let boundary = 1 + ptr;
            if self.synced {
                let end = boundary.min(payload.len());
                if end > 1 {
                    self.buf.extend_from_slice(&payload[1..end]);
                }
                // Try to extract complete packets from accumulated buffer
                self.try_extract_packets();
            }

            // §6.1.1: pointer_field is authoritative — a new T2-MI packet
            // starts exactly at `boundary`. Anything still buffered belongs to
            // a packet that never completed (corrupt payload_len_bits or lost
            // TS packets); drop it so the corruption cannot swallow the good
            // packets that follow.
            self.buf.clear();

            if boundary <= payload.len() {
                if boundary < payload.len() {
                    self.buf.extend_from_slice(&payload[boundary..]);
                    self.try_extract_packets();
                }
                // boundary == payload.len(): the new packet starts at the very
                // end — zero bytes yet; continuation arrives on the next feed.
                self.synced = true;
            } else {
                // pointer_field points past the payload — malformed; wait for
                // the next PUSI to resync.
                self.synced = false;
            }
        } else if self.synced {
            // Continuation: all payload bytes extend current T2-MI packet
            self.buf.extend_from_slice(payload);
            self.try_extract_packets();
        }
        // !synced && !pusi: discard (waiting for first PUSI)
    }

    /// Attempt to extract one or more complete T2-MI packets from buf.
    fn try_extract_packets(&mut self) {
        loop {
            // Need at least header bytes to determine packet size
            if self.buf.len() < HEADER_LEN {
                break;
            }

            // Parse payload_len_bits from header (bytes 4-5, big-endian)
            let payload_len_bits = ((self.buf[4] as u16) << 8) | (self.buf[5] as u16);
            let payload_len_bytes = payload_len_bits.div_ceil(8);
            let total_packet_len = HEADER_LEN + payload_len_bytes as usize + CRC_LEN;

            if self.buf.len() < total_packet_len {
                break;
            }

            // Extract complete packet
            let packet_bytes = self.buf.split_to(total_packet_len);
            self.pending.push_back(packet_bytes.freeze());
        }
    }

    /// Drain the next completed T2-MI packet.
    pub fn pop_packet(&mut self) -> Option<bytes::Bytes> {
        self.pending.pop_front()
    }

    /// Drain all pending packets.
    pub fn drain_packets(&mut self) -> impl Iterator<Item = bytes::Bytes> + '_ {
        self.pending.drain(..)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_t2mi_packet(packet_type: u8, count: u8, payload: &[u8]) -> Vec<u8> {
        let mut pkt = Vec::with_capacity(6 + payload.len() + 4);
        pkt.push(packet_type);
        pkt.push(count);
        // superframe_idx=0, rfu=0, t2mi_stream_id=0
        let header_byte2 = 0x00;
        let header_byte3 = 0x00;
        pkt.push(header_byte2);
        pkt.push(header_byte3);
        let bits = (payload.len() * 8) as u16;
        pkt.extend_from_slice(&bits.to_be_bytes());
        pkt.extend_from_slice(payload);
        // CRC (zeros for reassembler tests — we don't validate here)
        pkt.extend_from_slice(&[0u8; 4]);
        pkt
    }

    #[test]
    fn reassembles_single_packet_with_pusi_offset_0() {
        let t2mi = make_t2mi_packet(0x00, 0, &[0xAA, 0xBB]);
        let mut reasm = PacketReassembler::new();
        // TS payload: pointer_field=0, then T2-MI packet
        let mut ts_payload = vec![0x00];
        ts_payload.extend_from_slice(&t2mi);
        reasm.feed(&ts_payload, true);
        let pkt = reasm.pop_packet().unwrap();
        assert_eq!(&pkt[..], &t2mi[..]);
    }

    #[test]
    fn reassembles_packet_spanning_two_ts_packets() {
        let t2mi = make_t2mi_packet(0x00, 0, &[0xCC; 200]);
        let mut reasm = PacketReassembler::new();

        // First TS: PUSI=1, pointer=0, first 100 bytes of T2-MI
        let mut ts1 = vec![0x00];
        ts1.extend_from_slice(&t2mi[..100]);
        reasm.feed(&ts1, true);
        assert!(reasm.pop_packet().is_none());

        // Second TS: !PUSI, remaining bytes
        reasm.feed(&t2mi[100..], false);
        let pkt = reasm.pop_packet().unwrap();
        assert_eq!(&pkt[..], &t2mi[..]);
    }

    #[test]
    fn reassembles_two_packets_in_one_ts_payload() {
        let t1 = make_t2mi_packet(0x00, 0, &[0x11]);
        let t2 = make_t2mi_packet(0x01, 1, &[0x22]);
        let mut reasm = PacketReassembler::new();

        let mut ts_payload = vec![0x00]; // pointer=0
        ts_payload.extend_from_slice(&t1);
        ts_payload.extend_from_slice(&t2);
        reasm.feed(&ts_payload, true);

        let p1 = reasm.pop_packet().unwrap();
        let p2 = reasm.pop_packet().unwrap();
        assert_eq!(&p1[..], &t1[..]);
        assert_eq!(&p2[..], &t2[..]);
    }

    #[test]
    fn handles_pusi_with_nonzero_pointer() {
        let t2mi = make_t2mi_packet(0x00, 5, &[0xDE]);
        let mut reasm = PacketReassembler::new();
        // TS payload: pointer=3, 3 bytes junk, then T2-MI packet
        let mut ts_payload = vec![0x03, 0xFF, 0xFF, 0xFF];
        ts_payload.extend_from_slice(&t2mi);
        reasm.feed(&ts_payload, true);
        let pkt = reasm.pop_packet().unwrap();
        assert_eq!(&pkt[..], &t2mi[..]);
    }

    #[test]
    fn discards_data_before_first_pusi() {
        let mut reasm = PacketReassembler::new();
        reasm.feed(&[0xAA, 0xBB], false); // !synced, !pusi → discard
        assert!(reasm.pop_packet().is_none());
    }

    #[test]
    fn handles_empty_payload() {
        let mut reasm = PacketReassembler::new();
        reasm.feed(&[], true);
        assert!(reasm.pop_packet().is_none());
    }

    /// §6.1.1: pointer_field is authoritative. A buffered partial whose
    /// declared length over-ran (corruption / lost TS packets) must be
    /// dropped at the next PUSI instead of swallowing the good packet that
    /// starts there.
    #[test]
    fn corrupt_length_resyncs_at_next_pusi() {
        let mut reasm = PacketReassembler::new();

        // A partial packet that claims a huge payload (8000 bits = 1000
        // bytes) but only ever delivers a few bytes.
        let mut corrupt = vec![0x00u8, 0x00, 0x00, 0x00];
        corrupt.extend_from_slice(&8000u16.to_be_bytes());
        corrupt.extend_from_slice(&[0xEE; 20]);
        let mut ts1 = vec![0x00]; // PUSI, pointer=0
        ts1.extend_from_slice(&corrupt);
        reasm.feed(&ts1, true);
        assert!(reasm.pop_packet().is_none());

        // Next PUSI with pointer=0: a clean, complete packet starts here.
        let good = make_t2mi_packet(0x00, 7, &[0xAB, 0xCD]);
        let mut ts2 = vec![0x00];
        ts2.extend_from_slice(&good);
        reasm.feed(&ts2, true);

        let pkt = reasm.pop_packet().expect("good packet must survive resync");
        assert_eq!(&pkt[..], &good[..]);
        assert!(reasm.pop_packet().is_none());
    }

    /// A pointer_field that points past the payload end is malformed — the
    /// reassembler must drop sync and recover on the following PUSI.
    #[test]
    fn pointer_past_payload_end_drops_sync() {
        let mut reasm = PacketReassembler::new();
        reasm.feed(&[0xFF, 0xAA, 0xBB], true); // ptr=255 > payload len
        assert!(reasm.pop_packet().is_none());
        // Continuation while unsynced is discarded.
        reasm.feed(&[0xCC; 8], false);
        assert!(reasm.pop_packet().is_none());
        // Clean PUSI recovers.
        let good = make_t2mi_packet(0x00, 1, &[0x55]);
        let mut ts = vec![0x00];
        ts.extend_from_slice(&good);
        reasm.feed(&ts, true);
        assert_eq!(&reasm.pop_packet().unwrap()[..], &good[..]);
    }

    #[test]
    fn drains_multiple_pending_packets() {
        let t1 = make_t2mi_packet(0x00, 0, &[0xAA]);
        let t2 = make_t2mi_packet(0x00, 1, &[0xBB]);
        let t3 = make_t2mi_packet(0x00, 2, &[0xCC]);
        let mut reasm = PacketReassembler::new();

        let mut ts_payload = vec![0x00];
        ts_payload.extend_from_slice(&t1);
        ts_payload.extend_from_slice(&t2);
        ts_payload.extend_from_slice(&t3);
        reasm.feed(&ts_payload, true);

        let packets: Vec<_> = reasm.drain_packets().collect();
        assert_eq!(packets.len(), 3);
        assert_eq!(&packets[0][..], &t1[..]);
        assert_eq!(&packets[1][..], &t2[..]);
        assert_eq!(&packets[2][..], &t3[..]);
    }
}
