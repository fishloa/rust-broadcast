//! TS packet helpers — scramble/descramble 188-byte MPEG-2 TS packets.
//!
//! Handles adaptation-field offset and transport_scrambling_control bits
//! per ISO/IEC 13818-1 §2.4.3.3/§2.4.3.4 (ETSI TS 100 289 scrambles the
//! payload located this way, but does not itself redefine TS framing).
use crate::csa;
use crate::error::Error;
use crate::key::ControlWord;
use mpeg_ts::ts::{SCRAMBLING_MASK, TS_PACKET_SIZE, TsHeader};

/// `transport_scrambling_control` value for "scrambled with the even control
/// word" (ISO/IEC 13818-1 §2.4.3.3 Table 2-4: bits `[7:6]` of byte 3 = `10`).
const TSC_EVEN_KEY: u8 = 0x80;

/// One byte: the `adaptation_field_length` field itself, immediately after
/// the 4-byte TS header when `adaptation_field_control` signals a present
/// adaptation field (ISO/IEC 13818-1 §2.4.3.4).
const ADAPTATION_FIELD_LENGTH_SIZE: usize = 1;

/// Scramble the payload of a 188-byte TS packet in-place.
///
/// Skips the 4-byte header and any adaptation field bytes.
/// Sets the transport_scrambling_control bits to `10` (even key).
pub fn scramble_ts_packet(
    cw: &ControlWord,
    packet: &mut [u8; TS_PACKET_SIZE],
) -> Result<(), Error> {
    let payload = ts_payload_mut(packet)?;
    csa::scramble(cw, payload);
    packet[3] = (packet[3] & !SCRAMBLING_MASK) | TSC_EVEN_KEY;
    Ok(())
}

/// Descramble the payload of a 188-byte TS packet in-place.
///
/// Skips the 4-byte header and any adaptation field bytes.
/// Clears the transport_scrambling_control bits to `00`.
pub fn descramble_ts_packet(
    cw: &ControlWord,
    packet: &mut [u8; TS_PACKET_SIZE],
) -> Result<(), Error> {
    let payload = ts_payload_mut(packet)?;
    csa::descramble(cw, payload);
    packet[3] &= !SCRAMBLING_MASK;
    Ok(())
}

/// Get a mutable slice to the TS packet payload, skipping the header and
/// adaptation field.
///
/// The adaptation_field_control decode reuses `mpeg_ts::ts::TsHeader::parse`
/// (ISO/IEC 13818-1 §2.4.3.3) rather than re-deriving the same bit masks a
/// second time — an overrun/bit-order fix in that parser now reaches this
/// crate too. `TsHeader::parse` does not itself locate the payload byte
/// offset (that also needs the `adaptation_field_length` byte, §2.4.3.4,
/// which is not a header field), and `mpeg_ts::ts::TsPacket` — which does
/// compute that offset — only exposes an **immutable** `payload: &[u8]`
/// borrowed from its input, with no in-place-mutable equivalent. CSA
/// (de)scrambling must write the descrambled bytes back into the caller's
/// own buffer, so this function still computes the offset and takes the
/// `&mut` slice itself rather than depending on a mutable view `mpeg-ts`
/// does not provide.
fn ts_payload_mut(packet: &mut [u8; TS_PACKET_SIZE]) -> Result<&mut [u8], Error> {
    let header = TsHeader::parse(&packet[..TsHeader::serialized_len()])
        .expect("packet[..TsHeader::serialized_len()] is always exactly 4 bytes");

    if !header.has_payload {
        return Err(Error::BufferTooShort { need: 1, have: 0 });
    }

    let mut payload_start = TsHeader::serialized_len();

    if header.has_adaptation {
        let af_len = packet[payload_start] as usize; // adaptation_field_length, §2.4.3.4
        payload_start += ADAPTATION_FIELD_LENGTH_SIZE + af_len;
    }

    if payload_start >= TS_PACKET_SIZE {
        return Err(Error::BufferTooShort { need: 1, have: 0 });
    }

    Ok(&mut packet[payload_start..TS_PACKET_SIZE])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_ts_packet() {
        let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        // Build a minimal TS packet: header 0x47, PID 0x0100, no adaptation, payload
        let mut packet = [0u8; 188];
        packet[0] = 0x47; // sync byte
        packet[1] = 0x41; // TEI=0, PUSI=1, priority=0, PID high=0x0100>>8
        packet[2] = 0x00; // PID low=0x00
        packet[3] = 0x10; // no scrambling, adaptation_field_control=01 (payload only), CC=0
        // Fill payload with non-zero data (need at least 8 bytes)
        for i in 0..184 {
            packet[4 + i] = (i % 256) as u8;
        }

        let original = packet;
        scramble_ts_packet(&cw, &mut packet).unwrap();
        assert_ne!(packet[4..], original[4..]);
        assert_eq!(packet[3] & 0xc0, 0x80); // scrambling bits set

        descramble_ts_packet(&cw, &mut packet).unwrap();
        // Compare payload only (skip header)
        assert_eq!(packet[4..], original[4..]);
        assert_eq!(packet[3] & 0xc0, 0x00); // scrambling bits cleared
    }
}
