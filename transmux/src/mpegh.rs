//! MPEG-H 3D Audio in ISOBMFF — `mha1`/`mhm1` + `mhaC` (ISO/IEC 23008-3 §20).
//!
//! Implements the `MHAConfigurationBox` (`mhaC`) and the typed
//! `MHADecoderConfigurationRecord` that it carries.  The `mpegh3daConfig` blob
//! is **opaque** to this crate — the caller supplies it already encoded
//! (identical treatment to the `esds` AudioSpecificConfig and `dac4` body).
//!
//! Sources:
//! - **ISO/IEC 23008-3 §20** — canonical box/record syntax (paid).
//! - **ATSC A/342-3 §5.2.2** — profile-level constraints for broadcast
//!   (`transmux/docs/codec/mpegh-atsc-a342-3.md`).
//! - Record layout: `transmux/docs/codec/mpegh-mhaC.md`.
//!
//! ## Record layout
//!
//! ```text
//! aligned(8) class MHADecoderConfigurationRecord {
//!     unsigned int(8)  configurationVersion;           // must be 1
//!     unsigned int(8)  mpegh3daProfileLevelIndication; // CICP profile-level
//!     unsigned int(8)  referenceChannelLayout;         // CICP ChannelConfig
//!     unsigned int(16) mpegh3daConfigLength;
//!     unsigned int(8)  mpegh3daConfig[mpegh3daConfigLength];
//! }
//! ```
//!
//! ## `mhaC` box
//!
//! `MHAConfigurationBox` extends `Box('mhaC')` with exactly one
//! `MHADecoderConfigurationRecord` as its body.

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

// ---------------------------------------------------------------------------
// Four-CC constants
// ---------------------------------------------------------------------------

/// FourCC of the MPEG-H configuration box (`mhaC`).
pub const MHAC_FOURCC: [u8; 4] = *b"mhaC";
/// FourCC of the MPEG-H 3D Audio sample entry — raw (unpackaged) MHAS frames.
pub const MHA1_FOURCC: [u8; 4] = *b"mha1";
/// FourCC of the MPEG-H 3D Audio sample entry (variant 2).
pub const MHA2_FOURCC: [u8; 4] = *b"mha2";
/// FourCC of the MPEG-H 3D Audio sample entry — in-band MHAS (config in stream).
pub const MHM1_FOURCC: [u8; 4] = *b"mhm1";
/// FourCC of the MPEG-H 3D Audio sample entry (variant 2, in-band).
pub const MHM2_FOURCC: [u8; 4] = *b"mhm2";

// ---------------------------------------------------------------------------
// Fixed sizes
// ---------------------------------------------------------------------------

/// Minimum byte size of the `MHADecoderConfigurationRecord` (header fields only,
/// no `mpegh3daConfig` bytes).
///
/// Byte layout: `configurationVersion` `[7:0]` (1) + `mpegh3daProfileLevelIndication`
/// `[7:0]` (1) + `referenceChannelLayout` `[7:0]` (1) + `mpegh3daConfigLength` `[15:0]` (2)
/// = 5 bytes.
pub const MHAC_RECORD_FIXED_LEN: usize = 5;

/// The only valid value for `configurationVersion` (ISO/IEC 23008-3 §20).
pub const MHAC_CONFIGURATION_VERSION: u8 = 1;

// ---------------------------------------------------------------------------
// MHADecoderConfigurationRecord
// ---------------------------------------------------------------------------

/// `MHADecoderConfigurationRecord` — ISO/IEC 23008-3 §20.
///
/// The fixed-header part of an `mhaC` box.  The `mpegh3daConfig` blob is kept
/// opaque; transmux copies it through unchanged.
///
/// ## Wire layout
///
/// | Offset | Size | Field |
/// |--------|------|-------|
/// | 0      | 1    | `configurationVersion` (= 1) |
/// | 1      | 1    | `mpegh3daProfileLevelIndication` `[7:0]` |
/// | 2      | 1    | `referenceChannelLayout` `[7:0]` |
/// | 3      | 2    | `mpegh3daConfigLength` `[15:0]` (big-endian) |
/// | 5      | N    | `mpegh3daConfig[0..mpegh3daConfigLength]` |
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MHADecoderConfigurationRecord {
    /// Must be `1` per ISO/IEC 23008-3 §20.
    pub configuration_version: u8,
    /// CICP profile-level indicator `[7:0]`.  ATSC A/342-3 §5.2.2.1 mandates
    /// `0x0B`/`0x0C`/`0x0D` (LC L1/L2/L3) and `0x10`/`0x11`/`0x12` (BL L1/L2/L3).
    pub mpegh3da_profile_level_indication: u8,
    /// CICP `ChannelConfiguration` `[7:0]` (reference channel layout).
    pub reference_channel_layout: u8,
    /// The opaque `mpegh3daConfig()` bytes (ISO/IEC 23008-3 §20).
    pub mpegh3da_config: Vec<u8>,
}

impl MHADecoderConfigurationRecord {
    /// Construct a record from its constituent fields.
    pub fn new(
        mpegh3da_profile_level_indication: u8,
        reference_channel_layout: u8,
        mpegh3da_config: Vec<u8>,
    ) -> Self {
        Self {
            configuration_version: MHAC_CONFIGURATION_VERSION,
            mpegh3da_profile_level_indication,
            reference_channel_layout,
            mpegh3da_config,
        }
    }

    /// RFC 6381 codec string for this profile-level — `mhm1.0xNN`
    /// (ATSC A/342-3 §5.2.2; `mhm1` is the in-band MHAS sample-entry code).
    pub fn rfc6381(&self) -> alloc::string::String {
        alloc::format!("mhm1.0x{:02X}", self.mpegh3da_profile_level_indication)
    }
}

impl<'a> Parse<'a> for MHADecoderConfigurationRecord {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < MHAC_RECORD_FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: MHAC_RECORD_FIXED_LEN,
                have: bytes.len(),
                what: "MHADecoderConfigurationRecord",
            });
        }
        let configuration_version = bytes[0];
        if configuration_version != MHAC_CONFIGURATION_VERSION {
            return Err(Error::InvalidValue {
                field: "configurationVersion",
                value: configuration_version as u64,
                reason: "must be 1 (ISO/IEC 23008-3 §20)",
            });
        }
        let mpegh3da_profile_level_indication = bytes[1];
        let reference_channel_layout = bytes[2];
        let config_len = u16::from_be_bytes([bytes[3], bytes[4]]) as usize;
        let need = MHAC_RECORD_FIXED_LEN + config_len;
        if bytes.len() < need {
            return Err(Error::BufferTooShort {
                need,
                have: bytes.len(),
                what: "MHADecoderConfigurationRecord.mpegh3daConfig",
            });
        }
        let mpegh3da_config = bytes[MHAC_RECORD_FIXED_LEN..need].to_vec();
        Ok(Self {
            configuration_version,
            mpegh3da_profile_level_indication,
            reference_channel_layout,
            mpegh3da_config,
        })
    }
}

impl Serialize for MHADecoderConfigurationRecord {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        MHAC_RECORD_FIXED_LEN + self.mpegh3da_config.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0] = self.configuration_version;
        buf[1] = self.mpegh3da_profile_level_indication;
        buf[2] = self.reference_channel_layout;
        let config_len = self.mpegh3da_config.len() as u16;
        buf[3..5].copy_from_slice(&config_len.to_be_bytes());
        buf[5..need].copy_from_slice(&self.mpegh3da_config);
        Ok(need)
    }
}

// ---------------------------------------------------------------------------
// MhaC box (MHAConfigurationBox body — record only, without the box header)
// ---------------------------------------------------------------------------

/// The body of an `mhaC` box (`MHAConfigurationBox`) — ISO/IEC 23008-3 §20.
///
/// Serialises/parses only the record bytes; the 8-byte box header (`size` +
/// `'mhaC'`) is added by the caller via [`crate::init_segment::OpaqueBox`].
pub type MhaCBox = MHADecoderConfigurationRecord;
