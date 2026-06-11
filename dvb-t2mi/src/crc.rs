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
/// preceding payload. Returns `Error::BufferTooShort` if `bytes.len() < CRC_LEN`.
/// Returns `Error::CrcMismatch { computed, expected }` on mismatch.
pub fn validate_crc(bytes: &[u8]) -> Result<()> {
    if bytes.len() < CRC_LEN {
        return Err(Error::BufferTooShort {
            need: CRC_LEN,
            have: bytes.len(),
            what: "T2-MI CRC",
        });
    }
    let (payload, trailer) = bytes.split_at(bytes.len() - CRC_LEN);
    let expected = u32::from_be_bytes(trailer.try_into().unwrap());
    let computed = crc32_mpeg2::compute(payload);
    if computed == expected {
        Ok(())
    } else {
        Err(Error::CrcMismatch { computed, expected })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvb_common::crc32_mpeg2::compute;

    #[test]
    fn crc32_of_empty_input_is_initial_state() {
        assert_eq!(compute(&[]), 0xFFFF_FFFF);
    }

    #[test]
    fn compute_deterministic_and_not_identity() {
        let c = compute(&[0x00]);
        assert_eq!(c, compute(&[0x00]));
        assert_ne!(c, 0xFFFF_FFFF);
    }

    #[test]
    fn compute_different_inputs_produce_different_results() {
        assert_ne!(compute(&[0x00]), compute(&[0x01]));
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
