//! DTS-UHD Descriptor — ETSI EN 300 468 Annex G.5, Table G.15 (tag_extension 0x21).
use super::*;

impl<'a> ExtensionBodyDef<'a> for DtsUhd<'a> {
    const TAG_EXTENSION: u8 = 0x21;
    const NAME: &'static str = "DTS_UHD";
}

/// DTS-UHD descriptor body (Table G.15, Annex G.5).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct DtsUhd<'a> {
    /// `DecoderProfileCode`(6) — decoder profile = value + 2.
    pub decoder_profile_code: u8,
    /// `FrameDurationCode`(2) — PCM-sample frame duration at 48 kHz base (Table G.16).
    pub frame_duration_code: FrameDurationCode,
    /// `MaxPayloadCode`(3) — maximum audio payload size (Table G.17).
    pub max_payload_code: MaxPayloadCode,
    /// `DTS_reserved`(2) — preserved for byte-exact round-trip.
    pub dts_reserved: u8,
    /// `StreamIndex`(3) — stream priority (0 = main, 1..7 = aux).
    pub stream_index: u8,
    /// `codec_selector_byte` run — codec-defined selector field.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub codec_selector: &'a [u8],
}

impl DtsUhd<'_> {
    /// `DecoderProfile = DecoderProfileCode + 2`.
    #[must_use]
    pub fn decoder_profile(&self) -> u8 {
        self.decoder_profile_code + 2
    }
}

/// `FrameDurationCode` — Table G.16.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FrameDurationCode {
    /// 512 samples.
    Samples512,
    /// 1 024 samples.
    Samples1024,
    /// 2 048 samples.
    Samples2048,
    /// 4 096 samples.
    Samples4096,
}

impl FrameDurationCode {
    /// Construct from a raw `u8`; total, lossless (2-bit field).
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => FrameDurationCode::Samples512,
            1 => FrameDurationCode::Samples1024,
            2 => FrameDurationCode::Samples2048,
            3 => FrameDurationCode::Samples4096,
            _ => FrameDurationCode::Samples4096, // fallback for invalid values; never reached (2-bit field)
        }
    }

    /// Inverse of `from_u8`.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            FrameDurationCode::Samples512 => 0,
            FrameDurationCode::Samples1024 => 1,
            FrameDurationCode::Samples2048 => 2,
            FrameDurationCode::Samples4096 => 3,
        }
    }

    /// Human-readable spec name per Table G.16.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            FrameDurationCode::Samples512 => "512 samples",
            FrameDurationCode::Samples1024 => "1 024 samples",
            FrameDurationCode::Samples2048 => "2 048 samples",
            FrameDurationCode::Samples4096 => "4 096 samples",
        }
    }
}
dvb_common::impl_spec_display!(FrameDurationCode);

/// `MaxPayloadCode` — Table G.17.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MaxPayloadCode {
    /// 2 048 byte.
    Byte2048,
    /// 4 096 byte.
    Byte4096,
    /// 8 192 byte.
    Byte8192,
    /// 16 384 byte.
    Byte16384,
    /// 32 768 byte.
    Byte32768,
    /// 65 536 byte.
    Byte65536,
    /// 131 072 byte.
    Byte131072,
    /// Reserved for future use.
    Reserved(u8),
}

impl MaxPayloadCode {
    /// Construct from a raw `u8`; total, lossless (3-bit field, reserved catch-all).
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => MaxPayloadCode::Byte2048,
            1 => MaxPayloadCode::Byte4096,
            2 => MaxPayloadCode::Byte8192,
            3 => MaxPayloadCode::Byte16384,
            4 => MaxPayloadCode::Byte32768,
            5 => MaxPayloadCode::Byte65536,
            6 => MaxPayloadCode::Byte131072,
            other => MaxPayloadCode::Reserved(other),
        }
    }

    /// Inverse of `from_u8`; `Self::Reserved` emits its stored value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            MaxPayloadCode::Byte2048 => 0,
            MaxPayloadCode::Byte4096 => 1,
            MaxPayloadCode::Byte8192 => 2,
            MaxPayloadCode::Byte16384 => 3,
            MaxPayloadCode::Byte32768 => 4,
            MaxPayloadCode::Byte65536 => 5,
            MaxPayloadCode::Byte131072 => 6,
            MaxPayloadCode::Reserved(v) => v,
        }
    }

    /// Human-readable spec name per Table G.17.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            MaxPayloadCode::Byte2048 => "2 048 byte",
            MaxPayloadCode::Byte4096 => "4 096 byte",
            MaxPayloadCode::Byte8192 => "8 192 byte",
            MaxPayloadCode::Byte16384 => "16 384 byte",
            MaxPayloadCode::Byte32768 => "32 768 byte",
            MaxPayloadCode::Byte65536 => "65 536 byte",
            MaxPayloadCode::Byte131072 => "131 072 byte",
            MaxPayloadCode::Reserved(_) => "reserved for future use",
        }
    }
}
dvb_common::impl_spec_display!(MaxPayloadCode, Reserved);

/// Fixed length before the `codec_selector` bytes.
const DTS_UHD_FIXED_LEN: usize = 2;

impl<'a> Parse<'a> for DtsUhd<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < DTS_UHD_FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: DTS_UHD_FIXED_LEN,
                have: sel.len(),
                what: "DTS-UHD descriptor body",
            });
        }
        let decoder_profile_code = (sel[0] >> 2) & 0x3F;
        let frame_duration_code = FrameDurationCode::from_u8(sel[0] & 0x03);
        let max_payload_code = MaxPayloadCode::from_u8((sel[1] >> 5) & 0x07);
        let dts_reserved = (sel[1] >> 3) & 0x03;
        let stream_index = sel[1] & 0x07;
        Ok(DtsUhd {
            decoder_profile_code,
            frame_duration_code,
            max_payload_code,
            dts_reserved,
            stream_index,
            codec_selector: &sel[DTS_UHD_FIXED_LEN..],
        })
    }
}

impl Serialize for DtsUhd<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        DTS_UHD_FIXED_LEN + self.codec_selector.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = (self.decoder_profile_code << 2) | (self.frame_duration_code.to_u8() & 0x03);
        buf[1] = (self.max_payload_code.to_u8() << 5)
            | ((self.dts_reserved & 0x03) << 3)
            | (self.stream_index & 0x07);
        buf[DTS_UHD_FIXED_LEN..len].copy_from_slice(self.codec_selector);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor};

    #[test]
    fn decodes_frame_duration_code() {
        assert_eq!(FrameDurationCode::from_u8(0).name(), "512 samples");
        assert_eq!(FrameDurationCode::from_u8(2).name(), "2 048 samples");
    }

    #[test]
    fn decodes_max_payload_code() {
        assert_eq!(MaxPayloadCode::from_u8(4).name(), "32 768 byte");
    }

    #[test]
    fn parse_dts_uhd_structured() {
        // DPC=1, FDC=2, MPC=2, DTS_rsv=0, SI=3, codec_selector=ABCD
        let sel = [0x06, 0x43, 0xAB, 0xCD];
        let bytes = wrap(0x21, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::DtsUhd(b) => {
                assert_eq!(b.decoder_profile_code, 1);
                assert_eq!(b.decoder_profile(), 3);
                assert_eq!(b.frame_duration_code, FrameDurationCode::Samples2048);
                assert_eq!(b.max_payload_code, MaxPayloadCode::Byte8192);
                assert_eq!(b.dts_reserved, 0);
                assert_eq!(b.stream_index, 3);
                assert_eq!(b.codec_selector, &[0xAB, 0xCD]);
            }
            other => panic!("expected DtsUhd, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_dts_uhd_no_codec_selector() {
        let sel = [0x06, 0x43];
        let bytes = wrap(0x21, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::DtsUhd(b) => {
                assert!(b.codec_selector.is_empty());
            }
            other => panic!("expected DtsUhd, got {other:?}"),
        }
        round_trip(&d);
    }
}
