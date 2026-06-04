//! T2-MI CRC helpers.
//!
//! The CRC-32 MPEG-2 primitive lives in `dvb_common::crc32_mpeg2`.
//! This module keeps the T2-MI-packet-level validator, whose trailer
//! position semantics come from TS 102 773 §5.1 — a T2-MI-framing
//! concern, not a CRC primitive.

use crate::error::{Error, Result};
use dvb_common::crc32_mpeg2;

/// Length of the CRC-32 trailer in each T2-MI packet.
pub const CRC_LEN: usize = 4;

/// Validate a T2-MI packet's 4-byte CRC-32 trailer.
///
/// Returns `Ok(())` if the last 4 bytes match the computed CRC-32 of the
/// preceding payload. Returns `Error::Truncated` if `bytes.len() < CRC_LEN`.
/// Returns `Error::InvalidCrc { expected, computed }` on mismatch.
pub fn validate_crc(bytes: &[u8]) -> Result<()> {
    if bytes.len() < CRC_LEN {
        return Err(Error::Truncated);
    }
    let (payload, trailer) = bytes.split_at(bytes.len() - CRC_LEN);
    let expected = u32::from_be_bytes(trailer.try_into().unwrap());
    let computed = crc32_mpeg2::compute(payload);
    if computed == expected {
        Ok(())
    } else {
        Err(Error::InvalidCrc { expected, computed })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvb_common::crc32_mpeg2::{compute, TABLE};

    #[test]
    fn crc32_of_empty_input_is_initial_state() {
        assert_eq!(compute(&[]), 0xFFFF_FFFF);
    }

    #[test]
    fn table_first_entry_is_zero() {
        assert_eq!(TABLE[0], 0);
    }

    #[test]
    fn single_zero_byte_yields_index_ff() {
        let crc = compute(&[0x00]);
        let expected = TABLE[0xFF] ^ 0xFFFF_FF00;
        assert_eq!(crc, expected);
    }

    #[test]
    fn validate_crc_passes_for_known_packet() {
        let data = [0x01, 0x02, 0x03, 0x04];
        let c = compute(&data);
        let mut buf = Vec::with_capacity(data.len() + CRC_LEN);
        buf.extend_from_slice(&data);
        buf.extend_from_slice(&c.to_be_bytes());
        assert!(validate_crc(&buf).is_ok());
    }

    #[test]
    fn validate_crc_fails_for_corrupted_trailer() {
        let data = [0x01, 0x02, 0x03, 0x04];
        let c = compute(&data);
        let mut buf = Vec::with_capacity(data.len() + CRC_LEN);
        buf.extend_from_slice(&data);
        buf.extend_from_slice(&c.to_be_bytes());
        *buf.last_mut().unwrap() ^= 0xFF;
        assert!(validate_crc(&buf).is_err());
    }

    #[test]
    fn validate_crc_fails_for_corrupted_data() {
        let data = [0x01, 0x02, 0x03, 0x04];
        let c = compute(&data);
        let mut buf = Vec::with_capacity(data.len() + CRC_LEN);
        buf.extend_from_slice(&data);
        buf.extend_from_slice(&c.to_be_bytes());
        buf[0] ^= 0xFF;
        assert!(validate_crc(&buf).is_err());
    }

    #[test]
    fn validate_crc_rejects_too_short_input() {
        assert!(validate_crc(&[0x01, 0x02]).is_err());
    }
}
