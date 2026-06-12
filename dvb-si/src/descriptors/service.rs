//! Service Descriptor — ETSI EN 300 468 §6.2.33 (tag 0x48).
//!
//! Carried inside SDT. Provides the provider and service name plus a
//! service_type byte classifying the service (TV SD, TV HD, radio, data, …).

use super::descriptor_body;
use crate::error::{Error, Result};
use crate::text::DvbText;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for service_descriptor.
pub const TAG: u8 = 0x48;
const HEADER_LEN: usize = 2;

/// Service type — ETSI EN 300 468 Table 89.
///
/// # Examples
/// ```
/// use dvb_si::descriptors::service::ServiceType;
///
/// assert_eq!(ServiceType::from_u8(0x01).name(), "digital television service");
/// assert_eq!(ServiceType::from_u8(0x19).to_u8(), 0x19); // advanced-codec HD, lossless
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ServiceType {
    /// 0x01 — digital television service.
    DigitalTelevision,
    /// 0x02 — digital radio sound service.
    DigitalRadioSound,
    /// 0x03 — teletext service.
    Teletext,
    /// 0x04 — NVOD reference service.
    NvodReference,
    /// 0x05 — NVOD time-shifted service.
    NvodTimeShifted,
    /// 0x06 — mosaic service.
    Mosaic,
    /// 0x07 — FM radio service.
    FmRadio,
    /// 0x08 — DVB SRM service.
    DvbSrm,
    /// 0x0A — advanced codec digital radio sound service.
    AdvancedCodecDigitalRadio,
    /// 0x0B — H.264/AVC mosaic service.
    AvcMosaic,
    /// 0x0C — data broadcast service.
    DataBroadcast,
    /// 0x0E — RCS Map.
    RcsMap,
    /// 0x0F — RCS FLS.
    RcsFls,
    /// 0x10 — DVB MHP service.
    Mhp,
    /// 0x11 — HD digital television service.
    HdDigitalTelevision,
    /// 0x16 — H.264/AVC SD digital television service.
    AvcSdDigitalTelevision,
    /// 0x17 — H.264/AVC SD NVOD time-shifted service.
    AvcSdNvodTimeShifted,
    /// 0x18 — H.264/AVC SD NVOD reference service.
    AvcSdNvodReference,
    /// 0x19 — H.264/AVC HD digital television service.
    AvcHdDigitalTelevision,
    /// 0x1A — H.264/AVC HD NVOD time-shifted service.
    AvcHdNvodTimeShifted,
    /// 0x1B — H.264/AVC HD NVOD reference service.
    AvcHdNvodReference,
    /// 0x1C — H.264/AVC frame compatible plano-stereoscopic HD digital
    /// television service.
    AvcFrameCompatiblePlanoStereoscopicHd,
    /// 0x1D — H.264/AVC frame compatible plano-stereoscopic HD NVOD
    /// time-shifted service.
    AvcFrameCompatiblePlanoStereoscopicHdNvodTimeShifted,
    /// 0x1E — H.264/AVC frame compatible plano-stereoscopic HD NVOD
    /// reference service.
    AvcFrameCompatiblePlanoStereoscopicHdNvodReference,
    /// 0x1F — HEVC digital television service.
    HevcDigitalTelevision,
    /// 0x20 — HEVC UHD digital television service.
    HevcUhdDigitalTelevision,
    /// 0x21 — VVC digital television service.
    VvcDigitalTelevision,
    /// 0x22 — AVS3 digital television service.
    Avs3DigitalTelevision,
    /// Reserved/unallocated wire value, preserved verbatim for round-trip.
    Reserved(u8),
}

impl ServiceType {
    #[must_use]
    /// Creates a value from a wire byte, preserving every possible
    /// byte value for lossless round-trip.
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::DigitalTelevision,
            0x02 => Self::DigitalRadioSound,
            0x03 => Self::Teletext,
            0x04 => Self::NvodReference,
            0x05 => Self::NvodTimeShifted,
            0x06 => Self::Mosaic,
            0x07 => Self::FmRadio,
            0x08 => Self::DvbSrm,
            0x0A => Self::AdvancedCodecDigitalRadio,
            0x0B => Self::AvcMosaic,
            0x0C => Self::DataBroadcast,
            0x0E => Self::RcsMap,
            0x0F => Self::RcsFls,
            0x10 => Self::Mhp,
            0x11 => Self::HdDigitalTelevision,
            0x16 => Self::AvcSdDigitalTelevision,
            0x17 => Self::AvcSdNvodTimeShifted,
            0x18 => Self::AvcSdNvodReference,
            0x19 => Self::AvcHdDigitalTelevision,
            0x1A => Self::AvcHdNvodTimeShifted,
            0x1B => Self::AvcHdNvodReference,
            0x1C => Self::AvcFrameCompatiblePlanoStereoscopicHd,
            0x1D => Self::AvcFrameCompatiblePlanoStereoscopicHdNvodTimeShifted,
            0x1E => Self::AvcFrameCompatiblePlanoStereoscopicHdNvodReference,
            0x1F => Self::HevcDigitalTelevision,
            0x20 => Self::HevcUhdDigitalTelevision,
            0x21 => Self::VvcDigitalTelevision,
            0x22 => Self::Avs3DigitalTelevision,
            v => Self::Reserved(v),
        }
    }

    #[must_use]
    /// Returns the wire byte for this value.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::DigitalTelevision => 0x01,
            Self::DigitalRadioSound => 0x02,
            Self::Teletext => 0x03,
            Self::NvodReference => 0x04,
            Self::NvodTimeShifted => 0x05,
            Self::Mosaic => 0x06,
            Self::FmRadio => 0x07,
            Self::DvbSrm => 0x08,
            Self::AdvancedCodecDigitalRadio => 0x0A,
            Self::AvcMosaic => 0x0B,
            Self::DataBroadcast => 0x0C,
            Self::RcsMap => 0x0E,
            Self::RcsFls => 0x0F,
            Self::Mhp => 0x10,
            Self::HdDigitalTelevision => 0x11,
            Self::AvcSdDigitalTelevision => 0x16,
            Self::AvcSdNvodTimeShifted => 0x17,
            Self::AvcSdNvodReference => 0x18,
            Self::AvcHdDigitalTelevision => 0x19,
            Self::AvcHdNvodTimeShifted => 0x1A,
            Self::AvcHdNvodReference => 0x1B,
            Self::AvcFrameCompatiblePlanoStereoscopicHd => 0x1C,
            Self::AvcFrameCompatiblePlanoStereoscopicHdNvodTimeShifted => 0x1D,
            Self::AvcFrameCompatiblePlanoStereoscopicHdNvodReference => 0x1E,
            Self::HevcDigitalTelevision => 0x1F,
            Self::HevcUhdDigitalTelevision => 0x20,
            Self::VvcDigitalTelevision => 0x21,
            Self::Avs3DigitalTelevision => 0x22,
            Self::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Returns a human-readable spec name for this value.
    pub fn name(self) -> &'static str {
        match self {
            Self::DigitalTelevision => "digital television service",
            Self::DigitalRadioSound => "digital radio sound service",
            Self::Teletext => "teletext service",
            Self::NvodReference => "NVOD reference service",
            Self::NvodTimeShifted => "NVOD time-shifted service",
            Self::Mosaic => "mosaic service",
            Self::FmRadio => "FM radio service",
            Self::DvbSrm => "DVB SRM service",
            Self::AdvancedCodecDigitalRadio => "advanced codec digital radio sound service",
            Self::AvcMosaic => "H.264/AVC mosaic service",
            Self::DataBroadcast => "data broadcast service",
            Self::RcsMap => "RCS Map",
            Self::RcsFls => "RCS FLS",
            Self::Mhp => "DVB MHP service",
            Self::HdDigitalTelevision => "HD digital television service",
            Self::AvcSdDigitalTelevision => "H.264/AVC SD digital television service",
            Self::AvcSdNvodTimeShifted => "H.264/AVC SD NVOD time-shifted service",
            Self::AvcSdNvodReference => "H.264/AVC SD NVOD reference service",
            Self::AvcHdDigitalTelevision => "H.264/AVC HD digital television service",
            Self::AvcHdNvodTimeShifted => "H.264/AVC HD NVOD time-shifted service",
            Self::AvcHdNvodReference => "H.264/AVC HD NVOD reference service",
            Self::AvcFrameCompatiblePlanoStereoscopicHd => {
                "H.264/AVC frame compatible plano-stereoscopic HD digital television service"
            }
            Self::AvcFrameCompatiblePlanoStereoscopicHdNvodTimeShifted => {
                "H.264/AVC frame compatible plano-stereoscopic HD NVOD time-shifted service"
            }
            Self::AvcFrameCompatiblePlanoStereoscopicHdNvodReference => {
                "H.264/AVC frame compatible plano-stereoscopic HD NVOD reference service"
            }
            Self::HevcDigitalTelevision => "HEVC digital television service",
            Self::HevcUhdDigitalTelevision => "HEVC UHD digital television service",
            Self::VvcDigitalTelevision => "VVC digital television service",
            Self::Avs3DigitalTelevision => "AVS3 digital television service",
            Self::Reserved(_) => "reserved",
        }
    }
}

/// Service Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct ServiceDescriptor<'a> {
    /// service_type byte (ETSI Table 89).
    pub service_type: ServiceType,
    /// DVB Annex-A encoded provider name.
    pub provider_name: DvbText<'a>,
    /// DVB Annex-A encoded service name.
    pub service_name: DvbText<'a>,
}

impl<'a> Parse<'a> for ServiceDescriptor<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ServiceDescriptor",
            "unexpected tag for service_descriptor",
        )?;
        if body.len() < 3 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "service_descriptor body too short for service_type + two length fields",
            });
        }
        let service_type = ServiceType::from_u8(body[0]);
        let provider_len = body[1] as usize;
        let provider_end = 2 + provider_len;
        if provider_end + 1 > body.len() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "service_provider_name_length runs past descriptor end",
            });
        }
        let provider_name = DvbText::new(&body[2..provider_end]);
        let service_len = body[provider_end] as usize;
        let service_end = provider_end + 1 + service_len;
        if service_end > body.len() {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "service_name_length runs past descriptor end",
            });
        }
        let service_name = DvbText::new(&body[provider_end + 1..service_end]);
        Ok(Self {
            service_type,
            provider_name,
            service_name,
        })
    }
}

impl Serialize for ServiceDescriptor<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + 1 + 1 + self.provider_name.len() + 1 + self.service_name.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = (len - HEADER_LEN) as u8;
        buf[2] = self.service_type.to_u8();
        buf[3] = self.provider_name.len() as u8;
        let p_start = 4;
        let p_end = p_start + self.provider_name.len();
        buf[p_start..p_end].copy_from_slice(self.provider_name.raw());
        buf[p_end] = self.service_name.len() as u8;
        let s_start = p_end + 1;
        buf[s_start..s_start + self.service_name.len()].copy_from_slice(self.service_name.raw());
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for ServiceDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SERVICE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_all_fields() {
        // service_type=1 (DigitalTelevision), provider="EUTE", service="TF1"
        let bytes = [
            TAG, 10, 0x01, 4, b'E', b'U', b'T', b'E', 3, b'T', b'F', b'1',
        ];
        let d = ServiceDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.service_type, ServiceType::DigitalTelevision);
        assert_eq!(d.provider_name.raw(), b"EUTE");
        assert_eq!(d.service_name.raw(), b"TF1");
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = ServiceDescriptor::parse(&[0x49, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x49, .. }));
    }

    #[test]
    fn parse_rejects_short_header() {
        let err = ServiceDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_truncated_body() {
        let err = ServiceDescriptor::parse(&[TAG, 5, 0x01, 0xFF]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_provider_length_overrun() {
        // provider_len says 100 but descriptor body only 5 bytes.
        let bytes = [TAG, 5, 0x01, 100, b'A', b'B', b'C'];
        let err = ServiceDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn empty_provider_and_service_names_valid() {
        let bytes = [TAG, 3, 0x01, 0, 0];
        let d = ServiceDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.service_type, ServiceType::DigitalTelevision);
        assert!(d.provider_name.raw().is_empty());
        assert!(d.service_name.raw().is_empty());
    }

    #[test]
    fn serialize_round_trip() {
        let d = ServiceDescriptor {
            service_type: ServiceType::AvcHdDigitalTelevision,
            provider_name: DvbText::new(b"BBC"),
            service_name: DvbText::new(b"BBC ONE HD"),
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = ServiceDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn descriptor_length_matches_payload() {
        let d = ServiceDescriptor {
            service_type: ServiceType::DigitalTelevision,
            provider_name: DvbText::new(b"AA"),
            service_name: DvbText::new(b"BBB"),
        };
        // 1 (type) + 1 (p_len) + 2 (p) + 1 (s_len) + 3 (s) = 8
        assert_eq!(d.serialized_len() - 2, 8);
    }

    #[test]
    fn service_type_full_range_round_trip() {
        for b in 0..=0xFF_u8 {
            let st = ServiceType::from_u8(b);
            assert_eq!(st.to_u8(), b, "round-trip failed for byte 0x{b:02X}");
        }
    }

    #[test]
    fn service_type_name_for_known() {
        assert_eq!(
            ServiceType::DigitalTelevision.name(),
            "digital television service"
        );
        assert_eq!(
            ServiceType::HevcDigitalTelevision.name(),
            "HEVC digital television service"
        );
        assert_eq!(
            ServiceType::HevcUhdDigitalTelevision.name(),
            "HEVC UHD digital television service"
        );
        assert_eq!(ServiceType::HevcUhdDigitalTelevision.to_u8(), 0x20);
        assert_eq!(
            ServiceType::from_u8(0x20),
            ServiceType::HevcUhdDigitalTelevision
        );
        assert_eq!(ServiceType::Reserved(0x55).name(), "reserved");
    }
}
