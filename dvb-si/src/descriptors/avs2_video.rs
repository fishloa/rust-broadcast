//! AVS2_video_descriptor — GY/T 420-2025 clause 7.2.3 (Table 3), tag `0x40`
//! (64), carried in the PMT ES_info loop of an AVS2 video elementary stream
//! (`stream_type` 0xD2, see [`crate::tables::pmt::StreamType::Avs2Video`]).
//!
//! Tag 0x40 is also the DVB `network_name_descriptor`, so the AVS2 descriptor is
//! scoped: GY/T 420 clause 7.2.2 puts a `registration_descriptor` with
//! `format_identifier` 'AVSV' (0x41565356) in the PMT first; the registry also
//! accepts a private_data_specifier 'AVSV'. Enable it with
//! [`crate::descriptors::DescriptorRegistry::with_avs2_video`]; it is never
//! auto-dispatched (the built-in tag 0x40 stays `network_name`).
//!
//! Layout (descriptor_length is 5):
//!
//! ```text
//! descriptor_tag(8)=0x40 descriptor_length(8)
//! profile_id(8) level_id(8) extension_layer_number(8)
//! multiple_frame_rate_flag(1) frame_rate_code(4) AVS_still_present(1) chroma_format(2)
//! sample_precision(3) reserved(5)
//! ```
//!
//! The standard states no value for `reserved`: captured on parse, all ones
//! when hand-built. `extension_layer_number` "is 0" per the semantics but is
//! preserved verbatim. `frame_rate_code` is Table 4, identical to the AVS3
//! table ([`crate::descriptors::avs3_video::Avs3FrameRateCode`]).

use super::avs3_video::Avs3FrameRateCode;
use super::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use broadcast_common::len::{fit_bits, need_out};
use broadcast_common::reserved::ReservedOnes;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for AVS2_video_descriptor (GY/T 420 Table 3: 64).
pub const TAG: u8 = 0x40;
const HEADER_LEN: usize = 2;
const BODY_LEN: usize = 5;
/// Offset of `extension_layer_number` in the descriptor body.
const EXTENSION_LAYER_NUMBER_OFFSET: usize = 2;
/// Offset of the frame-rate/still/chroma byte in the descriptor body.
const FLAGS_HI_OFFSET: usize = 3;
/// Offset of the sample-precision byte in the descriptor body.
const FLAGS_LO_OFFSET: usize = 4;
const MULTIPLE_FRAME_RATE_FLAG_MASK: u8 = 0x80;
const MULTIPLE_FRAME_RATE_FLAG_SHIFT: u8 = 7;
const FRAME_RATE_CODE_BITS: u32 = 4;
const FRAME_RATE_CODE_SHIFT: u8 = 3;
const FRAME_RATE_CODE_MASK: u8 = 0x0F;
const AVS_STILL_PRESENT_MASK: u8 = 0x04;
const AVS_STILL_PRESENT_SHIFT: u8 = 2;
const CHROMA_FORMAT_BITS: u32 = 2;
const CHROMA_FORMAT_MASK: u8 = 0x03;
const SAMPLE_PRECISION_BITS: u32 = 3;
const SAMPLE_PRECISION_SHIFT: u8 = 5;
/// Width of the `reserved` tail (5 bits).
const RESERVED_BITS: u32 = 5;
/// Spec/default of the 5-bit `reserved` field: all ones when hand-built
/// (the standard states no value; parse preserves whatever was sent).
const RESERVED5_ONES: ReservedOnes<RESERVED_BITS> = ReservedOnes::<RESERVED_BITS>::FILL;

/// AVS2_video_descriptor (GY/T 420-2025 Table 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Avs2VideoDescriptor {
    /// `profile_id` (8 bits, GY/T 299.1-2016).
    pub profile_id: u8,
    /// `level_id` (8 bits, GY/T 299.1-2016).
    pub level_id: u8,
    /// `extension_layer_number` (8 bits; the semantics say it is 0).
    pub extension_layer_number: u8,
    /// `multiple_frame_rate_flag` (1 bit).
    pub multiple_frame_rate_flag: bool,
    /// `frame_rate_code` (4 bits, Table 4).
    pub frame_rate_code: Avs3FrameRateCode,
    /// `AVS_still_present` (1 bit): the stream carries only still pictures.
    pub avs_still_present: bool,
    /// `chroma_format` (2 bits, GY/T 299.1-2016).
    pub chroma_format: u8,
    /// `sample_precision` (3 bits, GY/T 299.1-2016).
    pub sample_precision: u8,
    /// `reserved` (5 bits, right-aligned).
    pub reserved: ReservedOnes<RESERVED_BITS>,
}

impl Avs2VideoDescriptor {
    /// Build a descriptor (`extension_layer_number` 0, all-ones reserved).
    #[must_use]
    pub const fn new(
        profile_id: u8,
        level_id: u8,
        multiple_frame_rate_flag: bool,
        frame_rate_code: Avs3FrameRateCode,
        avs_still_present: bool,
        chroma_format: u8,
        sample_precision: u8,
    ) -> Self {
        Self {
            profile_id,
            level_id,
            extension_layer_number: 0,
            multiple_frame_rate_flag,
            frame_rate_code,
            avs_still_present,
            chroma_format,
            sample_precision,
            reserved: RESERVED5_ONES,
        }
    }
}

impl<'a> Parse<'a> for Avs2VideoDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "Avs2VideoDescriptor",
            "unexpected tag for AVS2_video_descriptor",
        )?;
        if body.len() != BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "AVS2_video_descriptor body must be 5 bytes",
            });
        }
        let (b3, b4) = (body[FLAGS_HI_OFFSET], body[FLAGS_LO_OFFSET]);
        Ok(Self {
            profile_id: body[0],
            level_id: body[1],
            extension_layer_number: body[EXTENSION_LAYER_NUMBER_OFFSET],
            multiple_frame_rate_flag: b3 & MULTIPLE_FRAME_RATE_FLAG_MASK != 0,
            frame_rate_code: Avs3FrameRateCode::from_u8((b3 >> FRAME_RATE_CODE_SHIFT) & FRAME_RATE_CODE_MASK),
            avs_still_present: b3 & AVS_STILL_PRESENT_MASK != 0,
            chroma_format: b3 & CHROMA_FORMAT_MASK,
            sample_precision: b4 >> SAMPLE_PRECISION_SHIFT,
            reserved: ReservedOnes::from_bits(u64::from(b4)),
        })
    }
}

impl Serialize for Avs2VideoDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        need_out(buf, len)?;
        let frc = fit_bits(
            u64::from(self.frame_rate_code.to_u8()),
            FRAME_RATE_CODE_BITS,
            "frame_rate_code",
        )? as u8;
        let cf = fit_bits(
            u64::from(self.chroma_format),
            CHROMA_FORMAT_BITS,
            "chroma_format",
        )? as u8;
        let sp = fit_bits(
            u64::from(self.sample_precision),
            SAMPLE_PRECISION_BITS,
            "sample_precision",
        )? as u8;
        let rsv = self.reserved.get() as u8;
        write_descriptor_header(buf, TAG, BODY_LEN)?;
        buf[HEADER_LEN] = self.profile_id;
        buf[HEADER_LEN + 1] = self.level_id;
        buf[HEADER_LEN + EXTENSION_LAYER_NUMBER_OFFSET] = self.extension_layer_number;
        buf[HEADER_LEN + FLAGS_HI_OFFSET] =
            (u8::from(self.multiple_frame_rate_flag) << MULTIPLE_FRAME_RATE_FLAG_SHIFT)
                | (frc << FRAME_RATE_CODE_SHIFT)
                | (u8::from(self.avs_still_present) << AVS_STILL_PRESENT_SHIFT)
                | cf;
        buf[HEADER_LEN + FLAGS_LO_OFFSET] = (sp << SAMPLE_PRECISION_SHIFT) | rsv;
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for Avs2VideoDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "AVS2_VIDEO";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(bytes: &[u8]) -> Avs2VideoDescriptor {
        let d = Avs2VideoDescriptor::parse(bytes).unwrap();
        let mut out = [0u8; 16];
        let n = d.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..n], bytes, "byte-exact round trip");
        d
    }

    /// GY/T 420-2025 Table 3: profile 0x20, level 0x42, extension_layer_number
    /// 0, multiple_frame_rate_flag 1, frame_rate_code 0b0101 (30 Hz),
    /// AVS_still_present 0, chroma_format 1, sample_precision 2, reserved all ones.
    #[test]
    fn gyt_420_table_3_vector() {
        let bytes = [TAG, 5, 0x20, 0x42, 0x00, 0xA9, 0x5F];
        let d = round_trip(&bytes);
        assert_eq!(
            (d.profile_id, d.level_id, d.extension_layer_number),
            (0x20, 0x42, 0)
        );
        assert!(d.multiple_frame_rate_flag);
        assert_eq!(d.frame_rate_code, Avs3FrameRateCode::Fps30);
        assert!(!d.avs_still_present);
        assert_eq!(
            (d.chroma_format, d.sample_precision, d.reserved.get()),
            (1, 2, 0x1F)
        );
    }

    #[test]
    fn still_picture_flag_round_trips() {
        let bytes = [TAG, 5, 0x10, 0x20, 0x00, 0x1C, 0x5F];
        let d = round_trip(&bytes);
        assert!(d.avs_still_present && !d.multiple_frame_rate_flag);
        assert_eq!(d.frame_rate_code, Avs3FrameRateCode::Fps25);
    }

    #[test]
    fn rejects_wrong_tag_length_and_truncation() {
        assert!(matches!(
            Avs2VideoDescriptor::parse(&[0x41, 5, 0, 0, 0, 0, 0]),
            Err(Error::InvalidDescriptor { tag: 0x41, .. })
        ));
        assert!(matches!(
            Avs2VideoDescriptor::parse(&[TAG, 4, 0, 0, 0, 0]),
            Err(Error::InvalidDescriptor { tag: TAG, .. })
        ));
        assert!(matches!(
            Avs2VideoDescriptor::parse(&[TAG, 6, 0, 0, 0, 0, 0, 0]),
            Err(Error::InvalidDescriptor { tag: TAG, .. })
        ));
        assert!(matches!(
            Avs2VideoDescriptor::parse(&[TAG, 5, 0]),
            Err(Error::BufferTooShort(..))
        ));
    }

    #[test]
    fn serialize_rejects_overwide_fields() {
        let mut d = Avs2VideoDescriptor::new(1, 1, false, Avs3FrameRateCode::Fps25, false, 1, 1);
        d.sample_precision = 8;
        assert!(matches!(
            &(d.serialize_into(&mut [0u8; 8])),
            Err(crate::error::Error::FieldOverflow(_))
        ));
        d.sample_precision = 1;
        d.chroma_format = 4;
        assert!(matches!(
            d.serialize_into(&mut [0u8; 8]),
            Err(Error::FieldOverflow(..))
        ));
    }

    /// `reserved` is a 5-bit typed field, so a value wider than the wire field
    /// cannot be stored (the old raw `u8` accepted `0x20` and had to re-mask).
    #[test]
    fn reserved_field_is_five_bits() {
        assert_eq!(ReservedOnes::<RESERVED_BITS>::from_bits(0x20).get(), 0);
        assert_eq!(
            ReservedOnes::<RESERVED_BITS>::from_bits(u64::MAX).get(),
            0x1F
        );
    }

    #[test]
    fn built_defaults() {
        let d = Avs2VideoDescriptor::new(0x20, 0x42, true, Avs3FrameRateCode::Fps30, false, 1, 2);
        let mut out = [0u8; 7];
        d.serialize_into(&mut out).unwrap();
        assert_eq!(out, [TAG, 5, 0x20, 0x42, 0x00, 0xA9, 0x5F]);
    }
}
