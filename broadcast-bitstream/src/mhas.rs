//! MPEG-H Audio Stream (MHAS) packet framing — ISO/IEC 23008-3 Clause 14,
//! ETSI TS 101 154 §6.8.3.
//!
//! Used only to locate the `PACTYP_MPEGH3DACFG` packet whose payload is the
//! opaque `mpegh3daConfig()` blob already carried by
//! `isobmff::mpegh::MHADecoderConfigurationRecord`. The `mhaC` box itself
//! lives in `isobmff::mpegh`.

use alloc::vec::Vec;

// ---------------------------------------------------------------------------
// MHAS packet framing (MPEG-2 TS carriage — issue #579)
// ---------------------------------------------------------------------------
//
// MPEG-H Audio elementary streams in MPEG-2 TS are formatted as the MPEG-H
// Audio Stream (MHAS), ISO/IEC 23008-3 Clause 14 — ETSI TS 101 154 §6.8.3
// ("MHAS elementary stream formatting") and ATSC A/342-3 §5.2.1 both cite it
// but neither vendored spec transcribes the packet's byte-level framing
// (Clause 14 itself is ISO/IEC 23008-3, paid, not vendored). This crate never
// decodes the MPEG-H audio bitstream, so the framing below is used for
// exactly one purpose: **locating** the `PACTYP_MPEGH3DACFG` packet whose
// payload is the opaque `mpegh3daConfig()` blob (already copied through
// unchanged by `isobmff::mpegh::MHADecoderConfigurationRecord`), and flagging an access
// unit that carries one as a random-access point.
//
// The packet-type numbering and the three-tier "escaped value" header coding
// are **empirically verified**, not printed in either vendored spec: every
// access unit of the vendored Fraunhofer MPEG-H-in-TS fixture
// (`private/fixtures/audio/mpegh/mpegh-cicp-01-baseline-32kbps-single.ts`) parses end-to-end under
// this scheme with no truncation/overrun, and the two random-access access
// units decode to exactly the packet order ETSI TS 101 154 §6.8.4.1
// mandates for a RAP (`PACTYP_MPEGH3DACFG`, then — directly following, per
// §6.8.4.1 — `PACTYP_AUDIOSCENEINFO`, `PACTYP_BUFFERINFO`,
// `PACTYP_MPEGH3DAFRAME`). The recovered `mpegh3daConfig()`'s leading byte
// also independently agrees with the PMT `MPEG-H_3dAudio_descriptor`'s
// `mpegh3daProfileLevelIndication` byte in the same fixture (both `0x10` —
// BL Profile Level 1), cross-confirming the packet boundaries are correct.
// See `private/specs-verified/mpeg/transmux/codec/mpegh-ts-101154.md` for the full derivation.

/// `MHASPacketType` for the config packet carrying `mpegh3daConfig()`
/// (empirically verified — see the module section docs above).
const MHAS_PACTYP_MPEGH3DACFG: u8 = 1;

/// Bit widths `(n1, n2, n3)` for the three-tier "escaped value" header
/// coding: read `n1` bits; if all-ones, read `n2` more and add; if *those*
/// are all-ones too, read `n3` more and add. Chosen (empirically confirmed)
/// so every representable `MHASPacketType`/`Label`/`Length` combination
/// leaves the following payload byte-aligned, matching ATSC A/342-3 §4.2.3's
/// description that "any MHAS packet payload always is byte-aligned".
const MHAS_TYPE_BITS: (u32, u32, u32) = (3, 8, 8);
const MHAS_LABEL_BITS: (u32, u32, u32) = (2, 8, 32);
const MHAS_LENGTH_BITS: (u32, u32, u32) = (11, 24, 24);

/// A big-endian, MSB-first bit cursor over a byte slice.
///
/// Thin wrapper over [`broadcast_core::bits::BitReader`] — the actual bit
/// extraction lives there (shared with `dvb-t2mi`/`rdd29`/`st291`; an overrun
/// or bit-order fix there now reaches this reader too), so this type only
/// adds the `Option`-returning shape the MHAS walker below wants, plus
/// `byte_pos`/`seek_to_byte`.
struct BitReader<'a> {
    inner: broadcast_core::bits::BitReader<'a>,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            inner: broadcast_core::bits::BitReader::new(data),
        }
    }

    fn bits_left(&self) -> usize {
        self.inner.bits_remaining()
    }

    /// Read `n` bits (`n <= 64`), MSB first. `None` if fewer than `n` bits remain.
    fn read(&mut self, n: u32) -> Option<u64> {
        self.inner.read_bits(n).ok()
    }

    /// Jump forward to `byte_offset` (never backward — every call site here
    /// only ever advances past a just-located, already-consumed header).
    fn seek_to_byte(&mut self, byte_offset: usize) {
        let target_bits = byte_offset * u8::BITS as usize;
        let current_bits = self.inner.bits_read();
        if target_bits > current_bits {
            let _ = self.inner.skip_bits(target_bits - current_bits);
        }
    }

    /// The current position as a byte offset, if bit-aligned to a byte.
    fn byte_pos(&self) -> Option<usize> {
        self.inner
            .is_byte_aligned()
            .then_some(self.inner.bits_read() / u8::BITS as usize)
    }
}

/// Decode one three-tier "escaped value" field (see [`MHAS_TYPE_BITS`] etc.).
fn escaped_value(br: &mut BitReader<'_>, bits: (u32, u32, u32)) -> Option<u64> {
    let (n1, n2, n3) = bits;
    let mut value = br.read(n1)?;
    if value == (1u64 << n1) - 1 {
        let v2 = br.read(n2)?;
        value += v2;
        if v2 == (1u64 << n2) - 1 {
            value += br.read(n3)?;
        }
    }
    Some(value)
}

/// One MHAS packet's type and payload, walked from an MHAS elementary-stream
/// access unit (`MHASPacketLabel` is discarded — not needed for config/RAP
/// recovery).
#[non_exhaustive]
pub struct MhasPacket<'a> {
    packet_type: u8,
    payload: &'a [u8],
}

impl<'a> MhasPacket<'a> {
    /// `MHASPacketType` — ISO/IEC 23008-3 Clause 14 Table 6.
    pub fn packet_type(&self) -> u8 {
        self.packet_type
    }

    /// The packet's payload bytes.
    pub fn payload(&self) -> &'a [u8] {
        self.payload
    }
}

/// Walk the MHAS packets in one access unit. Stops (returning whatever was
/// found so far) at the first malformed/truncated header instead of
/// erroring: this framing is used only to *locate* the
/// `PACTYP_MPEGH3DACFG` payload, never to reject a stream — an
/// unparseable/unknown-future packet layout simply yields no more packets,
/// and the access unit's bytes are still carried through opaquely as the
/// `Sample`.
pub fn walk_mhas_packets(data: &[u8]) -> Vec<MhasPacket<'_>> {
    let mut br = BitReader::new(data);
    let mut out = Vec::new();
    // A packet's minimal header (unescaped type + label + length) is 16 bits.
    const MIN_HEADER_BITS: usize = 16;
    while br.bits_left() >= MIN_HEADER_BITS {
        let Some(packet_type) = escaped_value(&mut br, MHAS_TYPE_BITS) else {
            break;
        };
        let Some(_label) = escaped_value(&mut br, MHAS_LABEL_BITS) else {
            break;
        };
        let Some(length) = escaped_value(&mut br, MHAS_LENGTH_BITS) else {
            break;
        };
        let Some(start) = br.byte_pos() else {
            break; // never byte-aligned in a well-formed MHAS stream
        };
        if packet_type > u64::from(u8::MAX) {
            break;
        }
        let Some(end) = start.checked_add(length as usize) else {
            break;
        };
        if end > data.len() {
            break;
        }
        out.push(MhasPacket {
            packet_type: packet_type as u8,
            payload: &data[start..end],
        });
        br.seek_to_byte(end);
    }
    out
}

/// Find the `mpegh3daConfig()` blob — the `PACTYP_MPEGH3DACFG` packet's
/// payload — in one MHAS access unit, if present. An access unit carrying
/// one is a random-access point (ETSI TS 101 154 §6.8.4.1).
pub fn find_mpegh3da_config(data: &[u8]) -> Option<&[u8]> {
    walk_mhas_packets(data)
        .into_iter()
        .find(|p| p.packet_type == MHAS_PACTYP_MPEGH3DACFG)
        .map(|p| p.payload)
}

#[cfg(test)]
mod mhas_tests {
    use super::*;

    /// Hand-built two-packet MHAS buffer using only unescaped (base) header
    /// widths: a 1-byte `PACTYP_SYNC`-style packet (type=6, label=0, len=1,
    /// payload `0xA5`) followed by a `PACTYP_MPEGH3DACFG`-style packet
    /// (type=1, label=1, len=3, payload `0x10 0x11 0x12`).
    fn build_packet(packet_type: u8, label: u8, payload: &[u8]) -> Vec<u8> {
        // header: 3 bits type + 2 bits label + 11 bits length = 16 bits (2 bytes).
        let len = payload.len() as u16;
        assert!(packet_type < 7 && label < 3 && len < 0x7FF);
        let word: u32 = ((packet_type as u32) << 13) | ((label as u32) << 11) | len as u32;
        let mut out = alloc::vec![(word >> 8) as u8, (word & 0xFF) as u8];
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn walks_unescaped_packets() {
        let mut data = build_packet(6, 0, &[0xA5]);
        data.extend(build_packet(
            MHAS_PACTYP_MPEGH3DACFG,
            1,
            &[0x10, 0x11, 0x12],
        ));
        let packets = walk_mhas_packets(&data);
        assert_eq!(packets.len(), 2);
        assert_eq!(packets[0].packet_type, 6);
        assert_eq!(packets[0].payload, &[0xA5]);
        assert_eq!(packets[1].packet_type, MHAS_PACTYP_MPEGH3DACFG);
        assert_eq!(packets[1].payload, &[0x10, 0x11, 0x12]);
    }

    #[test]
    fn finds_config_packet() {
        let mut data = build_packet(6, 0, &[0xA5]);
        data.extend(build_packet(MHAS_PACTYP_MPEGH3DACFG, 1, &[0xAA, 0xBB]));
        assert_eq!(find_mpegh3da_config(&data), Some([0xAA, 0xBB].as_slice()));
    }

    #[test]
    fn no_config_packet_returns_none() {
        let data = build_packet(6, 0, &[0xA5]);
        assert_eq!(find_mpegh3da_config(&data), None);
    }

    #[test]
    fn truncated_header_stops_cleanly() {
        // A single stray byte: not enough bits for even the minimal header.
        let data = [0xFFu8];
        assert!(walk_mhas_packets(&data).is_empty());
    }

    #[test]
    fn declared_length_past_end_stops_cleanly() {
        // type=1(MPEGH3DACFG) label=0 len=5, but only 1 payload byte follows.
        let word: u32 = (1u32 << 13) | 5;
        let data = [(word >> 8) as u8, (word & 0xFF) as u8, 0xAA];
        assert!(walk_mhas_packets(&data).is_empty());
    }

    /// Pack an exact MSB-first header bit string (the base-width helper cannot
    /// author an escaped field).
    fn mhas_header(bits: &str) -> Vec<u8> {
        assert_eq!(bits.len() % 8, 0, "header must be byte aligned");
        let mut out = Vec::with_capacity(bits.len() / 8);
        for chunk in bits.as_bytes().chunks(8) {
            let mut byte = 0u8;
            for &c in chunk {
                byte = (byte << 1) | u8::from(c == b'1');
            }
            out.push(byte);
        }
        out
    }

    /// #3800: an escaped length in its second tier (`v2 != all-ones`) is
    /// `2047 + v2`; a payload longer than the base 11-bit range is the normal
    /// case for a large `mpegh3daConfig`.
    #[test]
    fn escaped_length_tier_two_frames_the_payload() {
        // type 1 (3 bits, not escaped), label 0 (2 bits), length 11 ones + a
        // 24-bit 5 = 2052.
        let mut bits = alloc::string::String::from("001");
        bits.push_str("00");
        bits.push_str("11111111111");
        bits.push_str("000000000000000000000101");
        let payload = alloc::vec![0x5Au8; 2052];
        let mut data = mhas_header(&bits);
        assert_eq!(data.len(), 5);
        data.extend_from_slice(&payload);
        let packets = walk_mhas_packets(&data);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].packet_type, 1);
        assert_eq!(packets[0].payload, &payload[..]);
    }

    /// #3800: a length escaped into the third tier is the sum of all three
    /// fields. A run of all-ones tiers names a length far past the buffer and
    /// must frame nothing (an accumulator that subtracts instead would frame a
    /// 2047-byte packet out of the same bytes).
    #[test]
    fn escaped_length_tier_three_does_not_underflow() {
        let mut bits = alloc::string::String::from("001");
        bits.push_str("00");
        bits.push_str("11111111111");
        bits.push_str("111111111111111111111111");
        bits.push_str("111111111111111111111111");
        let mut data = mhas_header(&bits);
        assert_eq!(data.len(), 8);
        data.extend_from_slice(&alloc::vec![0u8; 2047]);
        assert!(walk_mhas_packets(&data).is_empty());
    }

    /// #3800: a third tier of zero must not multiply the accumulator to zero
    /// and frame an empty packet out of the same bytes.
    #[test]
    fn escaped_length_tier_three_zero_frames_nothing() {
        let mut bits = alloc::string::String::from("001");
        bits.push_str("00");
        bits.push_str("11111111111");
        bits.push_str("111111111111111111111111");
        bits.push_str("000000000000000000000000");
        let mut data = mhas_header(&bits);
        assert_eq!(data.len(), 8);
        data.push(0xA5);
        assert!(walk_mhas_packets(&data).is_empty());
    }

    /// #3800: the peer/third tier of an escaped label is followed correctly;
    /// reading a tier short would shift the length field into the label's own
    /// bytes and mis-frame the packet.
    #[test]
    fn escaped_label_tier_three_keeps_the_length_aligned() {
        // type 6 (3 bits, not escaped), label 2 ones + 8 ones + a 32-bit 0,
        // then length 1 (11 bits).
        let mut bits = alloc::string::String::from("110");
        bits.push_str("11");
        bits.push_str("11111111");
        bits.push_str("00000000000000000000000000000000");
        bits.push_str("00000000001");
        let mut data = mhas_header(&bits);
        assert_eq!(data.len(), 7);
        data.push(0xA5);
        let packets = walk_mhas_packets(&data);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].packet_type, 6);
        assert_eq!(packets[0].payload, &[0xA5]);
    }

    /// #3800: an escaped packet type of exactly 255 is legal; one that escapes
    /// to 256 cannot fit `u8` and must stop the walk.
    #[test]
    fn escaped_packet_type_is_bounded_by_u8() {
        let mut bits = alloc::string::String::from("111");
        bits.push_str("11111000"); // 248: 7 + 248 = 255
        bits.push_str("00");
        bits.push_str("00000000001");
        let mut data = mhas_header(&bits);
        data.push(0xA5);
        let packets = walk_mhas_packets(&data);
        assert_eq!(packets.len(), 1, "255 fits u8");
        assert_eq!(packets[0].packet_type, 255);
        assert_eq!(packets[0].payload, &[0xA5]);

        let mut bits = alloc::string::String::from("111");
        bits.push_str("11111001"); // 249: 7 + 249 = 256
        bits.push_str("00");
        bits.push_str("00000000001");
        let mut data = mhas_header(&bits);
        data.push(0xA5);
        assert!(walk_mhas_packets(&data).is_empty(), "256 does not fit u8");
    }
}
