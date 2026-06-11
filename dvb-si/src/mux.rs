//! Section → TS packetizer (the byte-exact inverse of
//! [`SectionReassembler::feed`](crate::ts::SectionReassembler::feed)).
//!
//! Per the PSI carriage rules of ISO/IEC 13818-1:2007 §2.4.4
//! (`docs/iso_13818_1_systems.md`): sections are packed into 188-byte packets
//! with a `pointer_field` where sections begin, concatenated contiguously, and
//! 0xFF-stuffed at the batch tail.

use crate::ts::{TsHeader, TS_PACKET_SIZE};

/// Maximum data bytes in a PUSI=1 packet (188 − 4 header − 1 pointer_field). §2.4.4.
const PUSI_PAYLOAD_CAP: usize = 183;
/// Maximum data bytes in a continuation packet (188 − 4 header). §2.4.4.
const PAYLOAD_CAP: usize = 184;
/// Stuffing byte for unused TS payload bytes (ISO/IEC 13818-1 §2.4.4).
const STUFFING_BYTE: u8 = 0xFF;

/// Packetizes PSI/SI sections into 188-byte TS packets.
///
/// This is the byte-exact inverse of
/// [`SectionReassembler::feed`](crate::ts::SectionReassembler::feed): packets
/// produced here, when fed back through the reassembler, yield the same
/// sections in order.
///
/// ISO/IEC 13818-1:2007 §2.4.4 (`docs/iso_13818_1_systems.md`).
pub struct SectionPacketizer {
    pid: u16,
    continuity_counter: u8,
}

impl SectionPacketizer {
    /// Start a packetizer for `pid` with continuity_counter = 0.
    pub fn new(pid: u16) -> Self {
        Self {
            pid,
            continuity_counter: 0,
        }
    }

    /// Start at a specific continuity_counter (0..=15) — for resuming a stream.
    pub fn with_continuity(pid: u16, cc: u8) -> Self {
        Self {
            pid,
            continuity_counter: cc & 0x0F,
        }
    }

    /// The PID this packetizer emits packets for.
    pub fn pid(&self) -> u16 {
        self.pid
    }

    /// The continuity_counter for the next emitted packet.
    pub fn continuity_counter(&self) -> u8 {
        self.continuity_counter
    }

    /// Packetize a batch of complete sections into 188-byte TS packets,
    /// appended to `out` (cleared first).
    ///
    /// Returns the number of packets appended.
    pub fn packetize_into(
        &mut self,
        sections: &[&[u8]],
        out: &mut Vec<[u8; TS_PACKET_SIZE]>,
    ) -> usize {
        out.clear();

        if sections.is_empty() {
            return 0;
        }

        // Concatenate all sections and record section-start byte offsets.
        let total_len: usize = sections.iter().map(|s| s.len()).sum();
        if total_len == 0 {
            return 0;
        }
        let mut data = Vec::with_capacity(total_len);
        let mut starts = Vec::with_capacity(sections.len());
        for s in sections {
            starts.push(data.len());
            data.extend_from_slice(s);
        }

        let count_before = out.len();
        let mut pos = 0usize;

        while pos < data.len() {
            // Smallest section-start offset ≥ pos.
            let next_start = starts.iter().copied().find(|&s| s >= pos);

            let pusi: bool;
            let pointer_field: u8;
            let cap: usize;

            if let Some(ns) = next_start {
                let diff = ns.saturating_sub(pos);
                if diff <= PUSI_PAYLOAD_CAP {
                    pusi = true;
                    pointer_field = diff as u8;
                    cap = PUSI_PAYLOAD_CAP;
                } else {
                    pusi = false;
                    pointer_field = 0;
                    cap = PAYLOAD_CAP;
                }
            } else {
                pusi = false;
                pointer_field = 0;
                cap = PAYLOAD_CAP;
            }

            let mut pkt = [0u8; TS_PACKET_SIZE];

            let header = TsHeader {
                tei: false,
                pusi,
                pid: self.pid,
                scrambling: 0,
                has_adaptation: false,
                has_payload: true,
                continuity_counter: self.continuity_counter,
            };
            header
                .serialize_into(&mut pkt[..4])
                .expect("4-byte header buffer");

            self.continuity_counter = (self.continuity_counter + 1) & 0x0F;

            let mut write_pos = 4usize;

            if pusi {
                pkt[write_pos] = pointer_field;
                write_pos += 1;
            }

            let remaining = data.len() - pos;
            let to_copy = remaining.min(cap);
            pkt[write_pos..write_pos + to_copy].copy_from_slice(&data[pos..pos + to_copy]);
            pos += to_copy;
            write_pos += to_copy;

            // 0xFF-stuff remaining payload bytes.
            for b in &mut pkt[write_pos..] {
                *b = STUFFING_BYTE;
            }

            out.push(pkt);
        }

        out.len() - count_before
    }

    /// Allocating convenience wrapper over [`packetize_into`](Self::packetize_into).
    pub fn packetize(&mut self, sections: &[&[u8]]) -> Vec<[u8; TS_PACKET_SIZE]> {
        let mut out = Vec::new();
        self.packetize_into(sections, &mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ts::{SectionReassembler, TsPacket};

    /// Build a long-form section with the given table_id and body bytes.
    /// Returns the full section including its 3-byte header (no CRC — the
    /// reassembler does not validate CRC).
    fn build_section(table_id: u8, body_after_length: &[u8]) -> Vec<u8> {
        let section_length = body_after_length.len() as u16;
        let mut v = Vec::with_capacity(3 + section_length as usize);
        v.push(table_id);
        // SSI=1, PI=0, reserved=11, length upper 4 bits
        v.push(0xB0 | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(body_after_length);
        v
    }

    /// Round-trip `sections` through packetize → reassembler, asserting
    /// byte-identical output in order and no leftovers.
    fn assert_round_trip(sections: &[Vec<u8>]) {
        let mut packetizer = SectionPacketizer::new(0x0100);
        let refs: Vec<&[u8]> = sections.iter().map(|s| s.as_slice()).collect();
        let packets = packetizer.packetize(&refs);

        let mut reasm = SectionReassembler::default();
        for pkt_raw in &packets {
            let pkt = TsPacket::parse(pkt_raw).expect("parse generated packet");
            let payload = pkt.payload.expect("payload present");
            let pusi = pkt.header.pusi;
            reasm.feed(payload, pusi);
        }

        let got: Vec<_> = std::iter::from_fn(|| reasm.pop_section()).collect();
        assert_eq!(
            got.len(),
            sections.len(),
            "section count mismatch: expected {}, got {}",
            sections.len(),
            got.len()
        );
        for (i, (orig, round)) in sections.iter().zip(got.iter()).enumerate() {
            assert_eq!(
                round.as_ref(),
                orig.as_slice(),
                "section {i} round-trip mismatch"
            );
        }
        assert!(reasm.is_empty(), "reassembler should be empty after drain");
    }

    // ── round-trip property (the mandatory acceptance oracle) ────────────────

    #[test]
    fn round_trip_single_short_section() {
        let s = build_section(0x42, &[0xAA; 10]);
        assert_round_trip(&[s]);
    }

    #[test]
    fn round_trip_one_byte_body() {
        let s = build_section(0x46, &[0xBB]); // 4 bytes total
        assert_round_trip(&[s]);
    }

    #[test]
    fn round_trip_section_exactly_pusi_cap_boundary() {
        // A section whose total length is exactly PUSI_PAYLOAD_CAP (183).
        let body = vec![0xCC; PUSI_PAYLOAD_CAP - 3];
        let s = build_section(0x50, &body);
        assert_eq!(s.len(), PUSI_PAYLOAD_CAP);
        assert_round_trip(&[s]);
    }

    #[test]
    fn round_trip_section_just_over_pusi_cap() {
        // One byte more than fits in a PUSI packet → must span to continuation.
        let body = vec![0xDD; PUSI_PAYLOAD_CAP - 3 + 1];
        let s = build_section(0x52, &body);
        assert_eq!(s.len(), PUSI_PAYLOAD_CAP + 1);
        assert_round_trip(&[s]);
    }

    #[test]
    fn round_trip_section_spans_many_packets() {
        // A 2000-byte section spans ~11 packets.
        let body = vec![0xEE; 2000 - 3];
        let s = build_section(0x60, &body);
        assert_round_trip(&[s]);
    }

    #[test]
    fn round_trip_section_near_reassembler_max() {
        // The largest section that round-trips through the reassembler's buffer
        // guard (which checks `buf.len() + payload.len() ≤ 4098` before each
        // continuation extension).  With a full first PUSI packet (183 bytes)
        // and full continuations (184 bytes each), the maximal total that fits
        // without the last continuation overflowing the guard is:
        //   183 + 20·184 + 184 = 4047  (§2.4.4)
        let body = vec![0x11; 4044]; // 3-header + 4044 body = 4047
        let s = build_section(0x80, &body);
        assert_eq!(s.len(), 4047);
        assert_round_trip(&[s]);
    }

    #[test]
    fn round_trip_multiple_short_sections_in_one_batch() {
        let s1 = build_section(0x42, &[0x01, 0x02]); // 5 bytes
        let s2 = build_section(0x46, &[0x03]); // 4 bytes
        let s3 = build_section(0x4A, &[0x04, 0x05, 0x06]); // 6 bytes
        assert_round_trip(&[s1, s2, s3]);
    }

    #[test]
    fn round_trip_section_ends_exactly_at_boundary() {
        // First section is exactly PUSI_PAYLOAD_CAP bytes — ends at packet
        // boundary.  Second section starts fresh in the next packet with
        // PUSI=1, pointer_field=0.
        let body1 = vec![0xA1; PUSI_PAYLOAD_CAP - 3];
        let s1 = build_section(0x50, &body1);
        assert_eq!(s1.len(), PUSI_PAYLOAD_CAP);

        let s2 = build_section(0x52, &[0xB1, 0xB2]);
        assert_round_trip(&[s1, s2]);
    }

    #[test]
    fn round_trip_mix_small_large_sections() {
        // Mix of small and spanning sections that stress pointer_field and
        // concatenation.
        let s1 = build_section(0x10, &[0xAA; 5]);
        let body2 = vec![0xBB; 200];
        let s2 = build_section(0x20, &body2);
        let s3 = build_section(0x30, &[0xCC; 50]);
        let body4 = vec![0xDD; 800];
        let s4 = build_section(0x40, &body4);
        let s5 = build_section(0x50, &[0xEE]); // 1-byte body
        assert_round_trip(&[s1, s2, s3, s4, s5]);
    }

    // ── continuity counter ───────────────────────────────────────────────────

    #[test]
    fn continuity_counter_increments_per_packet() {
        // Use a section large enough to span several packets.
        let body = vec![0xAA; 500];
        let section = build_section(0x42, &body);
        let mut p = SectionPacketizer::new(0x0100);

        let packets = p.packetize(&[&section]);
        assert!(packets.len() >= 3, "need multiple packets to test CC");

        let mut last_cc: Option<u8> = None;
        for pkt_raw in &packets {
            let pkt = TsPacket::parse(pkt_raw).unwrap();
            let cc = pkt.header.continuity_counter;
            if let Some(last) = last_cc {
                assert_eq!(cc, (last + 1) & 0x0F, "CC must increment per packet");
            }
            last_cc = Some(cc);
        }
    }

    #[test]
    fn continuity_counter_wraps_and_continues_across_calls() {
        let mut p = SectionPacketizer::with_continuity(0x0100, 14);
        // Section large enough to span at least 3 packets.
        let body = vec![0xBB; 500];
        let s = build_section(0x42, &body);

        // First call: CC 14, 15, 0, …
        let pkts1 = p.packetize(&[&s]);
        assert!(pkts1.len() >= 3, "section must span ≥3 packets");
        let ccs1: Vec<u8> = pkts1
            .iter()
            .map(|b| TsPacket::parse(b).unwrap().header.continuity_counter)
            .collect();
        assert_eq!(ccs1[0], 14);
        assert_eq!(ccs1[1], 15);
        assert_eq!(ccs1[2], 0);

        // Second call: CC continues from where first left off.
        let pkts2 = p.packetize(&[&s]);
        let cc_first_pkt2 = TsPacket::parse(&pkts2[0])
            .unwrap()
            .header
            .continuity_counter;
        assert_eq!(cc_first_pkt2, ccs1.last().map(|c| (c + 1) & 0x0F).unwrap());
    }

    // ── PUSI placement ──────────────────────────────────────────────────────

    #[test]
    fn pusi_set_when_section_starts() {
        let s = build_section(0x42, &[0xAA; 10]);
        let mut p = SectionPacketizer::new(0x0100);
        let packets = p.packetize(&[&s]);
        assert!(!packets.is_empty());
        let pkt = TsPacket::parse(&packets[0]).unwrap();
        assert!(pkt.header.pusi, "first packet must have PUSI=1");
    }

    #[test]
    fn pusi_not_set_on_mid_section_continuation() {
        let body = vec![0xAA; 500];
        let s = build_section(0x42, &body);
        let mut p = SectionPacketizer::new(0x0100);
        let packets = p.packetize(&[&s]);
        assert!(packets.len() >= 2);
        let pkt1 = TsPacket::parse(&packets[0]).unwrap();
        let pkt2 = TsPacket::parse(&packets[1]).unwrap();
        assert!(pkt1.header.pusi, "first packet must have PUSI=1");
        assert!(
            !pkt2.header.pusi,
            "second packet is continuation, must have PUSI=0"
        );
    }

    #[test]
    fn pointer_field_equals_tail_length_before_new_section() {
        // Section1 = 200 bytes.  Section2 = 50 bytes.
        // Packet 1: PUSI=1, pointer=0, section1 head.
        // Packet 2: PUSI=1, pointer > 0 (tail of section1 before section2).
        let body1 = vec![0xA1; 197]; // 200-byte section
        let s1 = build_section(0x52, &body1);
        assert_eq!(s1.len(), 200);
        let s2 = build_section(0x54, &[0xB1; 47]); // 50-byte section
        assert_eq!(s2.len(), 50);

        let mut p = SectionPacketizer::new(0x0100);
        let packets = p.packetize(&[&s1, &s2]);

        // Find the packet where PUSI=1 and pointer>0.
        let pkt_with_pointer = packets
            .iter()
            .map(|raw| TsPacket::parse(raw).unwrap())
            .find(|pkt| pkt.header.pusi && pkt.payload.is_some_and(|pl| pl.first() != Some(&0)))
            .expect("must have a PUSI packet with non-zero pointer");

        let payload = pkt_with_pointer.payload.unwrap();
        let pointer = payload[0] as usize;
        assert!(pointer > 0, "pointer must be non-zero");
        // The tail bytes should be from the end of section1.
        let tail_start = s1.len() - pointer;
        assert_eq!(&payload[1..1 + pointer], &s1[tail_start..]);
    }

    // ── stuffing ─────────────────────────────────────────────────────────────

    #[test]
    fn final_packet_unused_tail_is_stuffing() {
        let s = build_section(0x42, &[0xAA; 5]); // 8 bytes total
        let mut p = SectionPacketizer::new(0x0100);
        let packets = p.packetize(&[&s]);

        let pkt = TsPacket::parse(&packets[0]).unwrap();
        let payload = pkt.payload.unwrap();
        assert_eq!(payload[0], 0, "pointer_field should be 0");

        let section_end = 1 + s.len(); // after pointer + section
        assert!(
            section_end < payload.len(),
            "must have stuffing after section"
        );
        for &b in &payload[section_end..] {
            assert_eq!(b, STUFFING_BYTE, "all trailing bytes must be 0xFF");
        }
    }

    #[test]
    fn reassembler_discards_stuffing() {
        let s1 = build_section(0x42, &[0xAA; 10]);
        let s2 = build_section(0x46, &[0xBB; 5]);

        let mut p = SectionPacketizer::new(0x0100);
        let packets = p.packetize(&[&s1, &s2]);

        let mut reasm = SectionReassembler::default();
        for pkt_raw in &packets {
            let pkt = TsPacket::parse(pkt_raw).unwrap();
            reasm.feed(pkt.payload.unwrap(), pkt.header.pusi);
        }

        let got: Vec<_> = std::iter::from_fn(|| reasm.pop_section()).collect();
        assert_eq!(got.len(), 2);
        assert!(
            reasm.is_empty(),
            "stuffing tail must be discarded, not buffered"
        );
    }

    // ── misc ─────────────────────────────────────────────────────────────────

    #[test]
    fn empty_batch_produces_no_packets() {
        let mut p = SectionPacketizer::new(0x0100);
        let packets: Vec<[u8; TS_PACKET_SIZE]> = p.packetize(&[]);
        assert!(packets.is_empty());
    }

    #[test]
    fn packetize_into_clears_out_first() {
        let s = build_section(0x42, &[0xAA; 5]);
        let mut p = SectionPacketizer::new(0x0100);

        let mut out = vec![[0u8; TS_PACKET_SIZE]; 99]; // pre-existing junk
        let n = p.packetize_into(&[&s], &mut out);
        assert_eq!(n, out.len(), "out must contain only the new packets");
        // Verify the output is correct (round-trip).
        let mut reasm = SectionReassembler::default();
        for pkt_raw in &out {
            let pkt = TsPacket::parse(pkt_raw).unwrap();
            reasm.feed(pkt.payload.unwrap(), pkt.header.pusi);
        }
        let got = reasm.pop_section().unwrap();
        assert_eq!(got.as_ref(), s.as_slice());
    }

    #[test]
    fn pid_is_correct() {
        let p = SectionPacketizer::new(0x1234);
        assert_eq!(p.pid(), 0x1234);
    }

    #[test]
    fn with_continuity_masks_to_4_bits() {
        let p = SectionPacketizer::with_continuity(0x0100, 0xFE);
        assert_eq!(p.continuity_counter(), 0x0E);
    }

    #[test]
    fn has_payload_always_true_no_adaptation() {
        let s = build_section(0x42, &[0xAA; 50]);
        let mut p = SectionPacketizer::new(0x0100);
        let packets = p.packetize(&[&s]);
        for pkt_raw in &packets {
            let pkt = TsPacket::parse(pkt_raw).unwrap();
            assert!(pkt.header.has_payload, "every packet must carry payload");
            assert!(!pkt.header.has_adaptation, "no adaptation field is emitted");
            assert!(!pkt.header.tei, "TEI must be false");
            assert_eq!(pkt.header.scrambling, 0, "scrambling must be 0");
        }
    }
}
