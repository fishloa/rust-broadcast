//! CRC-32 MPEG-2 — Annex C of ETSI EN 300 468, Annex A of ETSI TS 102 773.
//!
//! Polynomial `0x04C1_1DB7`, initial shift-register value `0xFFFF_FFFF`,
//! MSB-first bit order, no reflection, no final XOR. Used by every PSI/SI
//! section trailer and every T2-MI packet trailer.

/// CRC-32 MPEG-2 generator polynomial.
pub const POLY: u32 = 0x04C1_1DB7;

/// Width of the lookup table.
const TABLE_ENTRIES: usize = 256;
/// Bit shifts of one table/compute step (MSB-first, no reflection).
const CRC_HI_SHIFT: u32 = 24;
const CRC_STEP_BITS: u32 = 8;
/// Mask testing the shift register's most-significant bit.
const CRC_MSB_MASK: u32 = 0x8000_0000;
/// Canonical initial shift-register value.
const CRC_INIT: u32 = 0xFFFF_FFFF;

/// Precomputed 256-entry forward table, built at compile time — zero
/// runtime initialisation cost.
pub(crate) const TABLE: [u32; TABLE_ENTRIES] = {
    let mut t = [0u32; TABLE_ENTRIES];
    let mut i = 0u32;
    while i < TABLE_ENTRIES as u32 {
        let mut c = i << CRC_HI_SHIFT;
        let mut j = 0;
        while j < u8::BITS {
            c = if c & CRC_MSB_MASK != 0 {
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
    let mut crc: u32 = CRC_INIT;
    for &b in bytes {
        crc = (crc << CRC_STEP_BITS) ^ TABLE[((crc >> CRC_HI_SHIFT) as u8 ^ b) as usize];
    }
    crc
}
