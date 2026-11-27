//! Codec-metadata extension methods over the ISOBMFF NAL-unit newtypes
//! (`isobmff` feature).
//!
//! The newtypes (`AvcSps`, `HevcNalUnit`) are pure box syntax; decoding their
//! payloads and formatting the RFC 6381 codec string need this crate's SPS
//! bitstream parsers. These traits keep the call sites concise.

use crate::error::Result;
#[cfg(feature = "hevc")]
use crate::nal::{HEVC_NAL_HEADER_LEN, HEVC_NAL_TYPE_MASK};
use crate::rfc6381::rfc6381_avc1;
#[cfg(feature = "hevc")]
use crate::rfc6381::rfc6381_hvc1;
use crate::sps::{AvcSpsInfo, decode_avc_sps};
#[cfg(feature = "hevc")]
use crate::sps::{HevcSpsInfo, decode_hevc_sps};
use alloc::string::String;
use isobmff::nalu_types::AvcSps;
#[cfg(feature = "hevc")]
use isobmff::nalu_types::{HEVC_SPS_NUT, HevcNalUnit};

/// Decode an AVC SPS NAL unit and format its RFC 6381 codec string.
pub trait AvcSpsCodecInfo {
    /// Decode the SPS fields needed for codec configuration.
    fn decode(&self) -> Result<AvcSpsInfo>;

    /// Build the RFC 6381 codec string (e.g. `"avc1.4D400D"`).
    fn rfc6381(&self) -> Result<String>;
}

impl AvcSpsCodecInfo for AvcSps {
    fn decode(&self) -> Result<AvcSpsInfo> {
        decode_avc_sps(&self.0)
    }

    fn rfc6381(&self) -> Result<String> {
        let info = self.decode()?;
        Ok(rfc6381_avc1(
            info.profile_idc(),
            info.constraint_flags(),
            info.level_idc(),
        ))
    }
}

/// Decode an HEVC NAL unit when it is an SPS and format its RFC 6381 string.
#[cfg(feature = "hevc")]
pub trait HevcNalUnitCodecInfo {
    /// Decode the SPS fields if this NAL unit is an SPS (NAL unit type 33).
    fn decode_sps(&self) -> Result<Option<HevcSpsInfo>>;

    /// Build the RFC 6381 `hvc1.…` codec string if this is an SPS NAL unit.
    fn rfc6381(&self) -> Result<Option<String>>;
}

#[cfg(feature = "hevc")]
impl HevcNalUnitCodecInfo for HevcNalUnit {
    fn decode_sps(&self) -> Result<Option<HevcSpsInfo>> {
        if self.0.len() < HEVC_NAL_HEADER_LEN {
            return Ok(None);
        }
        let nal_type = (self.0[0] >> 1) & HEVC_NAL_TYPE_MASK;
        if nal_type != HEVC_SPS_NUT {
            return Ok(None);
        }
        decode_hevc_sps(&self.0).map(Some)
    }

    fn rfc6381(&self) -> Result<Option<String>> {
        Ok(self.decode_sps()?.map(|info| rfc6381_hvc1(&info)))
    }
}

#[cfg(all(test, feature = "hevc"))]
mod tests {
    use super::*;
    use alloc::vec;

    /// A 0- or 1-byte NAL is too short to carry even an HEVC header, so it is
    /// `Ok(None)`; a 2-byte SPS-typed NAL does reach the SPS decoder and its
    /// failure is an error, not `None`.
    #[test]
    fn hevc_nal_unit_decode_sps_distinguishes_short_from_sps_typed() {
        assert!(matches!(HevcNalUnit::new(vec![]).decode_sps(), Ok(None)));
        assert!(matches!(
            HevcNalUnit::new(vec![0x42]).decode_sps(),
            Ok(None)
        ));
        assert!(
            HevcNalUnit::new(vec![0x42, 0x01]).decode_sps().is_err(),
            "a 2-byte SPS-typed NAL reaches the decoder and fails there"
        );
        assert!(matches!(
            HevcNalUnit::new(vec![0x40, 0x01, 0x0C]).decode_sps(),
            Ok(None)
        ));
    }
}
