//! CRC-32 MPEG-2 — Annex C of ETSI EN 300 468, Annex A of ETSI TS 102 773.
//!
//! Polynomial `0x04C1_1DB7`, initial shift-register value `0xFFFF_FFFF`,
//! MSB-first bit order, no reflection, no final XOR. Used by every PSI/SI
//! section trailer and every T2-MI packet trailer.

/// CRC-32 MPEG-2 generator polynomial.
pub const POLY: u32 = 0x04C1_1DB7;

/// Precomputed 256-entry forward table, built at compile time — zero
/// runtime initialisation cost.
pub(crate) const TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0u32;
    while i < 256 {
        let mut c = i << 24;
        let mut j = 0;
        while j < 8 {
            c = if c & 0x8000_0000 != 0 {
                (c << 1) ^ POLY
            } else {
                c << 1
            };
            j += 1;
        }
        t[i as usize] = c;
        i += 1;
    }
    t
};

/// Compute CRC-32 MPEG-2 over `bytes`. Initial shift-register value is the
/// canonical `0xFFFF_FFFF`.
#[inline]
pub fn compute(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in bytes {
        crc = (crc << 8) ^ TABLE[((crc >> 24) as u8 ^ b) as usize];
    }
    crc
}
