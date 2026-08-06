//! TS packet helpers — scramble/descramble 188-byte MPEG-2 TS packets.
//!
//! Handles adaptation-field offset and transport_scrambling_control bits
//! per ETSI TS 100 289.
use crate::csa;
use crate::error::Error;
use crate::key::ControlWord;

/// Scramble the payload of a 188-byte TS packet in-place.
///
/// Skips the 4-byte header and any adaptation field bytes.
/// Sets the transport_scrambling_control bits to `10` (even key).
pub fn scramble_ts_packet(cw: &ControlWord, packet: &mut [u8; 188]) -> Result<(), Error> {
    let payload = ts_payload_mut(packet)?;
    csa::scramble(cw, payload);
    // Set transport_scrambling_control to 10 (scrambled, even key)
    packet[3] = (packet[3] & 0x3f) | 0x80;
    Ok(())
}

/// Descramble the payload of a 188-byte TS packet in-place.
///
/// Skips the 4-byte header and any adaptation field bytes.
/// Clears the transport_scrambling_control bits to `00`.
pub fn descramble_ts_packet(cw: &ControlWord, packet: &mut [u8; 188]) -> Result<(), Error> {
    let payload = ts_payload_mut(packet)?;
    csa::descramble(cw, payload);
    // Clear transport_scrambling_control
    packet[3] &= 0x3f;
    Ok(())
}

/// Get a mutable slice to the TS packet payload, skipping the header and
/// adaptation field.
fn ts_payload_mut(packet: &mut [u8; 188]) -> Result<&mut [u8], Error> {
    let adaptation_field_control = (packet[3] >> 4) & 0x03;
    let has_adaptation = adaptation_field_control & 0x02 != 0;
    let has_payload = adaptation_field_control & 0x01 != 0;

    if !has_payload {
        return Err(Error::BufferTooShort { need: 1, have: 0 });
    }

    let mut payload_start = 4; // after TS header

    if has_adaptation {
        let af_len = packet[4] as usize;
        payload_start += 1 + af_len; // adaptation_field_length byte + field
    }

    if payload_start >= 188 {
        return Err(Error::BufferTooShort { need: 1, have: 0 });
    }

    Ok(&mut packet[payload_start..188])
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
