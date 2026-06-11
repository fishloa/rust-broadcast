//! Conditional Access (CA) Descriptor — ISO/IEC 13818-1 §2.6.16 (tag 0x09).
//!
//! Identifies a conditional access system and the PID carrying ECM/EMM data
//! for that system. Optional private data may follow the standard fields.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for CA_descriptor.
pub const TAG: u8 = 0x09;
const HEADER_LEN: usize = 2;
const MIN_BODY_LEN: usize = 4; // ca_system_id (2) + ca_pid (2)

/// Best-effort, non-exhaustive mapping from CA system ID to a human-readable
/// name.  Verified ranges from the DVB Services registry.
#[must_use]
pub fn ca_system_name(ca_system_id: u16) -> Option<&'static str> {
    match ca_system_id {
        0x0100..=0x01FF => Some("Seca / Mediaguard"),
        0x0500..=0x05FF => Some("Viaccess"),
        0x0600..=0x06FF => Some("Irdeto"),
        0x0700..=0x07FF => Some("DigiCipher 2"),
        0x0900..=0x09FF => Some("NDS / Videoguard"),
        0x0B00..=0x0BFF => Some("Conax"),
        0x0D00..=0x0DFF => Some("Cryptoworks"),
        0x0E00..=0x0EFF => Some("PowerVu"),
        0x1700..=0x17FF => Some("Verimatrix (BetaCrypt)"),
        0x1800..=0x18FF => Some("Nagravision"),
        0x2600..=0x26FF => Some("BISS"),
        0x4AD4..=0x4AD5 => Some("Widevine"),
        0x4AE0..=0x4AE1 => Some("DRE-Crypt"),
        0x5601..=0x5604 => Some("Verimatrix VCAS"),
        _ => None,
    }
}

/// Conditional Access Descriptor.
///
/// Carried in the program-level or ES-level descriptor loops of a PMT, or in
/// the CAT. Identifies the CA system and the PID where Entitlement Control
/// Messages (ECMs) or Entitlement Management Messages (EMMs) can be found.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct CaDescriptor<'a> {
    /// Conditional Access System ID.
    ///
    /// Well-known CAID ranges (DVB Services registry):
    ///   0x0100–0x01FF  Seca / Mediaguard
    ///   0x0500–0x05FF  Viaccess
    ///   0x0600–0x06FF  Irdeto
    ///   0x0700–0x07FF  DigiCipher 2
    ///   0x0900–0x09FF  NDS / Videoguard
    ///   0x0B00–0x0BFF  Conax
    ///   0x0D00–0x0DFF  Cryptoworks
    ///   0x0E00–0x0EFF  PowerVu
    ///   0x1700–0x17FF  Verimatrix (BetaCrypt)
    ///   0x1800–0x18FF  Nagravision
    ///   0x2600–0x26FF  BISS
    ///   0x4AD4–0x4AD5  Widevine
    ///   0x4AE0–0x4AE1  DRE-Crypt
    ///   0x5601–0x5604  Verimatrix VCAS
    pub ca_system_id: u16,

    /// PID carrying ECM/EMM data for this CA system.
    /// Bits `[12:0]` of the 2-byte field; upper 3 bits are reserved.
    pub ca_pid: u16,

    /// Optional private data following the standard CA descriptor fields.
    pub private_data: &'a [u8],
}

impl<'a> Parse<'a> for CaDescriptor<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "CaDescriptor",
            "unexpected tag for CA_descriptor",
        )?;
        if body.len() < MIN_BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "CA_descriptor length too short for mandatory fields",
            });
        }
        let ca_system_id = u16::from_be_bytes([body[0], body[1]]);
        // ca_pid: upper 3 bits are reserved (should be 0b111), lower 13 bits are the PID
        let ca_pid = ((u16::from(body[2]) & 0x1F) << 8) | u16::from(body[3]);
        let private_data = if body.len() > MIN_BODY_LEN {
            &body[MIN_BODY_LEN..]
        } else {
            &[]
        };
        Ok(Self {
            ca_system_id,
            ca_pid,
            private_data,
        })
    }
}

impl Serialize for CaDescriptor<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + MIN_BODY_LEN + self.private_data.len()
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
        buf[2] = (self.ca_system_id >> 8) as u8;
        buf[3] = (self.ca_system_id & 0xFF) as u8;
        // ca_pid with reserved upper 3 bits set to 1
        buf[4] = 0xE0 | ((self.ca_pid >> 8) as u8);
        buf[5] = (self.ca_pid & 0xFF) as u8;
        if !self.private_data.is_empty() {
            buf[HEADER_LEN + MIN_BODY_LEN..len].copy_from_slice(self.private_data);
        }
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for CaDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "CA";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_viaccess_ecm_pid() {
        // tag=0x09, len=4, CAID=0x0500 (Viaccess), PID=0x0101
        let bytes = [TAG, 4, 0x05, 0x00, 0xE1, 0x01];
        let d = CaDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.ca_system_id, 0x0500);
        assert_eq!(d.ca_pid, 0x0101);
        assert!(d.private_data.is_empty());
    }

    #[test]
    fn parse_with_private_data() {
        // tag=0x09, len=6, CAID=0x0500, PID=0x0101, private=[0xAA, 0xBB]
        let bytes = [TAG, 6, 0x05, 0x00, 0xE1, 0x01, 0xAA, 0xBB];
        let d = CaDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.ca_system_id, 0x0500);
        assert_eq!(d.ca_pid, 0x0101);
        assert_eq!(d.private_data, &[0xAA, 0xBB]);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = CaDescriptor::parse(&[0x0A, 4, 0x05, 0x00, 0xE1, 0x01]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x0A, .. }));
    }

    #[test]
    fn parse_rejects_short_header() {
        let err = CaDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_length_too_short() {
        let bytes = [TAG, 3, 0x05, 0x00, 0xE1];
        let err = CaDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn parse_rejects_length_overflow() {
        let bytes = [TAG, 10, 0x05, 0x00, 0xE1, 0x01];
        let err = CaDescriptor::parse(&bytes).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = CaDescriptor {
            ca_system_id: 0x1800,
            ca_pid: 0x0200,
            private_data: &[0xDE, 0xAD],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = CaDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_round_trip_no_private_data() {
        let d = CaDescriptor {
            ca_system_id: 0x0500,
            ca_pid: 0x0101,
            private_data: &[],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = CaDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = CaDescriptor {
            ca_system_id: 0x0500,
            ca_pid: 0x0101,
            private_data: &[],
        };
        let mut tiny = vec![0u8; 3];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn descriptor_length_matches_payload() {
        let d = CaDescriptor {
            ca_system_id: 0x0500,
            ca_pid: 0x0101,
            private_data: &[0xAA],
        };
        assert_eq!(d.serialized_len() - 2, 5);
    }

    #[test]
    fn ca_system_name_verified_ranges() {
        assert_eq!(ca_system_name(0x0100), Some("Seca / Mediaguard"));
        assert_eq!(ca_system_name(0x01FF), Some("Seca / Mediaguard"));
        assert_eq!(ca_system_name(0x0500), Some("Viaccess"));
        assert_eq!(ca_system_name(0x05FF), Some("Viaccess"));
        assert_eq!(ca_system_name(0x0600), Some("Irdeto"));
        assert_eq!(ca_system_name(0x0700), Some("DigiCipher 2"));
        assert_eq!(ca_system_name(0x0900), Some("NDS / Videoguard"));
        assert_eq!(ca_system_name(0x0B00), Some("Conax"));
        assert_eq!(ca_system_name(0x0D00), Some("Cryptoworks"));
        assert_eq!(ca_system_name(0x0E00), Some("PowerVu"));
        assert_eq!(ca_system_name(0x1700), Some("Verimatrix (BetaCrypt)"));
        assert_eq!(ca_system_name(0x1800), Some("Nagravision"));
        assert_eq!(ca_system_name(0x2600), Some("BISS"));
        assert_eq!(ca_system_name(0x4AD4), Some("Widevine"));
        assert_eq!(ca_system_name(0x4AE0), Some("DRE-Crypt"));
        assert_eq!(ca_system_name(0x5601), Some("Verimatrix VCAS"));
    }

    #[test]
    fn ca_system_name_removed_entries_return_none() {
        assert_eq!(ca_system_name(0x0000), None);
        assert_eq!(ca_system_name(0x5605), None);
        assert_eq!(ca_system_name(0x4AE2), None);
        assert_eq!(ca_system_name(0x0F00), None);
    }

    #[test]
    fn ca_system_name_unknown() {
        assert_eq!(ca_system_name(0xdead), None);
    }
}
