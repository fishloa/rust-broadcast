//! MPEG-TS packet parser + section reassembler. Feature-gated under `ts`.

use crate::error::{Error, Result};

/// Size of one MPEG-TS packet (ETSI EN 300 468 §3.2, ISO/IEC 13818-1 §2.4.3.2).
pub const TS_PACKET_SIZE: usize = 188;
/// Sync byte that every TS packet starts with (ISO/IEC 13818-1 §2.4.3.2).
pub const TS_SYNC_BYTE: u8 = 0x47;
/// Upper bound on a single PSI/SI section — per ETSI EN 300 468 §5.1.1,
/// `section_length` is 12 bits so at most 4096 bytes inclusive of header.
const MAX_SECTION_SIZE: usize = 4096;

/// ETSI EN 300 468 §3.2.3: transport header byte 1 bits 7 = tei (Transport Error Indicator).
const TEI_MASK: u8 = 0x80;
/// ETSI EN 300 468 §3.2.3: byte 1 bits 6 = pusi (Payload Unit Start Indicator).
const PUSI_MASK: u8 = 0x40;
/// ETSI EN 300 468 §3.2.3: byte 1 bits 5..=1 = 13-bit PID (upper 5 bits).
pub const PID_MASK_HI: u8 = 0x1F;
/// ETSI EN 300 468 §3.2.3: byte 3 bits 7..=6 = 2-bit scrambling control.
pub const SCRAMBLING_MASK: u8 = 0xC0;
/// ETSI EN 300 468 §3.2.3: byte 3 bit 4 = adaptation_field_control (bit 4 = 1 means adaptation present).
pub const ADAPTATION_FLAG: u8 = 0x20;
/// ETSI EN 300 468 §3.2.3: byte 3 bit 3 = adaptation_field_control (bit 3 = 1 means payload present).
pub const PAYLOAD_FLAG: u8 = 0x10;
/// ETSI EN 300 468 §3.2.3: byte 3 bits 3..=0 = 4-bit continuity_counter.
pub const CC_MASK: u8 = 0x0F;

/// Parsed TS header — the 4-byte transport header fields.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TsHeader {
    /// Transport Error Indicator — set by the demodulator when an
    /// uncorrectable error is present in the packet.
    pub tei: bool,
    /// Payload Unit Start Indicator — first byte of the payload is a new
    /// PES packet or PSI section header when set.
    pub pusi: bool,
    /// 13-bit Packet Identifier.
    pub pid: u16,
    /// 2-bit transport_scrambling_control (0 = not scrambled).
    pub scrambling: u8,
    /// Adaptation field present flag (adaptation_field_control bit 1).
    pub has_adaptation: bool,
    /// Payload present flag (adaptation_field_control bit 0).
    pub has_payload: bool,
    /// 4-bit continuity_counter (wraps 0..=15 per PID).
    pub continuity_counter: u8,
}

/// Borrowed view into one 188-byte TS packet.
///
/// Serde: Serialize-only. Deserialize is omitted because `raw` is a
/// reference to fixed-length 188-byte storage that cannot be reconstructed
/// from a deserializer's lifetime budget; the field is also redundant once
/// the header has been parsed. `raw` is excluded from the serialized form.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TsPacket<'a> {
    /// Parsed header fields.
    pub header: TsHeader,
    /// Slice into the packet's payload, or `None` when `has_payload == false`
    /// or the adaptation field consumed the whole packet body.
    pub payload: Option<&'a [u8]>,
    /// The raw 188 bytes of the packet — kept for cheap forwarding.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub raw: &'a [u8; TS_PACKET_SIZE],
}

impl TsHeader {
    /// Parse a 4-byte TS transport header.
    ///
    /// Returns `None` if `raw4` is shorter than 4 bytes.
    pub fn parse(raw4: &[u8]) -> Option<Self> {
        if raw4.len() < 4 {
            return None;
        }
        let b1 = raw4[1];
        let b2 = raw4[2];
        let b3 = raw4[3];

        let tei = (b1 & TEI_MASK) != 0;
        let pusi = (b1 & PUSI_MASK) != 0;
        let pid = (((b1 & PID_MASK_HI) as u16) << 8) | (b2 as u16);
        let scrambling = (b3 & SCRAMBLING_MASK) >> 6;
        let has_adaptation = (b3 & ADAPTATION_FLAG) != 0;
        let has_payload = (b3 & PAYLOAD_FLAG) != 0;
        let continuity_counter = b3 & CC_MASK;

        Some(Self {
            tei,
            pusi,
            pid,
            scrambling,
            has_adaptation,
            has_payload,
            continuity_counter,
        })
    }

    /// Serialize this header into the first 4 bytes of `buf`.
    ///
    /// Panics if `buf` is shorter than 4 bytes.
    pub fn serialize_into(&self, buf: &mut [u8]) {
        assert!(buf.len() >= 4, "buffer must have at least 4 bytes for TS header");
        buf[0] = TS_SYNC_BYTE;
        buf[1] = 0;
        if self.tei {
            buf[1] |= TEI_MASK;
        }
        if self.pusi {
            buf[1] |= PUSI_MASK;
        }
        buf[1] |= ((self.pid >> 8) as u8) & PID_MASK_HI;
        buf[2] = (self.pid & 0xFF) as u8;
        buf[3] = (self.scrambling << 6) & SCRAMBLING_MASK;
        if self.has_adaptation {
            buf[3] |= ADAPTATION_FLAG;
        }
        if self.has_payload {
            buf[3] |= PAYLOAD_FLAG;
        }
        buf[3] |= self.continuity_counter & CC_MASK;
    }
}

impl<'a> TsPacket<'a> {
    /// Parse a single 188-byte TS packet from a buffer.
    ///
    /// Returns `Err(Error::InvalidSyncByte)` if the first byte is not `0x47`,
    /// `Err(Error::BufferTooShort)` if fewer than 188 bytes, or `Ok` with
    /// the parsed packet otherwise.
    pub fn parse(buf: &'a [u8]) -> Result<Self> {
        if buf.len() < TS_PACKET_SIZE {
            return Err(Error::BufferTooShort {
                need: TS_PACKET_SIZE,
                have: buf.len(),
                what: "TsPacket::parse",
            });
        }
        if buf[0] != TS_SYNC_BYTE {
            return Err(Error::InvalidSyncByte { found: buf[0] });
        }

        let raw: &[u8; TS_PACKET_SIZE] =
            buf[..TS_PACKET_SIZE]
                .try_into()
                .map_err(|_| Error::BufferTooShort {
                    need: TS_PACKET_SIZE,
                    have: buf.len(),
                    what: "TsPacket::parse (array conversion)",
                })?;

        let header = TsHeader::parse(&raw[..4])
            .expect("raw is 188 bytes so first 4 bytes are always present");

        let mut cursor = 4usize;
        let mut payload = None;

        // Skip adaptation field if present (not parsed in detail — not needed for sections).
        if header.has_adaptation && cursor < TS_PACKET_SIZE {
            let af_len = raw[cursor] as usize;
            cursor += 1 + af_len;
        }

        if header.has_payload && cursor < TS_PACKET_SIZE {
            payload = Some(&raw[cursor..]);
        }

        Ok(TsPacket {
            header,
            payload,
            raw,
        })
    }
}

/// Reassembles PSI/SI sections from TS packets on a single PID.
///
/// Feed each TS packet's payload with `feed`. Complete sections are
/// appended to an internal queue; drain them with `pop_section`.
#[derive(Default)]
pub struct SectionReassembler {
    buf: bytes::BytesMut,
    expected: usize,
    ready: std::collections::VecDeque<bytes::Bytes>,
}

impl SectionReassembler {
    /// Feed a TS payload and whether its packet had PUSI set.
    ///
    /// Extracts complete SI sections into the internal queue. A single call
    /// can produce zero or one section (the queue is for future-proofing
    /// where one feed might yield multiple sections).
    pub fn feed(&mut self, payload: &[u8], pusi: bool) {
        if pusi {
            let pointer = payload[0] as usize;
            let start = 1 + pointer;
            if start >= payload.len() {
                self.buf.clear();
                return;
            }
            self.buf.clear();
            let new_data = &payload[start..];
            if self.buf.len() + new_data.len() > MAX_SECTION_SIZE {
                self.buf.clear();
                self.expected = 0;
                return;
            }
            self.buf.extend_from_slice(new_data);
            if self.buf.len() >= 3 {
                self.expected = 3 + (((self.buf[1] & 0x0F) as usize) << 8 | self.buf[2] as usize);
            }
        } else {
            if self.buf.is_empty() {
                return;
            }
            if self.buf.len() + payload.len() > MAX_SECTION_SIZE {
                self.buf.clear();
                self.expected = 0;
                return;
            }
            self.buf.extend_from_slice(payload);
        }

        if self.expected > 0 && self.buf.len() >= self.expected {
            // split_to returns the first `expected` bytes as an owned BytesMut,
            // leaving the remaining bytes in self.buf — cheap (shifts pointers).
            let section = self.buf.split_to(self.expected).freeze();
            self.ready.push_back(section);
            self.expected = 0;
        }
    }

    /// Pop one complete section. Returns `None` when the queue is empty.
    pub fn pop_section(&mut self) -> Option<bytes::Bytes> {
        self.ready.pop_front()
    }

    /// Number of bytes currently buffered (incomplete section).
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// True if no bytes are currently buffered.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: construct a minimal 188-byte TS packet buffer with given header flags and payload.
    fn make_packet(b1: u8, b2: u8, b3: u8, payload_data: &[u8]) -> [u8; TS_PACKET_SIZE] {
        let mut pkt = [0u8; TS_PACKET_SIZE];
        pkt[0] = TS_SYNC_BYTE;
        pkt[1] = b1;
        pkt[2] = b2;
        pkt[3] = b3;
        let payload_start = 4;
        let end = (payload_start + payload_data.len()).min(TS_PACKET_SIZE);
        let len = (end - payload_start).min(payload_data.len());
        pkt[payload_start..payload_start + len].copy_from_slice(&payload_data[..len]);
        pkt
    }

    #[test]
    fn parse_rejects_non_0x47_sync_byte() {
        let mut pkt = [0u8; TS_PACKET_SIZE];
        pkt[0] = 0x46; // wrong sync byte
        let err = TsPacket::parse(&pkt).unwrap_err();
        match err {
            Error::InvalidSyncByte { found } => assert_eq!(found, 0x46),
            other => panic!("expected InvalidSyncByte, got {other:?}"),
        }
    }

    #[test]
    fn parse_extracts_pid_and_continuity_counter() {
        // PID = 0x1234 → upper 5 bits = 0x12, lower 8 bits = 0x34
        // CC = 5 → 0x05
        // b1 = 0x47 (sync=0, tei=0, pusi=0) | (0x12) = 0x47 & 0xE0 | 0x12 = 0x47 & 0xE0 = 0x40 | 0x12 = 0x52
        // Actually: b1 bits: [tei:1][pusi:1][pid_hi:5]
        // pid_hi = 0x12 = 0b00100_10 → bits 5..=1 = 0x12
        // b1 = 0b00_010010 = 0x12 (no tei, no pusi)
        let pkt = make_packet(0x12, 0x34, 0x05, &[]);
        let pkt = TsPacket::parse(&pkt).unwrap();
        assert_eq!(pkt.header.pid, 0x1234);
        assert_eq!(pkt.header.continuity_counter, 5);
    }

    #[test]
    fn payload_unit_start_indicator_flag_extracted() {
        // b1 = 0x40 → pusi = true (bit 6 set, no tei, no pid bits)
        let pkt1 = make_packet(0x40, 0x00, 0x00, &[]);
        let pkt1 = TsPacket::parse(&pkt1).unwrap();
        assert!(pkt1.header.pusi);

        // b1 = 0x00 → pusi = false
        let pkt2 = make_packet(0x00, 0x00, 0x00, &[]);
        let pkt2 = TsPacket::parse(&pkt2).unwrap();
        assert!(!pkt2.header.pusi);
    }

    /// Build a PSI-carrying TS payload: `pointer_field` byte followed by
    /// (optionally) some tail of a previous section, followed by a fresh
    /// section. `pointer_field` is the number of bytes of the previous
    /// section that precede the new one (per ETSI EN 300 468 §5.1.4).
    fn build_pusi_payload(pointer_field: u8, previous_tail: &[u8], section: &[u8]) -> Vec<u8> {
        assert_eq!(pointer_field as usize, previous_tail.len());
        let mut v = Vec::with_capacity(1 + previous_tail.len() + section.len());
        v.push(pointer_field);
        v.extend_from_slice(previous_tail);
        v.extend_from_slice(section);
        v
    }

    /// Build a long-form section with the given table_id and body bytes.
    /// Returns the full section including its 3-byte + 5-byte header and a
    /// placeholder CRC — for reassembler testing we don't validate the CRC.
    fn build_section(table_id: u8, body_after_length: &[u8]) -> Vec<u8> {
        let section_length = body_after_length.len() as u16;
        let mut v = Vec::with_capacity(3 + section_length as usize);
        v.push(table_id);
        // ssi=1, pi=0, reserved=11, length hi 4 bits
        v.push(0xB0 | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(body_after_length);
        v
    }

    // The reassembler tests below feed raw payload slices directly to
    // `feed()` rather than wrapping them in 188-byte TS packets. This avoids
    // the TS stuffing-byte tail (0xFF padding) bleeding into the reassembled
    // section and keeps the assertions exact.

    #[test]
    fn reassembler_accumulates_multi_packet_section() {
        // 200-byte section that spans two payload slices.
        let body = vec![0xAAu8; 197];
        let section = build_section(0x02, &body);
        assert_eq!(section.len(), 200);

        let first_chunk = 100;
        let payload1 = build_pusi_payload(0, &[], &section[..first_chunk]);
        let payload2 = section[first_chunk..].to_vec();

        let mut reasm = SectionReassembler::default();
        reasm.feed(&payload1, true);
        reasm.feed(&payload2, false);

        let out = reasm.pop_section().expect("section should be ready");
        assert_eq!(out.len(), 200);
        assert_eq!(out.as_ref(), &section[..]);
    }

    #[test]
    fn reassembler_yields_complete_section_once_length_satisfied() {
        // 1-byte-body section: table_id=0x42, section_length=1, total=4 bytes.
        let section = build_section(0x42, &[0xAA]);
        assert_eq!(section.len(), 4);
        let payload = build_pusi_payload(0, &[], &section);

        let mut reasm = SectionReassembler::default();
        reasm.feed(&payload, true);

        let out = reasm
            .pop_section()
            .expect("single-packet section should pop");
        assert_eq!(out.as_ref(), &section[..]);
    }

    #[test]
    fn reassembler_discards_on_buffer_overflow() {
        // Declare section_length larger than a single payload can carry. No
        // pop happens until continuations arrive; if continuations push the
        // buffer past MAX_SECTION_SIZE the reassembler must reset, not panic.
        let mut section = Vec::with_capacity(3 + 4095);
        section.push(0x00); // table_id
        section.push(0xB0 | ((4095u16 >> 8) as u8 & 0x0F));
        section.push(0xFF);
        section.extend_from_slice(&[0u8; 160]);
        let payload1 = build_pusi_payload(0, &[], &section);

        let mut reasm = SectionReassembler::default();
        reasm.feed(&payload1, true);
        assert!(reasm.pop_section().is_none());

        // Push enough continuation data to cross MAX_SECTION_SIZE.
        let filler = vec![0u8; 180];
        for _ in 0..(MAX_SECTION_SIZE / 180 + 1) {
            reasm.feed(&filler, false);
        }
        assert!(
            reasm.pop_section().is_none(),
            "no section should pop after overflow reset"
        );

        // State must be resettable — a fresh valid PUSI section works.
        let valid_section = build_section(0x00, &[0xAA]);
        let payload2 = build_pusi_payload(0, &[], &valid_section);
        reasm.feed(&payload2, true);
        let out = reasm
            .pop_section()
            .expect("fresh section should pop after reset");
        assert_eq!(out.as_ref(), &valid_section[..]);
    }

    #[test]
    fn reassembler_handles_pusi_with_nonzero_pointer_field() {
        // payload = pointer_field=3, 3 bytes of prior-section tail, then new section.
        let prior_tail = vec![0x11, 0x22, 0x33];
        let new_section = build_section(0x02, &[0xBB]);
        assert_eq!(new_section.len(), 4);
        let payload = build_pusi_payload(3, &prior_tail, &new_section);

        let mut reasm = SectionReassembler::default();
        reasm.feed(&payload, true);

        let out = reasm
            .pop_section()
            .expect("section after pointer_field skip should pop");
        assert_eq!(out.as_ref(), &new_section[..]);
    }

    #[test]
    fn reassembler_ignores_continuation_before_pusi() {
        // Feed a non-PUSI payload first (no prior PUSI seen).
        // SectionReassembler should discard it and stay empty.
        let pkt = make_packet(0x00, 0x00, PAYLOAD_FLAG, &[0xAA, 0xBB, 0xCC]);

        let mut reasm = SectionReassembler::default();
        reasm.feed(&pkt[4..], false); // no PUSI

        assert!(
            reasm.pop_section().is_none(),
            "no section should appear without prior PUSI"
        );
        assert!(
            reasm.pop_section().is_none(),
            "second pop should also be none"
        );
    }
}
