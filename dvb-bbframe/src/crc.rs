//! CRC-8 encoder per EN 302 755 Annex F / EN 302 307-1 §5.1.4.
//!
//! Polynomial: g(X) = (X⁵+X⁴+X³+X²+1)·(X²+X+1)·(X+1)
//!                      = X⁸+X⁷+X⁶+X⁴+X²+1 = 0xD5

/// CRC-8 polynomial (0xD5), MSB-first, no reflection.
pub const CRC8_POLY: u8 = 0xD5;

/// Standard CRC-8 initial register value (DVB-S2 NM mode).
pub const CRC8_INIT: u8 = 0x00;

/// CRC-8 initial register value used by DVB-T2 BBFrames inside T2-MI streams.
///
/// T2-MI (ETSI TS 102 773 §5.2.1) carries DVB-T2 BBFrames. Empirically,
/// these use CRC-8 with init=0xB5 instead of the standard init=0x00.
pub const CRC8_INIT_DVB_T2: u8 = 0xB5;

/// Compute CRC-8 with the standard initial value (0x00).
#[inline]
pub fn crc8(bytes: &[u8]) -> u8 {
    crc8_with_init(bytes, CRC8_INIT)
}

/// Compute CRC-8 with a custom initial register value.
///
/// This is used for DVB-T2 (init = [`CRC8_INIT_DVB_T2`]) to detect HEM mode.
#[inline]
pub fn crc8_with_init(bytes: &[u8], init: u8) -> u8 {
    let mut crc = init;
    for &byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            if crc & 0x80 != 0 {
                crc = (crc << 1) ^ CRC8_POLY;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc8_of_all_zeros_is_init_value() {
        assert_eq!(crc8(&[0x00; 9]), CRC8_INIT);
    }

    #[test]
    fn crc8_with_init_b5_on_empty_is_init() {
        assert_eq!(crc8_with_init(&[], CRC8_INIT_DVB_T2), CRC8_INIT_DVB_T2);
    }

    #[test]
    fn crc8_known_dvb_t2_vector() {
        // Rai T2-MI (12606V, ISI 5, PLP 0).
        let hdr = [0xf8u8, 0x00, 0xa4, 0x28, 0xbc, 0xc8, 0xe2, 0x03, 0x50];
        assert_eq!(crc8_with_init(&hdr, CRC8_INIT_DVB_T2), 0x1F);
        // Standard init=0x00 gives 0x1E (off by one — the HEM detection basis).
        assert_eq!(crc8(&hdr), 0x1E);
    }
}
