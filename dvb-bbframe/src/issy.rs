//! ISSY (Input Stream SYnchronizer) field decoding per EN 302 755 §5.1.7 / Annex C.
//!
//! ISSY carries the Input Stream Clock Reference (ISCR) and, in its long form,
//! buffer-status / time-to-output signalling, used for jitter-free transport
//! reconstruction at the receiver. The first bit selects the form:
//!
//! ```text
//!   bit7 = 0          -> ISCR short: 15-bit ISCR    (2-byte ISSY)
//!   bit7 = 1, bit6 = 0 -> ISCR long: 22-bit ISCR    (3-byte ISSY)
//!   bit7 = 1, bit6 = 1 -> BUFS / TTO signalling      (3-byte ISSY)
//! ```

/// BUFS unit selector — EN 302 755 Annex C, Table C.1 (2-bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum BufsUnit {
    /// 0b00 — bits.
    Bits,
    /// 0b01 — Kbits.
    Kbits,
    /// 0b10 — Mbits.
    Mbits,
    /// 0b11 — 8 Kbits.
    Kbits8,
}

impl BufsUnit {
    #[must_use]
    /// Construct from a raw `u8` (only the low 2 bits are used).
    pub fn from_u8(v: u8) -> Self {
        match v & 0x03 {
            0 => Self::Bits,
            1 => Self::Kbits,
            2 => Self::Mbits,
            3 => Self::Kbits8,
            _ => unreachable!(),
        }
    }

    #[must_use]
    /// Return the wire byte for this unit.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Bits => 0,
            Self::Kbits => 1,
            Self::Mbits => 2,
            Self::Kbits8 => 3,
        }
    }

    #[must_use]
    /// Human-readable unit name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Bits => "bits",
            Self::Kbits => "Kbits",
            Self::Mbits => "Mbits",
            Self::Kbits8 => "8 Kbits",
        }
    }
}

/// Decoded BUFS/TTO signalling — EN 302 755 Annex C, Table C.1.
///
/// The `11` prefix in the first ISSY byte selects one of two alternatives:
/// BUFS (buffer status) or TTO (time-to-output), indicated by bits `[5:4]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum SignallingKind {
    /// BUFS — maximum size of the requested receiver buffer.
    ///
    /// Fields: `(bufs, units)` where `bufs` is the 10-bit buffer status
    /// and `units` is the 2-bit unit selector.
    Bufs {
        /// 10-bit buffer status value.
        bufs: u16,
        /// 2-bit unit selector (Table C.1).
        units: BufsUnit,
    },
    /// TTO — time-to-output (mantissa + exponent form).
    ///
    /// The output time is `TTO = (tto_m + tto_l / 256) * 2^tto_e`
    /// where `tto_l` is zero when ISCRshort is in use.
    Tto {
        /// 5-bit exponent `TTO_E`.
        tto_e: u8,
        /// 7-bit mantissa `TTO_M`.
        tto_m: u8,
        /// 8-bit low-fraction `TTO_L` (zero when ISCRshort is in use).
        tto_l: u8,
    },
    /// Reserved signalling type (bits `[5:4]` = `0b10` or `0b11`).
    ///
    /// The raw 20-bit remainder is preserved for lossless round-trip.
    Reserved(u32),
}

/// Decoded ISSY value (EN 302 755 §5.1.7, Annex C).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Issy {
    /// ISCR short form — 15-bit Input Stream Clock Reference (2-byte ISSY).
    IscrShort(u16),
    /// ISCR long form — 22-bit Input Stream Clock Reference (3-byte ISSY).
    IscrLong(u32),
    /// Long-form BUFS/TTO signalling (3-byte ISSY, `11` prefix).
    ///
    /// The 22-bit payload is decoded into [`SignallingKind`]; see
    /// Annex C for the sub-coding.
    Signalling(SignallingKind),
}

/// Decode a 2-byte (short) ISSY field.
///
/// Returns `Some(Issy::IscrShort)` when the short-form bit (bit 7 of byte 0) is
/// `0`; `None` otherwise (a `1` prefix means a long-form field, which is 3 bytes
/// and must be decoded with [`decode_issy_long`]).
#[must_use]
pub fn decode_issy_short(bytes: [u8; 2]) -> Option<Issy> {
    if bytes[0] & 0x80 != 0 {
        return None;
    }
    let iscr = ((bytes[0] as u16 & 0x7F) << 8) | bytes[1] as u16;
    Some(Issy::IscrShort(iscr))
}

/// Decode a 3-byte (long) ISSY field.
///
/// Byte 0 bit 7 must be `1` (long form). Byte 0 bit 6 then selects: `0` → 22-bit
/// ISCR long; `1` → BUFS/TTO signalling. Returns `None` if bit 7 is `0` (that is
/// a short-form field — use [`decode_issy_short`]).
#[must_use]
pub fn decode_issy_long(bytes: [u8; 3]) -> Option<Issy> {
    if bytes[0] & 0x80 == 0 {
        return None;
    }
    let payload = ((bytes[0] as u32 & 0x3F) << 16) | (bytes[1] as u32) << 8 | bytes[2] as u32;
    if bytes[0] & 0x40 == 0 {
        Some(Issy::IscrLong(payload)) // '10' prefix
    } else {
        Some(Issy::Signalling(decode_signalling(payload))) // '11' prefix
    }
}

/// Decode the 22-bit `11`-prefix payload per Annex C Table C.1.
///
/// Bits `[21:20]` select the signalling type:
/// - `0b00` → BUFS: bits `[19:18]` = unit, bits `[17:8]` = 10-bit BUFS, `[7:0]` reserved
/// - `0b01` → TTO: bits `[19:16]` = 4 MSBs of TTO_E, byte 1 bit 7 = LSB of TTO_E,
///   byte 1 bits `[6:0]` = TTO_M, byte 2 = TTO_L (or reserved for ISCRshort)
/// - `0b10`, `0b11` → reserved
fn decode_signalling(payload: u32) -> SignallingKind {
    let kind = (payload >> 20) & 0x03;
    match kind {
        0 => {
            let units = BufsUnit::from_u8(((payload >> 18) & 0x03) as u8);
            let bufs = ((payload >> 8) & 0x03FF) as u16;
            SignallingKind::Bufs { bufs, units }
        }
        1 => {
            let tto_e = (((payload >> 16) & 0x0F) << 1 | ((payload >> 15) & 0x01)) as u8;
            let tto_m = ((payload >> 8) & 0x7F) as u8;
            let tto_l = (payload & 0xFF) as u8;
            SignallingKind::Tto {
                tto_e,
                tto_m,
                tto_l,
            }
        }
        _ => {
            let remainder = payload & 0x0F_FFFF;
            SignallingKind::Reserved(remainder)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iscr_short_decodes_15_bits() {
        assert_eq!(
            decode_issy_short([0x7A, 0xBC]),
            Some(Issy::IscrShort(0x7ABC))
        );
        assert_eq!(decode_issy_short([0x00, 0x01]), Some(Issy::IscrShort(1)));
    }

    #[test]
    fn short_rejects_long_prefix() {
        assert_eq!(decode_issy_short([0x80, 0x00]), None);
    }

    #[test]
    fn iscr_long_decodes_22_bits() {
        assert_eq!(
            decode_issy_long([0xBF, 0xFF, 0xFF]),
            Some(Issy::IscrLong(0x3FFFFF))
        );
        assert_eq!(
            decode_issy_long([0x80, 0x12, 0x34]),
            Some(Issy::IscrLong(0x1234))
        );
    }

    #[test]
    fn signalling_decodes_with_11_prefix() {
        assert_eq!(
            decode_issy_long([0xC0, 0x12, 0x34]),
            Some(Issy::Signalling(decode_signalling(0x1234)))
        );
    }

    #[test]
    fn long_rejects_short_prefix() {
        assert_eq!(decode_issy_long([0x00, 0x00, 0x00]), None);
    }

    #[test]
    fn signalling_bufs_decode() {
        // '11' prefix, bits[21:20]=0b00 (BUFS)
        // bits[19:18]=0b10 (Mbits), bits[17:8]=0x1FF (BUFS=511), bits[7:0]=reserved
        // First byte: 0b11_00_10_11 = 0xCB
        // Second byte: 0xFF (BUFS low 8 bits)
        // Third byte: 0x00 (reserved)
        // payload = 0x0B_FF_00? No, let me recalculate.
        // payload bits [21:0] = 00_10_11111111_00000000
        // = 0b00_10_1111_1111_0000_0000 = 0x02FF00
        // byte0 & 0x3F = (0xCB & 0x3F) = 0x0B, payload = (0x0B << 16) | (0xFF << 8) | 0x00 = 0xBFF00
        // Wait, let me be more careful.
        // bytes = [0xCB, 0xFF, 0x00]
        // payload = ((0xCB & 0x3F) << 16) | (0xFF << 8) | 0x00
        // = (0x0B << 16) | 0xFF00 | 0
        // = 0x0B_FF_00
        // kind = (0x0BFF00 >> 20) & 0x03 = 0x00 => BUFS
        // units = (0x0BFF00 >> 18) & 0x03 = (0x0BFF00 >> 18) = 0x02F >> ... let me compute in hex
        // 0x0BFF00 in binary: 0000_1011_1111_1111_0000_0000
        // bits[21:20] = 00 => BUFS
        // bits[19:18] = 10 => Mbits
        // bits[17:8] = 1111111111 = 0x3FF => BUFS = 1023
        // bits[7:0] = 00000000 => reserved
        let result = decode_issy_long([0xCB, 0xFF, 0x00]).unwrap();
        match result {
            Issy::Signalling(SignallingKind::Bufs { bufs, units }) => {
                assert_eq!(bufs, 0x3FF);
                assert_eq!(units, BufsUnit::Mbits);
            }
            other => panic!("expected BUFS, got {other:?}"),
        }
    }

    #[test]
    fn signalling_tto_decode() {
        // '11' prefix, bits[21:20]=0b01 (TTO)
        // bits[19:16]=0b0101 (4 MSBs of TTO_E = 5)
        // byte1 bit7 = LSB of TTO_E (1 => TTO_E = 0b1011 = 11)
        // byte1 bits[6:0] = TTO_M = 0x7F = 127
        // byte2 = TTO_L = 0x80
        // byte0: 0b11_01_0101 = 0xD5
        // byte1: 0b1_1111111 = 0xFF
        // byte2: 0x80
        // payload = ((0xD5 & 0x3F) << 16) | (0xFF << 8) | 0x80
        // = (0x15 << 16) | 0xFF00 | 0x80
        // = 0x15_FF_80
        // bits[21:20] = 01 => TTO
        // bits[19:16] = 0101 => TTO_E MSBs = 5
        // bit 15 = 1 => TTO_E LSB = 1 => TTO_E = 0b1011 = 11
        // bits[14:8] = 1111111 => TTO_M = 127
        // bits[7:0] = 10000000 => TTO_L = 128
        let result = decode_issy_long([0xD5, 0xFF, 0x80]).unwrap();
        match result {
            Issy::Signalling(SignallingKind::Tto {
                tto_e,
                tto_m,
                tto_l,
            }) => {
                assert_eq!(tto_e, 11);
                assert_eq!(tto_m, 127);
                assert_eq!(tto_l, 128);
            }
            other => panic!("expected TTO, got {other:?}"),
        }
    }

    #[test]
    fn signalling_reserved_decode() {
        // '11' prefix, bits[21:20]=0b10 (reserved)
        // byte0: 0b11_10_XXXX = 0b11_10_0000 = 0xE0
        // payload = ((0xE0 & 0x3F) << 16) | 0x0000 = 0x200000
        // kind = (0x200000 >> 20) & 0x03 = 2 => reserved
        let result = decode_issy_long([0xE0, 0x00, 0x00]).unwrap();
        match result {
            Issy::Signalling(SignallingKind::Reserved(remainder)) => {
                assert_eq!(remainder, 0x00000);
            }
            other => panic!("expected Reserved, got {other:?}"),
        }
    }

    #[test]
    fn bufs_unit_round_trip() {
        for b in 0..=3u8 {
            assert_eq!(BufsUnit::from_u8(b).to_u8(), b);
        }
    }

    #[test]
    fn bufs_unit_name() {
        assert_eq!(BufsUnit::Bits.name(), "bits");
        assert_eq!(BufsUnit::Kbits.name(), "Kbits");
        assert_eq!(BufsUnit::Mbits.name(), "Mbits");
        assert_eq!(BufsUnit::Kbits8.name(), "8 Kbits");
    }
}
