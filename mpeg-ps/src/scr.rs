//! System Clock Reference — 42-bit timestamp at 27 MHz
//! (ISO/IEC 13818-1 §2.5.3.3, Table 2-39).
//!
//! Encoded across 6 bytes: 33-bit base + 9-bit extension, with four interleaved
//! `marker_bit`s and a `'01'` prefix.

use crate::error::{Error, Result};

/// 33-bit base mask.
const BASE_MASK: u64 = (1 << 33) - 1;
/// 9-bit extension mask.
const EXT_MASK: u64 = (1 << 9) - 1;

/// Number of bytes in the SCR field (including the `01` prefix).
pub(crate) const SCR_FIELD_LEN: usize = 6;

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
            broadcast_common::error::BufferTooShort {
                need: SCR_FIELD_LEN,
                have: b.len(),
                what,
            },
        ));
    }

    let b0 = b[0];
    let b1 = b[1];
    let b2 = b[2];
    let b3 = b[3];
    let b4 = b[4];
    let b5 = b[5];

    // W4 (#1119): distinguish the ISO/IEC 11172-1 (MPEG-1) `'0010'` pack
    // prefix — a real, different (12-byte, no SCR-extension) layout this
    // crate does not implement — from bytes that are neither a valid
    // MPEG-1 nor MPEG-2 pack header. Previously both cases returned the
    // same `BadScrPrefix`, so a conformant plain-MPEG-1 `.mpg` (VCD etc.)
    // looked identical to actual corruption despite this crate's docs
    // claiming "MPEG-1/2 Program Stream" support.
    if (b0 >> 4) == 0b0010 {
        return Err(Error::Mpeg1NotSupported);
    }
    if (b0 >> 6) != 0b01 {
        return Err(Error::BadScrPrefix(b0 >> 6));
    }

    // marker bits
    if (b0 & 0x04) == 0 {
        return Err(Error::BadMarker(what));
    }
    if (b2 & 0x04) == 0 {
        return Err(Error::BadMarker(what));
    }
    if (b4 & 0x04) == 0 {
        return Err(Error::BadMarker(what));
    }
    if (b5 & 0x01) == 0 {
        return Err(Error::BadMarker(what));
    }

    // base[32:30] from b0[5:3]
    let hi = u64::from((b0 >> 3) & 0x07);
    // base[29:28] from b0[1:0]
    let mid_hi = u64::from(b0 & 0x03);
    // base[27:20] from b1[7:0]
    let mid_mid = u64::from(b1);
    // base[19:15] from b2[7:3]
    let mid_lo = u64::from((b2 >> 3) & 0x1F);
    // base[14:13] from b2[1:0]
    let lo_hi = u64::from(b2 & 0x03);
    // base[12:5] from b3[7:0]
    let lo_mid = u64::from(b3);
    // base[4:0] from b4[7:3]
    let lo_lo = u64::from((b4 >> 3) & 0x1F);

    let base = (hi << 30)
        | (mid_hi << 28)
        | (mid_mid << 20)
        | (mid_lo << 15)
        | (lo_hi << 13)
        | (lo_mid << 5)
        | lo_lo;

    // extension[8:7] from b4[1:0]
    let ext_hi = u16::from(b4 & 0x03);
    // extension[6:0] from b5[7:1]
    let ext_lo = u16::from(b5 >> 1);

    let extension = (ext_hi << 7) | ext_lo;

    Ok(Scr { base, extension })
}

/// Checked encode of an SCR field: rejects a `base` wider than 33 bits or an
/// `extension` wider than 9 bits instead of silently masking (#1129 class).
pub(crate) fn encode_scr_field(scr: Scr) -> crate::error::Result<[u8; SCR_FIELD_LEN]> {
    broadcast_common::len::fit_bits(scr.base, 33, "pack_header.scr.base")?;
    broadcast_common::len::fit_bits(u64::from(scr.extension), 9, "pack_header.scr.extension")?;
    Ok(write_scr_field(scr))
}

/// Encode a 42-bit SCR into a 6-byte SCR field (without pack_start_code).
/// Infallible and masking: callers must range-check first (see
/// [`encode_scr_field`]).
pub(crate) fn write_scr_field(scr: Scr) -> [u8; SCR_FIELD_LEN] {
    let base = scr.base & BASE_MASK;
    let extension = (scr.extension as u64) & EXT_MASK;

    let b0 = 0x40 // '01' in bits [7:6]
        | (((base >> 30) & 0x07) as u8) << 3   // base[32:30] in bits [5:3]
        | 0x04 // marker_bit at bit2
        | ((base >> 28) & 0x03) as u8; // base[29:28] in bits [1:0]

    let b1 = ((base >> 20) & 0xFF) as u8; // base[27:20]

    let b2 = (((base >> 15) & 0x1F) as u8) << 3 // base[19:15] in bits [7:3]
        | 0x04 // marker_bit at bit2
        | ((base >> 13) & 0x03) as u8; // base[14:13] in bits [1:0]

    let b3 = ((base >> 5) & 0xFF) as u8; // base[12:5]

    let b4 = ((base & 0x1F) as u8) << 3 // base[4:0] in bits [7:3]
        | 0x04 // marker_bit at bit2
        | ((extension >> 7) & 0x03) as u8; // extension[8:7] in bits [1:0]

    let b5 = (((extension & 0x7F) as u8) << 1) // extension[6:0] in bits [7:1]
        | 0x01; // marker_bit at bit0

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
