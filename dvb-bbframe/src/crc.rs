//! CRC-8 encoder per EN 302 755 Annex F / EN 302 307-1 §5.1.4.
//!
//! Polynomial: g(X) = (X⁵+X⁴+X³+X²+1)·(X²+X+1)·(X+1)
//!                      = X⁸+X⁷+X⁶+X⁴+X²+1 = 0xD5

/// CRC-8 polynomial (0xD5), MSB-first, no reflection.
pub const CRC8_POLY: u8 = 0xD5;

/// Standard CRC-8 initial register value (0x00).
pub const CRC8_INIT: u8 = 0x00;

/// Compute CRC-8 with the standard initial value (0x00).
///
/// There is no separate "HEM init": EN 302 755 §5.1.7 puts
/// `crc8(header) XOR MODE` on the wire, so HEM (MODE=1) is the init-0 CRC
/// XOR 1. (A previous revision modelled HEM as init=0xB5 — that value is
/// init-0 propagated through exactly 9 zero bytes and only coincided for
/// 9-byte inputs; do not reintroduce it.)
#[inline]
pub fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = CRC8_INIT;
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
    fn crc8_known_dvb_t2_vector() {
        // Rai T2-MI (12606V, ISI 5, PLP 0). Live capture: wire byte is 0x1F.
        let hdr = [0xf8u8, 0x00, 0xa4, 0x28, 0xbc, 0xc8, 0xe2, 0x03, 0x50];
        assert_eq!(crc8(&hdr), 0x1E);
        // §5.1.7: wire byte = crc8 XOR MODE → 0x1E ^ 1 = 0x1F (HEM).
        assert_eq!(crc8(&hdr) ^ 0x01, 0x1F);
    }
}
