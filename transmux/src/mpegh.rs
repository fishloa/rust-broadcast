//! MPEG-H 3D Audio in ISOBMFF — `mhaC` (ISO/IEC 23008-3 §20).
//!
//! Implements the `MHAConfigurationBox` (`mhaC`) and the typed
//! `MHADecoderConfigurationRecord` that it carries. The `mpegh3daConfig` blob
//! is **opaque** to this crate.
//!
//! The MHAS packet framing used to locate that blob in an MPEG-2 TS
//! elementary stream lives in [`crate::mhas`].

use crate::error::{Error, Result};
use crate::wire_cursor::be_u16;
use alloc::vec::Vec;
use broadcast_common::error::InvalidValue;
use broadcast_common::len::{need_in, need_out};
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

/// Byte offset of `referenceChannelLayout` in the record.
const MHAC_REFERENCE_CHANNEL_LAYOUT_OFFSET: usize = 2;
/// Byte offset of `mpegh3daConfigLength` in the record.
const MHAC_CONFIG_LENGTH_OFFSET: usize = 3;
/// Byte width of `mpegh3daConfigLength`.
const MHAC_CONFIG_LENGTH_LEN: usize = core::mem::size_of::<u16>();

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
        need_in(
            bytes,
            MHAC_RECORD_FIXED_LEN,
            "MHADecoderConfigurationRecord",
        )?;
        let configuration_version = bytes[0];
        if configuration_version != MHAC_CONFIGURATION_VERSION {
            return Err(Error::InvalidValue(InvalidValue {
                field: "configurationVersion",
                value: configuration_version as u64,
                reason: "must be 1 (ISO/IEC 23008-3 §20)",
            }));
        }
        let mpegh3da_profile_level_indication = bytes[1];
        let reference_channel_layout = bytes[MHAC_REFERENCE_CHANNEL_LAYOUT_OFFSET];
        let config_len = usize::from(be_u16(
            bytes,
            MHAC_CONFIG_LENGTH_OFFSET,
            "MHADecoderConfigurationRecord",
        )?);
        let need = MHAC_RECORD_FIXED_LEN + config_len;
        need_in(bytes, need, "MHADecoderConfigurationRecord.mpegh3daConfig")?;
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
        need_out(buf, need)?;
        buf[0] = self.configuration_version;
        buf[1] = self.mpegh3da_profile_level_indication;
        buf[MHAC_REFERENCE_CHANNEL_LAYOUT_OFFSET] = self.reference_channel_layout;
        let config_len =
            broadcast_common::len::fit_u16(self.mpegh3da_config.len(), "mpegh3daConfigLength")?;
        buf[MHAC_CONFIG_LENGTH_OFFSET..MHAC_CONFIG_LENGTH_OFFSET + MHAC_CONFIG_LENGTH_LEN]
            .copy_from_slice(&config_len.to_be_bytes());
        buf[MHAC_RECORD_FIXED_LEN..need].copy_from_slice(&self.mpegh3da_config);
        Ok(need)
    }
}

#[cfg(test)]
mod mhac_record_tests {
    use super::*;

    fn record(config_len: usize) -> MHADecoderConfigurationRecord {
        MHADecoderConfigurationRecord {
            configuration_version: 1,
            mpegh3da_profile_level_indication: 0x0B,
            reference_channel_layout: 2,
            mpegh3da_config: alloc::vec![0x5Au8; config_len],
        }
    }

    /// A 65 536-byte `mpegh3daConfig` cannot fit the 16-bit
    /// `mpegh3daConfigLength` field (#1129): unfixed, `(len as u16)` wraps
    /// 65536 down to 0 while the full-length payload is still copied,
    /// misframing the record with `Ok`.
    #[test]
    fn oversized_config_length_errors() {
        let rec = record(65536);
        let err = rec.try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                    field: "mpegh3daConfigLength",
                    ..
                })
            ),
            "expected FieldOverflow for mpegh3daConfigLength, got {err:?}"
        );
    }

    /// The boundary: exactly 65 535 bytes (u16::MAX) still round-trips.
    #[test]
    fn max_config_length_round_trips() {
        let rec = record(65535);
        let bytes = rec.try_to_bytes().unwrap();
        let re = MHADecoderConfigurationRecord::parse(&bytes).unwrap();
        assert_eq!(re.mpegh3da_config.len(), 65535);
    }
}
