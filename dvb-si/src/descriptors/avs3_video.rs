//! AVS3_video_descriptor — T/AI 109.6-2022 clause 9.3.2 (Table 9) and
//! GY/T 420-2025 clause 7.3.3.2 (Table 6), tag `0xD1` (209), carried in the PMT
//! ES_info loop of an AVS3 video elementary stream (`stream_type` 0xD4, see
//! [`crate::tables::pmt::StreamType::Avs3Video`]).
//!
//! The descriptor is **private** (tag 0xD1 is in the user-defined range).
//! ETSI TS 101 154 V2.10.1 section 4.1.8 requires it to be preceded by a
//! `private_data_specifier_descriptor` with value `0x41565356`
//! ([`crate::descriptors::private_data_specifier::PDS_AVSV`], 'AVSV'); the
//! Chinese documents instead scope it by a `registration_descriptor` whose
//! `format_identifier` is 'AVSV'. [`crate::descriptors::DescriptorRegistry::with_avs3_video`]
//! registers it for either context.
//!
//! Layout (descriptor_length is 7 or 8):
//!
//! ```text
//! descriptor_tag(8)=0xD1 descriptor_length(8)
//! profile_id(8) level_id(8)
//! multiple_frame_rate_flag(1) frame_rate_code(4) sample_precision(3)
//! chroma_format(2) temporal_id_flag(1) td_mode_flag(1) library_stream_flag(1)
//! library_picture_enable_flag(1) reserved(2)
//! [colour_primaries(8)]          // GY/T 420 Table 6 and the T/AI 109.6 clause 9.3.3
//!                                // semantics; absent from the T/AI 109.6 Table 9 syntax
//! transfer_characteristics(8) matrix_coefficients(8) reserved(8)
//! ```
//!
//! **Source discrepancy**: T/AI 109.6 Table 9 omits `colour_primaries` (7-byte
//! body) while its own clause 9.3.3 semantics list it and GY/T 420 Table 6
//! includes it (8-byte body). Both lengths are accepted and re-emitted
//! faithfully (`colour_primaries` is `Some` iff the body is 8 bytes). Table
//! 9/Table 6 give `td_mode_flag` 1 bit, the semantics say 2 bits; the syntax
//! (which sums to 8 bits) is followed. The spec states no value for the
//! `reserved` bits: captured on parse, all ones when hand-built.

use super::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::len::{fit_bits, need_out};
use broadcast_common::reserved::ReservedOnes;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for AVS3_video_descriptor (T/AI 109.6 Table 8: 209).
pub const TAG: u8 = 0xD1;
const HEADER_LEN: usize = 2;
const BODY_LEN_NO_COLOUR_PRIMARIES: usize = 7;
const BODY_LEN_WITH_COLOUR_PRIMARIES: usize = 8;
/// Offset of the frame-rate/precision byte in the descriptor body.
const FLAGS_HI_OFFSET: usize = 2;
/// Offset of the chroma/temporal-id byte in the descriptor body.
const FLAGS_LO_OFFSET: usize = 3;
/// Width of the fixed body prefix that precedes the transfer/matrix tail.
const FIXED_PREFIX_LEN: usize = 4;
/// Offset of `colour_primaries` in the colour-primaries variant.
const COLOUR_PRIMARIES_OFFSET: usize = 4;
/// Offset of the `reserved` byte after `matrix_coefficients` in the tail.
const RESERVED_TAIL_OFFSET: usize = 2;
const MULTIPLE_FRAME_RATE_FLAG_SHIFT: u32 = 7;
const FRAME_RATE_CODE_BITS: u32 = 4;
const FRAME_RATE_CODE_SHIFT: u32 = 3;
const FRAME_RATE_CODE_MASK: u8 = 0x0F;
const SAMPLE_PRECISION_BITS: u32 = 3;
const SAMPLE_PRECISION_MASK: u8 = 0x07;
const CHROMA_FORMAT_BITS: u32 = 2;
const CHROMA_FORMAT_SHIFT: u32 = 6;
const TEMPORAL_ID_FLAG_SHIFT: u32 = 5;
const TD_MODE_FLAG_SHIFT: u32 = 4;
const LIBRARY_STREAM_FLAG_SHIFT: u32 = 3;
const LIBRARY_PICTURE_ENABLE_FLAG_SHIFT: u32 = 2;
/// Width of the `reserved` tail of body byte 3 (2 bits).
pub const RESERVED_BITS: u32 = 2;
/// Width of the final `reserved` byte (8 bits).
pub const RESERVED_TAIL_BITS: u32 = 8;
const RESERVED2_ONES: ReservedOnes<RESERVED_BITS> = ReservedOnes::<RESERVED_BITS>::FILL;
const RESERVED8_ONES: ReservedOnes<RESERVED_TAIL_BITS> = ReservedOnes::<RESERVED_TAIL_BITS>::FILL;

/// `frame_rate_code` (4 bits, GY/T 420 Table 7 / T/AI 109.6 Table 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Avs3FrameRateCode {
    /// `0b0000` - forbidden.
    Forbidden,
    /// `0b0001` - 23.976 Hz.
    Fps23_976,
    /// `0b0010` - 24 Hz (also allows 23.976 when `multiple_frame_rate_flag`).
    Fps24,
    /// `0b0011` - 25 Hz.
    Fps25,
    /// `0b0100` - 29.97 Hz (also allows 23.976).
    Fps29_97,
    /// `0b0101` - 30 Hz (also allows 23.976, 24, 29.97).
    Fps30,
    /// `0b0110` - 50 Hz (also allows 25).
    Fps50,
    /// `0b0111` - 59.94 Hz (also allows 23.976, 29.97).
    Fps59_94,
    /// `0b1000` - 60 Hz (also allows 23.976, 24, 29.97, 30, 59.94).
    Fps60,
    /// `0b1001` - 100 Hz (also allows 50).
    Fps100,
    /// `0b1010` - 120 Hz (also allows 59.94, 60, 119.88).
    Fps120,
    /// `0b1110` - 119.88 Hz (120000/1001; T/AI 109.2-2021 `frame_rate_code`
    /// table, listed as a frame rate in T/AI 109.6 Table 10; also allows 59.94).
    Fps119_88,
    /// `0b1011` - 200 Hz (T/AI 109.2-2021 `frame_rate_code` table; **not**
    /// listed in the TS descriptor table T/AI 109.6 Table 10 / GY/T 420 Table 7).
    Fps200,
    /// `0b1100` - 240 Hz (T/AI 109.2-2021; not listed in T/AI 109.6 Table 10).
    Fps240,
    /// `0b1101` - 300 Hz (T/AI 109.2-2021; not listed in T/AI 109.6 Table 10).
    Fps300,
    /// `0b1111` - reserved; carries the value.
    Reserved(u8),
}

impl Avs3FrameRateCode {
    /// Decode from the 4-bit wire value (every value maps, lossless).
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v & 0x0F {
            0 => Self::Forbidden,
            1 => Self::Fps23_976,
            2 => Self::Fps24,
            3 => Self::Fps25,
            4 => Self::Fps29_97,
            5 => Self::Fps30,
            6 => Self::Fps50,
            7 => Self::Fps59_94,
            8 => Self::Fps60,
            9 => Self::Fps100,
            10 => Self::Fps120,
            11 => Self::Fps200,
            12 => Self::Fps240,
            13 => Self::Fps300,
            14 => Self::Fps119_88,
            other => Self::Reserved(other),
        }
    }

    /// The 4-bit wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Forbidden => 0,
            Self::Fps23_976 => 1,
            Self::Fps24 => 2,
            Self::Fps25 => 3,
            Self::Fps29_97 => 4,
            Self::Fps30 => 5,
            Self::Fps50 => 6,
            Self::Fps59_94 => 7,
            Self::Fps60 => 8,
            Self::Fps100 => 9,
            Self::Fps120 => 10,
            Self::Fps200 => 11,
            Self::Fps240 => 12,
            Self::Fps300 => 13,
            Self::Fps119_88 => 14,
            Self::Reserved(v) => v,
        }
    }

    /// Spec label (frame rate in Hz).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Forbidden => "forbidden",
            Self::Fps23_976 => "23.976 Hz",
            Self::Fps24 => "24 Hz",
            Self::Fps25 => "25 Hz",
            Self::Fps29_97 => "29.97 Hz",
            Self::Fps30 => "30 Hz",
            Self::Fps50 => "50 Hz",
            Self::Fps59_94 => "59.94 Hz",
            Self::Fps60 => "60 Hz",
            Self::Fps100 => "100 Hz",
            Self::Fps120 => "120 Hz",
            Self::Fps200 => "200 Hz",
            Self::Fps240 => "240 Hz",
            Self::Fps300 => "300 Hz",
            Self::Fps119_88 => "119.88 Hz",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(Avs3FrameRateCode, Reserved);

/// AVS3_video_descriptor (T/AI 109.6 Table 9 / GY/T 420 Table 6).
///
/// Field semantics (`profile_id`, `level_id`, `sample_precision`,
/// `chroma_format`, `colour_primaries`, `transfer_characteristics`,
/// `matrix_coefficients`) are those of the AVS3 video specification (T/AI
/// 109.2 / GY/T 368) and are carried as their numeric codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Avs3VideoDescriptor {
    /// `profile_id` (8 bits).
    pub profile_id: u8,
    /// `level_id` (8 bits).
    pub level_id: u8,
    /// `multiple_frame_rate_flag` (1 bit): other frame rates may occur too.
    pub multiple_frame_rate_flag: bool,
    /// `frame_rate_code` (4 bits).
    pub frame_rate_code: Avs3FrameRateCode,
    /// `sample_precision` (3 bits).
    pub sample_precision: u8,
    /// `chroma_format` (2 bits).
    pub chroma_format: u8,
    /// `temporal_id_flag` (1 bit).
    pub temporal_id_flag: bool,
    /// `td_mode_flag` (1 bit per the syntax tables).
    pub td_mode_flag: bool,
    /// `library_stream_flag` (1 bit): the ES is a library (knowledge) stream.
    pub library_stream_flag: bool,
    /// `library_picture_enable_flag` (1 bit).
    pub library_picture_enable_flag: bool,
    /// `reserved` (2 bits, right-aligned) after `library_picture_enable_flag`.
    pub reserved: ReservedOnes<RESERVED_BITS>,
    /// `colour_primaries` (8 bits); `Some` iff the descriptor body is 8 bytes.
    pub colour_primaries: Option<u8>,
    /// `transfer_characteristics` (8 bits).
    pub transfer_characteristics: u8,
    /// `matrix_coefficients` (8 bits).
    pub matrix_coefficients: u8,
    /// Final `reserved` byte (8 bits).
    pub reserved_tail: ReservedOnes<RESERVED_TAIL_BITS>,
}

impl Avs3VideoDescriptor {
    /// Build a descriptor with all-ones defaults for the two `reserved` runs
    /// (the spec states no value). `colour_primaries` is included (GY/T 420
    /// 8-byte form).
    #[must_use]
    pub const fn new(
        profile_id: u8,
        level_id: u8,
        multiple_frame_rate_flag: bool,
        frame_rate_code: Avs3FrameRateCode,
        sample_precision: u8,
        chroma_format: u8,
        colour_primaries: u8,
        transfer_characteristics: u8,
        matrix_coefficients: u8,
    ) -> Self {
        Self {
            profile_id,
            level_id,
            multiple_frame_rate_flag,
            frame_rate_code,
            sample_precision,
            chroma_format,
            temporal_id_flag: false,
            td_mode_flag: false,
            library_stream_flag: false,
            library_picture_enable_flag: false,
            reserved: RESERVED2_ONES,
            colour_primaries: Some(colour_primaries),
            transfer_characteristics,
            matrix_coefficients,
            reserved_tail: RESERVED8_ONES,
        }
    }

    fn body_len(&self) -> usize {
        if self.colour_primaries.is_some() {
            BODY_LEN_WITH_COLOUR_PRIMARIES
        } else {
            BODY_LEN_NO_COLOUR_PRIMARIES
        }
    }
}

fn bit(b: u8, shift: u32) -> bool {
    (b >> shift) & 1 != 0
}

impl<'a> Parse<'a> for Avs3VideoDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "Avs3VideoDescriptor",
            "unexpected tag for AVS3_video_descriptor",
        )?;
        let with_cp = match body.len() {
            BODY_LEN_NO_COLOUR_PRIMARIES => false,
            BODY_LEN_WITH_COLOUR_PRIMARIES => true,
            _ => {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "AVS3_video_descriptor body must be 7 or 8 bytes",
                });
            }
        };
        let b2 = body[FLAGS_HI_OFFSET];
        let b3 = body[FLAGS_LO_OFFSET];
        let (colour_primaries, rest) = if with_cp {
            (
                Some(body[COLOUR_PRIMARIES_OFFSET]),
                &body[COLOUR_PRIMARIES_OFFSET + 1..],
            )
        } else {
            (None, &body[FIXED_PREFIX_LEN..])
        };
        Ok(Self {
            profile_id: body[0],
            level_id: body[1],
            multiple_frame_rate_flag: bit(b2, MULTIPLE_FRAME_RATE_FLAG_SHIFT),
            frame_rate_code: Avs3FrameRateCode::from_u8(
                (b2 >> FRAME_RATE_CODE_SHIFT) & FRAME_RATE_CODE_MASK,
            ),
            sample_precision: b2 & SAMPLE_PRECISION_MASK,
            chroma_format: b3 >> CHROMA_FORMAT_SHIFT,
            temporal_id_flag: bit(b3, TEMPORAL_ID_FLAG_SHIFT),
            td_mode_flag: bit(b3, TD_MODE_FLAG_SHIFT),
            library_stream_flag: bit(b3, LIBRARY_STREAM_FLAG_SHIFT),
            library_picture_enable_flag: bit(b3, LIBRARY_PICTURE_ENABLE_FLAG_SHIFT),
            reserved: ReservedOnes::from_bits(u64::from(b3)),
            colour_primaries,
            transfer_characteristics: rest[0],
            matrix_coefficients: rest[1],
            reserved_tail: ReservedOnes::from_bits(u64::from(rest[RESERVED_TAIL_OFFSET])),
        })
    }
}

impl Serialize for Avs3VideoDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.body_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        let frc = fit_bits(
            u64::from(self.frame_rate_code.to_u8()),
            FRAME_RATE_CODE_BITS,
            "frame_rate_code",
        )? as u8;
        let sp = fit_bits(
            u64::from(self.sample_precision),
            SAMPLE_PRECISION_BITS,
            "sample_precision",
        )? as u8;
        let cf = fit_bits(
            u64::from(self.chroma_format),
            CHROMA_FORMAT_BITS,
            "chroma_format",
        )? as u8;
        let rsv = self.reserved.get() as u8;
        let b2 = (u8::from(self.multiple_frame_rate_flag) << MULTIPLE_FRAME_RATE_FLAG_SHIFT)
            | (frc << FRAME_RATE_CODE_SHIFT)
            | sp;
        let b3 = (cf << CHROMA_FORMAT_SHIFT)
            | (u8::from(self.temporal_id_flag) << TEMPORAL_ID_FLAG_SHIFT)
            | (u8::from(self.td_mode_flag) << TD_MODE_FLAG_SHIFT)
            | (u8::from(self.library_stream_flag) << LIBRARY_STREAM_FLAG_SHIFT)
            | (u8::from(self.library_picture_enable_flag) << LIBRARY_PICTURE_ENABLE_FLAG_SHIFT)
            | rsv;
        write_descriptor_header(buf, TAG, self.body_len())?;
        let mut i = HEADER_LEN;
        for b in [self.profile_id, self.level_id, b2, b3] {
            buf[i] = b;
            i += 1;
        }
        if let Some(cp) = self.colour_primaries {
            buf[i] = cp;
            i += 1;
        }
        for b in [
            self.transfer_characteristics,
            self.matrix_coefficients,
            self.reserved_tail.get() as u8,
        ] {
            buf[i] = b;
            i += 1;
        }
        Ok(i)
    }
}

impl crate::traits::DescriptorDef<'_> for Avs3VideoDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "AVS3_VIDEO";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    fn round_trip(bytes: &[u8]) -> Avs3VideoDescriptor {
        let d = Avs3VideoDescriptor::parse(bytes).unwrap();
        let mut out = vec![0u8; d.serialized_len()];
        let n = d.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..n], bytes, "byte-exact round trip");
        d
    }

    /// GY/T 420-2025 Table 6 (8-byte body, with colour_primaries): profile
    /// 0x22, level 0x42, multiple_frame_rate=1 frame_rate_code=0b0101 (30 Hz)
    /// sample_precision=2, chroma_format=1 temporal_id=0 td_mode=0
    /// library_stream=1 library_picture_enable=0 reserved=0b11,
    /// colour_primaries 9, transfer 14, matrix 9, reserved 0xFF.
    #[test]
    fn gyt_420_table_6_eight_byte_form() {
        let bytes = [TAG, 8, 0x22, 0x42, 0xAA, 0x4B, 9, 14, 9, 0xFF];
        let d = round_trip(&bytes);
        assert_eq!((d.profile_id, d.level_id), (0x22, 0x42));
        assert!(d.multiple_frame_rate_flag);
        assert_eq!(d.frame_rate_code, Avs3FrameRateCode::Fps30);
        assert_eq!((d.sample_precision, d.chroma_format), (2, 1));
        assert!(d.library_stream_flag && !d.library_picture_enable_flag);
        assert_eq!(d.colour_primaries, Some(9));
        assert_eq!((d.transfer_characteristics, d.matrix_coefficients), (14, 9));
        assert_eq!((d.reserved.get(), d.reserved_tail.get()), (3, 0xFF));
    }

    /// T/AI 109.6-2022 Table 9 (7-byte body, no colour_primaries).
    #[test]
    fn t_ai_109_6_table_9_seven_byte_form() {
        let bytes = [TAG, 7, 0x20, 0x40, 0x19, 0x34, 1, 1, 0x00];
        let d = round_trip(&bytes);
        assert_eq!(d.colour_primaries, None);
        assert_eq!(d.frame_rate_code, Avs3FrameRateCode::Fps25);
        assert!(d.temporal_id_flag && d.td_mode_flag);
        assert!(!d.library_stream_flag && d.library_picture_enable_flag);
        assert_eq!((d.reserved.get(), d.reserved_tail.get()), (0, 0));
    }

    #[test]
    fn rejects_wrong_tag_length_and_truncation() {
        assert!(matches!(
            Avs3VideoDescriptor::parse(&[0xD0, 7, 0, 0, 0, 0, 0, 0, 0]),
            Err(Error::InvalidDescriptor { tag: 0xD0, .. })
        ));
        assert!(matches!(
            Avs3VideoDescriptor::parse(&[TAG, 6, 0, 0, 0, 0, 0, 0]),
            Err(Error::InvalidDescriptor { tag: TAG, .. })
        ));
        assert!(matches!(
            Avs3VideoDescriptor::parse(&[TAG, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            Err(Error::InvalidDescriptor { tag: TAG, .. })
        ));
        assert!(matches!(
            Avs3VideoDescriptor::parse(&[TAG, 8, 0, 0]),
            Err(Error::BufferTooShort(..))
        ));
    }

    #[test]
    fn serialize_rejects_overwide_fields() {
        let mut d = Avs3VideoDescriptor::new(1, 1, false, Avs3FrameRateCode::Fps25, 1, 1, 1, 1, 1);
        d.sample_precision = 8;
        assert!(matches!(
            d.serialize_into(&mut [0u8; 16]),
            Err(Error::FieldOverflow(..))
        ));
        d.sample_precision = 1;
        d.chroma_format = 4;
        assert!(matches!(
            &(d.serialize_into(&mut [0u8; 16])),
            Err(crate::error::Error::FieldOverflow(_))
        ));
        d.chroma_format = 1;
        d.frame_rate_code = Avs3FrameRateCode::Reserved(0x1F);
        assert!(matches!(
            &(d.serialize_into(&mut [0u8; 16])),
            Err(crate::error::Error::FieldOverflow(_))
        ));
    }

    #[test]
    fn frame_rate_code_is_lossless_and_labelled() {
        for v in 0u8..16 {
            assert_eq!(Avs3FrameRateCode::from_u8(v).to_u8(), v);
        }
        assert_eq!(Avs3FrameRateCode::Fps29_97.name(), "29.97 Hz");
        // 0b1110 = 120000/1001 (T/AI 109.2-2021 frame_rate_code table).
        assert_eq!(
            Avs3FrameRateCode::from_u8(0b1110),
            Avs3FrameRateCode::Fps119_88
        );
        assert_eq!(Avs3FrameRateCode::from_u8(0b1110).name(), "119.88 Hz");
        assert_eq!(Avs3FrameRateCode::Fps119_88.to_string(), "119.88 Hz");
        assert_eq!(
            Avs3FrameRateCode::from_u8(0b1111),
            Avs3FrameRateCode::Reserved(0b1111)
        );
        assert_eq!(
            Avs3FrameRateCode::Reserved(15).to_string(),
            "reserved(0x0F)"
        );
    }

    #[test]
    fn built_defaults_are_all_ones() {
        let d = Avs3VideoDescriptor::new(1, 2, true, Avs3FrameRateCode::Fps50, 2, 1, 9, 14, 9);
        assert_eq!((d.reserved.get(), d.reserved_tail.get()), (0b11, 0xFF));
        assert_eq!(d.serialized_len(), 10);
    }

    /// Both `reserved` runs are typed wide enough to hold their wire field but
    /// no wider, so an over-wide value cannot be stored.
    #[test]
    fn reserved_fields_cannot_overflow() {
        assert_eq!(
            ReservedOnes::<RESERVED_BITS>::from_bits(u64::MAX).get(),
            0b11
        );
        assert_eq!(
            ReservedOnes::<RESERVED_TAIL_BITS>::from_bits(u64::MAX).get(),
            0xFF
        );
    }
}
