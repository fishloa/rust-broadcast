//! System Clock Reference — 42-bit timestamp at 27 MHz
//! (ISO/IEC 13818-1 §2.5.3.3, Table 2-39).
//!
//! Encoded across 6 bytes: 33-bit base + 9-bit extension, with four interleaved
//! `marker_bit`s and a `'01'` prefix.

use crate::error::{Error, Result};

/// Number of bits in the `system_clock_reference_base` field.
const BASE_BITS: u32 = 33;
/// Number of bits in the `system_clock_reference_extension` field.
const EXT_BITS: u32 = 9;
/// 33-bit base mask.
const BASE_MASK: u64 = (1 << BASE_BITS) - 1;
/// 9-bit extension mask.
const EXT_MASK: u64 = (1 << EXT_BITS) - 1;

/// Number of bytes in the SCR field (including the `01` prefix).
pub(crate) const SCR_FIELD_LEN: usize = 6;

/// Bit shifts of the `system_clock_reference_base` groups within the 42-bit
/// value (ISO/IEC 13818-1 Table 2-39).
const BASE_SHIFT_HI: u32 = 30;
const BASE_SHIFT_HI_MID: u32 = 28;
const BASE_SHIFT_MID_MID: u32 = 20;
const BASE_SHIFT_MID_LO: u32 = 15;
const BASE_SHIFT_LO_HI: u32 = 13;
const BASE_SHIFT_LO_MID: u32 = 5;
/// Bit shift of the `base[32:30]`/`base[19:15]`/`base[4:0]` groups within their
/// field bytes.
const BASE_GROUP_SHIFT: u32 = 3;
/// Bit shifts of the `system_clock_reference_extension` groups.
const EXT_SHIFT_HI: u32 = 7;
const EXT_SHIFT_LO: u32 = 1;

/// Masks of the base groups as they sit in their field bytes.
const BASE_HI_MASK: u8 = 0x07;
const BASE_HI_MID_MASK: u8 = 0x03;
const BASE_MID_LO_MASK: u8 = 0x1F;
const BASE_LO_LO_MASK: u8 = 0x1F;
/// Mask of one whole field byte.
const BYTE_MASK: u64 = 0xFF;
/// Masks of the extension groups as they sit in their field bytes.
const EXT_HI_MASK: u8 = 0x03;
const EXT_LO_MASK: u64 = 0x7F;

/// Bit shift of the 2-bit `'01'` pack prefix.
const PREFIX_SHIFT: u32 = 6;
/// The `'01'` pack prefix in bits `[7:6]` of the first field byte.
const PREFIX_BITS: u8 = 0x40;
/// Bit shift used to spot the MPEG-1 `'0010'` pack prefix.
const MPEG1_PREFIX_SHIFT: u32 = 4;
/// The MPEG-1 `'0010'` pack prefix in bits `[7:4]` of the first field byte.
const MPEG1_PREFIX_BITS: u8 = 0x02;

/// `marker_bit` at bit 2 of the SCR field bytes that carry one.
const MARKER_BIT: u8 = 0x04;
/// `marker_bit` at bit 0 of the last SCR field byte.
const MARKER_BIT_LSB: u8 = 0x01;

/// System Clock Reference (42-bit total: 33-bit base + 9-bit extension,
/// in 27 MHz units).
///
/// Constructing is strict ([`Scr::from_clock27`] takes the validated
/// [`broadcast_common::clock33::Clock27`]). Parsing is lossless: the 9-bit wire
/// extension can be `300..=511`, which is off the 27 MHz clock, so a parsed
/// value keeps the parts exactly as read and re-serializes byte-identically;
/// [`Scr::to_clock27`] is the checked view that reports such a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Scr {
    /// 33-bit SCR base (27 MHz ticks, integer part).
    pub base: u64,
    /// 9-bit SCR extension (27 MHz ticks, fractional part / 300); a parsed
    /// value may carry `300..=511` (see [`Scr::to_clock27`]).
    pub extension: u16,
}

impl Scr {
    /// Construct from a validated 27 MHz clock value.
    #[must_use]
    pub fn from_clock27(clock: broadcast_common::clock33::Clock27) -> Self {
        Self {
            base: clock.base.ticks(),
            extension: clock.ext,
        }
    }

    /// The total SCR value in 27 MHz ticks.
    ///
    /// `base * 300 + extension`. For a parsed value whose extension is at or
    /// above 300 this is not a time on the clock; check
    /// [`to_clock27`](Self::to_clock27) first.
    #[must_use]
    pub const fn ticks(self) -> u64 {
        broadcast_common::clock33::pcr_from_parts(self.base, self.extension)
    }

    /// The checked 27 MHz clock value.
    ///
    /// # Errors
    /// [`broadcast_common::clock33::ClockOutOfRange`] when the extension is at
    /// or above 300 (off the 27 MHz clock), as a real capture's pseudo-random
    /// field can be. An over-wide `base` (only constructible by hand, never by
    /// parsing) is reported the same way.
    pub fn to_clock27(
        self,
    ) -> core::result::Result<
        broadcast_common::clock33::Clock27,
        broadcast_common::clock33::ClockOutOfRange,
    > {
        use broadcast_common::clock33::{Clock27, ClockOutOfRange, WRAP_33BIT};
        if self.base >= WRAP_33BIT {
            return Err(ClockOutOfRange::base(self.base));
        }
        Clock27::from_wire_parts(self.base, self.extension).to_clock27()
    }

    /// Value in seconds.
    #[must_use]
    pub fn seconds(self) -> f64 {
        self.ticks() as f64 / broadcast_common::clock33::PCR_HZ as f64
    }
}

/// Bit-packed SCR field layout (48 bits total, 6 bytes):
///
/// `01`                                    | 2  [47:46]
/// `system_clock_reference_base[32:30]`    | 3  [45:43]
/// `marker_bit`                            | 1  [42]
/// `system_clock_reference_base[29:28]`    | 2  [41:40]
/// `system_clock_reference_base[27:20]`    | 8  [39:32]
/// `system_clock_reference_base[19:15]`    | 5  [31:27]
/// `marker_bit`                            | 1  [26]
/// `system_clock_reference_base[14:13]`    | 2  [25:24]
/// `system_clock_reference_base[12:5]`     | 8  [23:16]
/// `system_clock_reference_base[4:0]`      | 5  [15:11]
/// `marker_bit`                            | 1  [10]
/// `system_clock_reference_extension[8:7]` | 2  [9:8]
/// `system_clock_reference_extension[6:0]` | 7  [7:1]
/// `marker_bit`                            | 1  [0]
pub(crate) fn read_scr_field(b: &[u8], what: &'static str) -> Result<Scr> {
    if b.len() < SCR_FIELD_LEN {
        return Err(Error::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: SCR_FIELD_LEN,
                have: b.len(),
                what,
            },
        ));
    }

    let mut raw = [0_u8; SCR_FIELD_LEN];
    raw.copy_from_slice(&b[..SCR_FIELD_LEN]);
    let [b0, b1, b2, b3, b4, b5] = raw;

    // W4 (#1119): distinguish the ISO/IEC 11172-1 (MPEG-1) `'0010'` pack
    // prefix — a real, different (12-byte, no SCR-extension) layout this
    // crate does not implement — from bytes that are neither a valid
    // MPEG-1 nor MPEG-2 pack header. Previously both cases returned the
    // same `BadScrPrefix`, so a conformant plain-MPEG-1 `.mpg` (VCD etc.)
    // looked identical to actual corruption despite this crate's docs
    // claiming "MPEG-1/2 Program Stream" support.
    if (b0 >> MPEG1_PREFIX_SHIFT) == MPEG1_PREFIX_BITS {
        return Err(Error::Mpeg1NotSupported);
    }
    if (b0 >> PREFIX_SHIFT) != (PREFIX_BITS >> PREFIX_SHIFT) {
        return Err(Error::BadScrPrefix(b0 >> PREFIX_SHIFT));
    }

    // marker bits
    if (b0 & MARKER_BIT) == 0 {
        return Err(Error::BadMarker(what));
    }
    if (b2 & MARKER_BIT) == 0 {
        return Err(Error::BadMarker(what));
    }
    if (b4 & MARKER_BIT) == 0 {
        return Err(Error::BadMarker(what));
    }
    if (b5 & MARKER_BIT_LSB) == 0 {
        return Err(Error::BadMarker(what));
    }

    // base[32:30] from b0[5:3]
    let hi = u64::from((b0 >> BASE_GROUP_SHIFT) & BASE_HI_MASK);
    // base[29:28] from b0[1:0]
    let mid_hi = u64::from(b0 & BASE_HI_MID_MASK);
    // base[27:20] from b1[7:0]
    let mid_mid = u64::from(b1);
    // base[19:15] from b2[7:3]
    let mid_lo = u64::from((b2 >> BASE_GROUP_SHIFT) & BASE_MID_LO_MASK);
    // base[14:13] from b2[1:0]
    let lo_hi = u64::from(b2 & BASE_HI_MID_MASK);
    // base[12:5] from b3[7:0]
    let lo_mid = u64::from(b3);
    // base[4:0] from b4[7:3]
    let lo_lo = u64::from((b4 >> BASE_GROUP_SHIFT) & BASE_LO_LO_MASK);

    let base = (hi << BASE_SHIFT_HI)
        | (mid_hi << BASE_SHIFT_HI_MID)
        | (mid_mid << BASE_SHIFT_MID_MID)
        | (mid_lo << BASE_SHIFT_MID_LO)
        | (lo_hi << BASE_SHIFT_LO_HI)
        | (lo_mid << BASE_SHIFT_LO_MID)
        | lo_lo;

    // extension[8:7] from b4[1:0]
    let ext_hi = u16::from(b4 & EXT_HI_MASK);
    // extension[6:0] from b5[7:1]
    let ext_lo = u16::from(b5 >> EXT_SHIFT_LO);

    let extension = (ext_hi << EXT_SHIFT_HI) | ext_lo;

    Ok(Scr { base, extension })
}

/// Checked encode of an SCR field: rejects a `base` wider than 33 bits or an
/// `extension` wider than 9 bits instead of silently masking (#1129 class).
pub(crate) fn encode_scr_field(scr: Scr) -> crate::error::Result<[u8; SCR_FIELD_LEN]> {
    broadcast_core::len::fit_bits(scr.base, BASE_BITS, "pack_header.scr.base")?;
    broadcast_core::len::fit_bits(
        u64::from(scr.extension),
        EXT_BITS,
        "pack_header.scr.extension",
    )?;
    Ok(write_scr_field(scr))
}

/// Encode a 42-bit SCR into a 6-byte SCR field (without pack_start_code).
/// Infallible and masking: callers must range-check first (see
/// [`encode_scr_field`]).
pub(crate) fn write_scr_field(scr: Scr) -> [u8; SCR_FIELD_LEN] {
    let base = scr.base & BASE_MASK;
    let extension = (scr.extension as u64) & EXT_MASK;

    let b0 = PREFIX_BITS // '01' in bits [7:6]
        | (((base >> BASE_SHIFT_HI) & u64::from(BASE_HI_MASK)) as u8) << BASE_GROUP_SHIFT // base[32:30] in bits [5:3]
        | MARKER_BIT // marker_bit at bit2
        | ((base >> BASE_SHIFT_HI_MID) & u64::from(BASE_HI_MID_MASK)) as u8; // base[29:28] in bits [1:0]

    let b1 = ((base >> BASE_SHIFT_MID_MID) & BYTE_MASK) as u8; // base[27:20]

    let b2 = (((base >> BASE_SHIFT_MID_LO) & u64::from(BASE_MID_LO_MASK)) as u8) << BASE_GROUP_SHIFT // base[19:15] in bits [7:3]
        | MARKER_BIT // marker_bit at bit2
        | ((base >> BASE_SHIFT_LO_HI) & u64::from(BASE_HI_MID_MASK)) as u8; // base[14:13] in bits [1:0]

    let b3 = ((base >> BASE_SHIFT_LO_MID) & BYTE_MASK) as u8; // base[12:5]

    let b4 = ((base & u64::from(BASE_LO_LO_MASK)) as u8) << BASE_GROUP_SHIFT // base[4:0] in bits [7:3]
        | MARKER_BIT // marker_bit at bit2
        | ((extension >> EXT_SHIFT_HI) & u64::from(EXT_HI_MASK)) as u8; // extension[8:7] in bits [1:0]

    let b5 = (((extension & EXT_LO_MASK) as u8) << EXT_SHIFT_LO) // extension[6:0] in bits [7:1]
        | MARKER_BIT_LSB; // marker_bit at bit0

    [b0, b1, b2, b3, b4, b5]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scr_round_trip_boundary_values() {
        let cases: &[(u64, u16)] = &[
            (0, 0),
            (1, 0),
            (0, 1),
            (BASE_MASK, EXT_MASK as u16),
            (0x0001_9999, 0x0123),
            (0x0123_4567, 0x1AB),
        ];
        for &(base_val, ext) in cases {
            let scr = Scr {
                base: base_val,
                extension: ext,
            };
            let enc = write_scr_field(scr);
            let dec = read_scr_field(&enc, "SCR").unwrap();
            assert_eq!(dec, scr, "base={base_val:#x}, ext={ext:#x}");
        }
    }

    #[test]
    fn scr_zero_matches_fixture_pattern() {
        // The ffmpeg-muxed fixture uses SCR base=0, extension=0
        let scr = Scr {
            base: 0,
            extension: 0,
        };
        let enc = write_scr_field(scr);
        // Matches fixture bytes for the constant part of SCR: 44 00 04 00 04 01
        assert_eq!(enc, [0x44, 0x00, 0x04, 0x00, 0x04, 0x01]);
        let dec = read_scr_field(&enc, "SCR").unwrap();
        assert_eq!(dec, scr);
    }

    #[test]
    fn scr_rejects_bad_prefix() {
        let mut enc = write_scr_field(Scr {
            base: 0,
            extension: 0,
        });
        enc[0] |= 0x80; // change '01' to '11'
        assert!(matches!(
            read_scr_field(&enc, "SCR"),
            Err(Error::BadScrPrefix(_))
        ));
    }

    /// W4 (#1119): a real ISO/IEC 11172-1 (MPEG-1) pack prefix (`'0010'`,
    /// top 4 bits of byte 0) must be reported distinctly from actual
    /// corruption, not folded into the same `BadScrPrefix` a genuinely
    /// invalid byte gets.
    #[test]
    fn scr_reports_mpeg1_prefix_distinctly_from_corruption() {
        let mut enc = write_scr_field(Scr {
            base: 0,
            extension: 0,
        });
        enc[0] = (enc[0] & 0x0F) | 0x20; // top 4 bits '0010' (MPEG-1 prefix)
        assert!(matches!(
            read_scr_field(&enc, "SCR"),
            Err(Error::Mpeg1NotSupported)
        ));
    }

    #[test]
    fn scr_rejects_bad_marker() {
        for byte_idx in [0usize, 2, 4] {
            let mut enc = write_scr_field(Scr {
                base: 0,
                extension: 0,
            });
            enc[byte_idx] &= !0x04; // clear marker at bit2
            assert!(matches!(
                read_scr_field(&enc, "SCR"),
                Err(Error::BadMarker(_))
            ));
        }
        // marker in bit0 of byte5
        let mut enc = write_scr_field(Scr {
            base: 0,
            extension: 0,
        });
        enc[5] &= 0xFE;
        assert!(matches!(
            read_scr_field(&enc, "SCR"),
            Err(Error::BadMarker(_))
        ));
    }

    #[test]
    fn scr_ticks() {
        let scr = Scr {
            base: 100,
            extension: 50,
        };
        assert_eq!(scr.ticks(), 100 * 300 + 50);
    }

    #[test]
    fn scr_seconds() {
        let scr = Scr {
            base: 27_000_000 / 300,
            extension: 0,
        };
        assert!((scr.seconds() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn scr_seconds_follow_the_actual_clock_value() {
        assert_eq!(
            Scr {
                base: 0,
                extension: 0,
            }
            .seconds(),
            0.0
        );
        let half = Scr {
            base: 45_000,
            extension: 150,
        };
        assert!(
            (half.seconds() - 0.500_005_555_555_555_6).abs() < 1e-12,
            "got {}",
            half.seconds()
        );
    }

    #[test]
    fn bad_scr_prefix_reports_the_two_prefix_bits() {
        let mut enc = write_scr_field(Scr {
            base: 0,
            extension: 0,
        });
        enc[0] = 0x80; // '10' prefix
        assert_eq!(
            read_scr_field(&enc, "SCR").unwrap_err(),
            Error::BadScrPrefix(0b10)
        );
    }

    #[test]
    fn scr_from_clock27_round_trips_the_validated_value() {
        let clock = broadcast_common::clock33::Clock27::from_parts(100, 50).expect("in range");
        let scr = Scr::from_clock27(clock);
        assert_eq!((scr.base, scr.extension), (100, 50));
        assert_eq!(scr.to_clock27(), Ok(clock));
    }

    /// The 9-bit wire extension can encode 300..=511, which is off the 27 MHz
    /// clock. A parsed field keeps it (lossless round trip) and the checked
    /// accessor reports it, so a real capture's pack header still parses.
    #[test]
    fn scr_off_clock_wire_extension_parses_losslessly_and_reports() {
        for extension in [300u16, 408, EXT_MASK as u16] {
            let bytes = write_scr_field(Scr {
                base: 42,
                extension,
            });
            let scr = read_scr_field(&bytes, "SCR").expect("a wire-legal field parses");
            assert_eq!((scr.base, scr.extension), (42, extension));
            assert_eq!(
                write_scr_field(scr),
                bytes,
                "re-serializes byte-identically"
            );
            let err = scr.to_clock27().unwrap_err();
            assert_eq!(err.field, "27 MHz extension");
            assert_eq!(err.value, u64::from(extension));
        }
        let off_clock = Scr {
            base: 1,
            extension: 299,
        };
        assert_eq!(
            off_clock.to_clock27(),
            Ok(broadcast_common::clock33::Clock27::from_parts(1, 299).expect("in range"))
        );
        // A base is 33 bits on the wire, so only a hand-built over-wide one is
        // reported.
        assert_eq!(
            Scr {
                base: 1 << 33,
                extension: 0,
            }
            .to_clock27()
            .unwrap_err()
            .field,
            "27 MHz base"
        );
    }
}
