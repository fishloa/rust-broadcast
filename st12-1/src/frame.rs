//! The LTC codeword — SMPTE ST 12-1:2014 §9. See `st12-1/docs/st12-1.md` for
//! the curated spec transcription this module implements field-for-field.

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Named constants (no magic numbers) — ST 12-1 §9.2/Tables 2-5
// ---------------------------------------------------------------------------

/// Length of the LTC codeword: 80 bits (§9.1) packed 8 bits/byte.
pub const FRAME_LEN: usize = 10;

/// Maximum hour value — "24-hour clock ... to 23 hours" (§5.2/§6.2/§7.2).
pub const MAX_HOURS: u8 = 23;
/// Maximum minute/second value — "... 59 minutes, and 59 seconds".
pub const MAX_MINUTES_SECONDS: u8 = 59;
/// Maximum frame value across all supported frame rates: 30-frame counting
/// (drop or non-drop, §5.2.1/§5.2.2) numbers frames `00` through `29`, the
/// widest of the three per-rate bounds (25-frame: `00`-`24` per §6.2;
/// 24-frame: `00`-`23` per §7.2). The 80-bit codeword carries no
/// self-describing frame-rate field, so this crate validates against the
/// widest bound and leaves the tighter per-rate bound to a caller that knows
/// its stream's frame rate (see `docs/st12-1.md` §8.2).
pub const MAX_FRAMES: u8 = 29;
/// Maximum value of one 4-bit binary group ("user bits") nibble (§8.1/Table 4).
pub const MAX_BINARY_GROUP: u8 = 0x0F;

/// The fixed synchronization word (§9.2.5, Table 5), as the two bytes it
/// occupies under this crate's bit-to-byte packing (`docs/st12-1.md`'s "Byte
/// packing convention"): byte 8 holds bits 64-71, byte 9 holds bits 72-79.
/// This is the well-known LTC sync-word byte pair.
pub const SYNC_WORD: [u8; 2] = [0xFC, 0xBF];

// ---------------------------------------------------------------------------
// FrameRate — ST 12-1 Table 3's three counting-mode columns
// ---------------------------------------------------------------------------

/// Which of ST 12-1 Table 3's three flag-bit-position columns applies.
///
/// The drop-frame flag (bit 10) and color-frame flag (bit 11) sit at fixed
/// bit positions regardless of frame rate, but the polarity-correction/BGF0/
/// BGF1/BGF2 bits move: 30-frame and 24-frame share one mapping, while
/// 25-frame swaps bit 27 and bit 59's meaning (see `docs/st12-1.md`'s
/// "Judgment call" note on Table 3). The 80-bit codeword itself carries no
/// self-describing frame-rate field, so the caller supplies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FrameRate {
    /// 30-frame counting (NTSC-related, drop or non-drop, §5.2).
    Fps30,
    /// 25-frame counting (PAL-related, §6.2).
    Fps25,
    /// 24-frame counting (film-related, §7.2).
    Fps24,
}

impl FrameRate {
    /// The spec's own label for this counting mode (Table 3's column header).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Fps30 => "30-frame",
            Self::Fps25 => "25-frame",
            Self::Fps24 => "24-frame",
        }
    }
}

broadcast_common::impl_spec_display!(FrameRate);

// ---------------------------------------------------------------------------
// BinaryGroupUsage — ST 12-1 Table 1
// ---------------------------------------------------------------------------

/// The meaning of the eight binary groups ("user bits"), per the three
/// binary group flag bits BGF2/BGF1/BGF0 (§8.3.3, Table 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BinaryGroupUsage {
    /// `000` — time address reference unspecified, binary group content
    /// unspecified (§8.4.1).
    UnspecifiedUnspecified,
    /// `001` — time address reference unspecified, binary groups hold an
    /// eight-bit character set (§8.4.2).
    UnspecifiedEightBitCodes,
    /// `010` — time address referenced to clock time, binary group content
    /// unspecified (§8.4.3, §8.5).
    ClockTimeUnspecified,
    /// `011` — reserved for future definition by SMPTE (§8.4.4): "shall not
    /// be used".
    Reserved,
    /// `100` — time address reference unspecified, binary groups hold date
    /// and time zone data (§8.4.5).
    UnspecifiedDateTimeZone,
    /// `101` — time address reference unspecified, binary groups hold
    /// page/line multiplex data (§8.4.6).
    UnspecifiedPageLine,
    /// `110` — time address referenced to clock time, binary groups hold
    /// date and time zone data (§8.4.7, §8.5).
    ClockTimeDateTimeZone,
    /// `111` — time address referenced to clock time, binary groups hold
    /// page/line multiplex data (§8.4.8, §8.5).
    ClockTimePageLine,
}

impl BinaryGroupUsage {
    /// The spec's own compound label (Table 1's "Time address reference" /
    /// "Binary group" column pair).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::UnspecifiedUnspecified => "unspecified time address, unspecified binary group",
            Self::UnspecifiedEightBitCodes => "unspecified time address, 8-bit codes",
            Self::ClockTimeUnspecified => "clock time, unspecified binary group",
            Self::Reserved => "reserved time address, reserved binary group",
            Self::UnspecifiedDateTimeZone => "unspecified time address, date and time zone",
            Self::UnspecifiedPageLine => "unspecified time address, page/line",
            Self::ClockTimeDateTimeZone => "clock time, date and time zone",
            Self::ClockTimePageLine => "clock time, page/line",
        }
    }

    /// Look up the Table 1 row for a given BGF2/BGF1/BGF0 combination.
    #[must_use]
    pub fn from_flags(bgf2: bool, bgf1: bool, bgf0: bool) -> Self {
        match (bgf2, bgf1, bgf0) {
            (false, false, false) => Self::UnspecifiedUnspecified,
            (false, false, true) => Self::UnspecifiedEightBitCodes,
            (false, true, false) => Self::ClockTimeUnspecified,
            (false, true, true) => Self::Reserved,
            (true, false, false) => Self::UnspecifiedDateTimeZone,
            (true, false, true) => Self::UnspecifiedPageLine,
            (true, true, false) => Self::ClockTimeDateTimeZone,
            (true, true, true) => Self::ClockTimePageLine,
        }
    }
}

broadcast_common::impl_spec_display!(BinaryGroupUsage);

/// The three binary group flag bits (§8.3.3), resolved from an [`LtcFrame`]
/// against a chosen [`FrameRate`] (see [`LtcFrame::binary_group_flags`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BinaryGroupFlags {
    /// BGF2.
    pub bgf2: bool,
    /// BGF1.
    pub bgf1: bool,
    /// BGF0.
    pub bgf0: bool,
}

impl BinaryGroupFlags {
    /// Table 1's binary-group-usage classification for this flag combination.
    #[must_use]
    pub fn usage(&self) -> BinaryGroupUsage {
        BinaryGroupUsage::from_flags(self.bgf2, self.bgf1, self.bgf0)
    }
}

// ---------------------------------------------------------------------------
// LtcFrame — the 80-bit logical LTC codeword
// ---------------------------------------------------------------------------

/// A parsed (or to-be-serialized) 80-bit LTC codeword (§9.2): the BCD time
/// address, drop/color frame flags, the four rate-dependent flag bits
/// (polarity correction / BGF0 / BGF1 / BGF2 — see [`FrameRate`]), the eight
/// 4-bit binary groups ("user bits", §8.1), and the fixed synchronization
/// word (always [`SYNC_WORD`] on serialize; validated on parse).
///
/// This models only the logical codeword content — not the §9.3 biphase-mark
/// modulation that carries it as an analog/digital audio signal (see
/// `docs/st12-1.md`'s "Scope" section).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LtcFrame {
    /// Hours, `00`-`23` (§9.2.1, Table 2 bits 48-51/56-57).
    pub hours: u8,
    /// Minutes, `00`-`59` (Table 2 bits 32-35/40-42).
    pub minutes: u8,
    /// Seconds, `00`-`59` (Table 2 bits 16-19/24-26).
    pub seconds: u8,
    /// Frames, `00`-`29` (Table 2 bits 0-3/8-9); the caller applies the
    /// tighter per-`FrameRate` bound if it needs one (see [`MAX_FRAMES`]).
    pub frames: u8,
    /// Drop frame flag, bit 10 (§8.3.1): set when NTSC drop-frame
    /// compensation (§5.2.2) is in effect. Fixed bit position across all
    /// frame rates; reserved-zero where not applicable (§9.2.2 note).
    pub drop_frame_flag: bool,
    /// Color frame flag, bit 11 (§8.3.2). Fixed bit position across all
    /// frame rates; reserved-zero where not applicable (§9.2.2 note).
    pub color_frame_flag: bool,
    /// The wire bit at position 27 (Table 3): 30-frame/24-frame's polarity
    /// correction bit, or 25-frame's BGF0 — see [`LtcFrame::polarity_correction`]
    /// / [`LtcFrame::binary_group_flags`] to resolve its meaning for a given
    /// [`FrameRate`].
    pub flag_bit_27: bool,
    /// The wire bit at position 43 (Table 3): 30-frame/24-frame's BGF0, or
    /// 25-frame's BGF2.
    pub flag_bit_43: bool,
    /// The wire bit at position 58 (Table 3): BGF1 in all three frame rates.
    pub flag_bit_58: bool,
    /// The wire bit at position 59 (Table 3): 30-frame/24-frame's BGF2, or
    /// 25-frame's polarity correction bit.
    pub flag_bit_59: bool,
    /// The eight 4-bit binary groups ("user bits"), each `0x0`-`0xF`, in
    /// first..eighth order (Table 4 bits 4-7/12-15/20-23/28-31/36-39/44-47/
    /// 52-55/60-63). Their collective meaning is given by
    /// [`LtcFrame::binary_group_flags`]`.usage()`.
    pub user_bits: [u8; 8],
}

impl LtcFrame {
    /// Resolve the §9.2.3 biphase-mark polarity-correction bit for `rate`
    /// (Table 3: bit 27 for 30-frame/24-frame, bit 59 for 25-frame).
    #[must_use]
    pub fn polarity_correction(&self, rate: FrameRate) -> bool {
        match rate {
            FrameRate::Fps25 => self.flag_bit_59,
            FrameRate::Fps30 | FrameRate::Fps24 => self.flag_bit_27,
        }
    }

    /// Resolve the §8.3.3 binary group flags (BGF2/BGF1/BGF0) for `rate`
    /// (Table 3's per-rate bit-position mapping).
    #[must_use]
    pub fn binary_group_flags(&self, rate: FrameRate) -> BinaryGroupFlags {
        let (bgf0, bgf2) = match rate {
            FrameRate::Fps25 => (self.flag_bit_27, self.flag_bit_43),
            FrameRate::Fps30 | FrameRate::Fps24 => (self.flag_bit_43, self.flag_bit_59),
        };
        BinaryGroupFlags {
            bgf2,
            bgf1: self.flag_bit_58,
            bgf0,
        }
    }

    /// Validate all typed fields against their wire/semantic bounds. Called
    /// by both `parse` (on decoded values) and `serialize_into` (on
    /// caller-constructed values), so a hand-built out-of-range `LtcFrame`
    /// cannot silently round-trip.
    fn validate(&self) -> Result<()> {
        if self.hours > MAX_HOURS {
            return Err(Error::InvalidValue {
                field: "hours",
                value: self.hours,
                reason: "exceeds the 24-hour-clock maximum (23)",
            });
        }
        if self.minutes > MAX_MINUTES_SECONDS {
            return Err(Error::InvalidValue {
                field: "minutes",
                value: self.minutes,
                reason: "exceeds the maximum (59)",
            });
        }
        if self.seconds > MAX_MINUTES_SECONDS {
            return Err(Error::InvalidValue {
                field: "seconds",
                value: self.seconds,
                reason: "exceeds the maximum (59)",
            });
        }
        if self.frames > MAX_FRAMES {
            return Err(Error::InvalidValue {
                field: "frames",
                value: self.frames,
                reason: "exceeds the widest supported frame-rate maximum (29)",
            });
        }
        for (index, &value) in self.user_bits.iter().enumerate() {
            if value > MAX_BINARY_GROUP {
                return Err(Error::InvalidBinaryGroup { index, value });
            }
        }
        Ok(())
    }
}

impl<'a> Parse<'a> for LtcFrame {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < FRAME_LEN {
            return Err(Error::BufferTooShort {
                need: FRAME_LEN,
                have: bytes.len(),
                what: "LTC codeword",
            });
        }

        if [bytes[8], bytes[9]] != SYNC_WORD {
            return Err(Error::SyncWordMismatch {
                expected: SYNC_WORD,
                found: [bytes[8], bytes[9]],
            });
        }

        let frame_units = bytes[0] & 0x0F;
        let user_bits_1 = bytes[0] >> 4;

        let frame_tens = bytes[1] & 0x03;
        let drop_frame_flag = bytes[1] & 0x04 != 0;
        let color_frame_flag = bytes[1] & 0x08 != 0;
        let user_bits_2 = bytes[1] >> 4;

        let seconds_units = bytes[2] & 0x0F;
        let user_bits_3 = bytes[2] >> 4;

        let seconds_tens = bytes[3] & 0x07;
        let flag_bit_27 = bytes[3] & 0x08 != 0;
        let user_bits_4 = bytes[3] >> 4;

        let minutes_units = bytes[4] & 0x0F;
        let user_bits_5 = bytes[4] >> 4;

        let minutes_tens = bytes[5] & 0x07;
        let flag_bit_43 = bytes[5] & 0x08 != 0;
        let user_bits_6 = bytes[5] >> 4;

        let hours_units = bytes[6] & 0x0F;
        let user_bits_7 = bytes[6] >> 4;

        let hours_tens = bytes[7] & 0x03;
        let flag_bit_58 = bytes[7] & 0x04 != 0;
        let flag_bit_59 = bytes[7] & 0x08 != 0;
        let user_bits_8 = bytes[7] >> 4;

        let frame = Self {
            hours: hours_tens * 10 + hours_units,
            minutes: minutes_tens * 10 + minutes_units,
            seconds: seconds_tens * 10 + seconds_units,
            frames: frame_tens * 10 + frame_units,
            drop_frame_flag,
            color_frame_flag,
            flag_bit_27,
            flag_bit_43,
            flag_bit_58,
            flag_bit_59,
            user_bits: [
                user_bits_1,
                user_bits_2,
                user_bits_3,
                user_bits_4,
                user_bits_5,
                user_bits_6,
                user_bits_7,
                user_bits_8,
            ],
        };
        frame.validate()?;
        Ok(frame)
    }
}

impl Serialize for LtcFrame {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        FRAME_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if buf.len() < FRAME_LEN {
            return Err(Error::BufferTooShort {
                need: FRAME_LEN,
                have: buf.len(),
                what: "LTC codeword serialize output",
            });
        }
        self.validate()?;

        let frame_units = self.frames % 10;
        let frame_tens = self.frames / 10;
        let seconds_units = self.seconds % 10;
        let seconds_tens = self.seconds / 10;
        let minutes_units = self.minutes % 10;
        let minutes_tens = self.minutes / 10;
        let hours_units = self.hours % 10;
        let hours_tens = self.hours / 10;

        buf[0] = frame_units | (self.user_bits[0] << 4);
        buf[1] = frame_tens
            | (u8::from(self.drop_frame_flag) << 2)
            | (u8::from(self.color_frame_flag) << 3)
            | (self.user_bits[1] << 4);
        buf[2] = seconds_units | (self.user_bits[2] << 4);
        buf[3] = seconds_tens | (u8::from(self.flag_bit_27) << 3) | (self.user_bits[3] << 4);
        buf[4] = minutes_units | (self.user_bits[4] << 4);
        buf[5] = minutes_tens | (u8::from(self.flag_bit_43) << 3) | (self.user_bits[5] << 4);
        buf[6] = hours_units | (self.user_bits[6] << 4);
        buf[7] = hours_tens
            | (u8::from(self.flag_bit_58) << 2)
            | (u8::from(self.flag_bit_59) << 3)
            | (self.user_bits[7] << 4);
        buf[8] = SYNC_WORD[0];
        buf[9] = SYNC_WORD[1];

        Ok(FRAME_LEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_frame() -> LtcFrame {
        LtcFrame {
            hours: 1,
            minutes: 23,
            seconds: 45,
            frames: 13,
            drop_frame_flag: false,
            color_frame_flag: true,
            flag_bit_27: true,
            flag_bit_43: true,
            flag_bit_58: false,
            flag_bit_59: true,
            user_bits: [1, 2, 3, 4, 5, 6, 7, 8],
        }
    }

    #[test]
    fn round_trip() {
        let f = sample_frame();
        let mut out = [0u8; FRAME_LEN];
        f.serialize_into(&mut out).unwrap();
        assert_eq!(LtcFrame::parse(&out).unwrap(), f);
    }

    #[test]
    fn serialize_then_parse_matches_spec_vector() {
        // See docs/st12-1.md's "Worked vector" section.
        let f = sample_frame();
        let mut out = [0u8; FRAME_LEN];
        f.serialize_into(&mut out).unwrap();
        assert_eq!(
            out,
            [0x13, 0x29, 0x35, 0x4C, 0x53, 0x6A, 0x71, 0x88, 0xFC, 0xBF]
        );
    }

    #[test]
    fn rejects_sync_word_mismatch() {
        let mut bytes = [0x13, 0x29, 0x35, 0x4C, 0x53, 0x6A, 0x71, 0x88, 0xFC, 0xBF];
        bytes[9] = 0x00;
        assert!(matches!(
            LtcFrame::parse(&bytes),
            Err(Error::SyncWordMismatch { .. })
        ));
    }

    #[test]
    fn rejects_buffer_too_short() {
        let bytes = [0u8; 9];
        assert!(matches!(
            LtcFrame::parse(&bytes),
            Err(Error::BufferTooShort {
                need: 10,
                have: 9,
                ..
            })
        ));
    }

    #[test]
    fn rejects_hours_over_23() {
        let mut f = sample_frame();
        f.hours = 24;
        let mut out = [0u8; FRAME_LEN];
        assert!(matches!(
            f.serialize_into(&mut out),
            Err(Error::InvalidValue { field: "hours", .. })
        ));
    }

    #[test]
    fn rejects_frames_over_29() {
        let mut f = sample_frame();
        f.frames = 30;
        let mut out = [0u8; FRAME_LEN];
        assert!(matches!(
            f.serialize_into(&mut out),
            Err(Error::InvalidValue {
                field: "frames",
                ..
            })
        ));
    }

    #[test]
    fn rejects_user_bit_over_15() {
        let mut f = sample_frame();
        f.user_bits[3] = 0x10;
        let mut out = [0u8; FRAME_LEN];
        assert!(matches!(
            f.serialize_into(&mut out),
            Err(Error::InvalidBinaryGroup {
                index: 3,
                value: 0x10
            })
        ));
    }

    #[test]
    fn polarity_correction_and_bgf_swap_between_25_and_other_rates() {
        let f = sample_frame(); // flag_bit_27=1, flag_bit_43=1, flag_bit_58=0, flag_bit_59=1
        assert!(f.polarity_correction(FrameRate::Fps30));
        assert!(f.polarity_correction(FrameRate::Fps24));
        assert!(f.polarity_correction(FrameRate::Fps25));

        let bg30 = f.binary_group_flags(FrameRate::Fps30);
        assert_eq!(
            bg30,
            BinaryGroupFlags {
                bgf2: true,
                bgf1: false,
                bgf0: true
            }
        );
        assert_eq!(bg30.usage(), BinaryGroupUsage::UnspecifiedPageLine);

        // With distinct bit-27/bit-43 values, the 25-frame <-> 30/24-frame
        // swap (Table 3) is directly observable: bit 27 feeds BGF0 for
        // 25-frame but polarity-correction for 30/24-frame, and vice versa
        // for bit 59 vs. BGF2.
        let mut g = f;
        g.flag_bit_27 = true; // 30/24-frame: polarity=1; 25-frame: BGF0=1
        g.flag_bit_43 = false; // 30/24-frame: BGF0=0;    25-frame: BGF2=0
        g.flag_bit_58 = true; // BGF1=1 in every rate
        g.flag_bit_59 = false; // 30/24-frame: BGF2=0;    25-frame: polarity=0

        assert!(g.polarity_correction(FrameRate::Fps30));
        assert!(g.polarity_correction(FrameRate::Fps24));
        assert!(!g.polarity_correction(FrameRate::Fps25));

        let bg30 = g.binary_group_flags(FrameRate::Fps30);
        assert_eq!(
            bg30,
            BinaryGroupFlags {
                bgf2: false,
                bgf1: true,
                bgf0: false
            }
        );
        let bg25 = g.binary_group_flags(FrameRate::Fps25);
        assert_eq!(
            bg25,
            BinaryGroupFlags {
                bgf2: false,
                bgf1: true,
                bgf0: true
            }
        );
        assert_ne!(
            bg30, bg25,
            "25-frame's BGF0 must differ from 30/24-frame's here"
        );
    }

    #[test]
    fn binary_group_usage_covers_all_eight_combinations() {
        for bgf2 in [false, true] {
            for bgf1 in [false, true] {
                for bgf0 in [false, true] {
                    // Must not panic for any of the 8 possible combinations.
                    let _ = BinaryGroupUsage::from_flags(bgf2, bgf1, bgf0);
                }
            }
        }
    }

    #[test]
    fn field_mutation_changes_bytes() {
        let a = sample_frame();
        let mut b = a;
        b.seconds = a.seconds.wrapping_add(1);
        let mut oa = [0u8; FRAME_LEN];
        let mut ob = [0u8; FRAME_LEN];
        a.serialize_into(&mut oa).unwrap();
        b.serialize_into(&mut ob).unwrap();
        assert_ne!(oa, ob);
    }
}
