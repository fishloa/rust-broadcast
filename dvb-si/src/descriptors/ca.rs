//! Conditional Access (CA) Descriptor — ISO/IEC 13818-1 §2.6.16 (tag 0x09).
//!
//! Identifies a conditional access system and the PID carrying ECM/EMM data
//! for that system. Optional private data may follow the standard fields.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};
use mpeg_ts::pid::Pid;

/// Descriptor tag for CA_descriptor.
pub const TAG: u8 = 0x09;
const HEADER_LEN: usize = 2;
const MIN_BODY_LEN: usize = 4; // ca_system_id (2) + ca_pid (2)

/// Best-effort, non-exhaustive mapping from CA system ID to a human-readable
/// name.  Generated at build time from vendored TSDuck `.names` data
/// (`registries/tsCAS.names`); attribution in `registries/README.md`.
#[must_use]
pub fn ca_system_name(ca_system_id: u16) -> Option<&'static str> {
    crate::registry_names::ca_system_name_generated(ca_system_id)
}

/// Default of the 3-bit `reserved` ahead of `CA_PID`: H.222.0 clause 2.1
/// "reserved" = `1` (all 9 editions 07/1995-08/2023, Table 2-46). Never given
/// a meaning; 2/715 real descriptors (test-128) write `000`, so parse keeps it.
pub const RESERVED_DEFAULT: u8 = 0b111;

/// Conditional Access Descriptor.
///
/// Carried in the program-level or ES-level descriptor loops of a PMT, or in
/// the CAT. Identifies the CA system and the PID where Entitlement Control
/// Messages (ECMs) or Entitlement Management Messages (EMMs) can be found.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct CaDescriptor<'a> {
    /// `reserved(3)` ahead of `CA_PID` (3 bits, right-aligned). Captured on
    /// parse, re-emitted verbatim; [`CaDescriptor::new`] defaults to
    /// [`RESERVED_DEFAULT`] (`0b111`).
    pub reserved: u8,

    /// Conditional Access System ID.
    ///
    /// See [`ca_system_name`] for a best-effort human-readable name built
    /// from the TSDuck registry (`registries/tsCAS.names`).
    pub ca_system_id: u16,

    /// PID carrying ECM/EMM data for this CA system.
    /// Bits `[12:0]` of the 2-byte field; upper 3 bits are reserved.
    pub ca_pid: Pid,

    /// Optional private data following the standard CA descriptor fields.
    pub private_data: &'a [u8],
}

impl<'a> CaDescriptor<'a> {
    /// Build a descriptor with the spec-default reserved bits.
    #[must_use]
    pub const fn new(ca_system_id: u16, ca_pid: Pid, private_data: &'a [u8]) -> Self {
        Self {
            reserved: RESERVED_DEFAULT,
            ca_system_id,
            ca_pid,
            private_data,
        }
    }
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
        let (fixed, private_data) =
            body.split_first_chunk::<MIN_BODY_LEN>()
                .ok_or(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "CA_descriptor length too short for mandatory fields",
                })?;
        let ca_system_id = u16::from_be_bytes([fixed[0], fixed[1]]);
        // ca_pid: upper 3 bits are reserved (should be 0b111), lower 13 bits are the PID
        let ca_pid = Pid::new(u16::from_be_bytes([fixed[2], fixed[3]]));
        Ok(Self {
            reserved: fixed[2] >> 5,
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
        crate::descriptors::write_descriptor_header(buf, TAG, len - HEADER_LEN)?;
        buf[2] = (self.ca_system_id >> 8) as u8;
        buf[3] = (self.ca_system_id & 0xFF) as u8;
        // ca_pid is a 13-bit field (upper 3 bits reserved); `Pid` is 13-bit by
        // construction, so no out-of-range value can reach the wire.
        let ca_pid = self.ca_pid.value();
        buf[4] = ((self.reserved & 0x07) << 5) | ((ca_pid >> 8) as u8 & 0x1F);
        buf[5] = (ca_pid & 0xFF) as u8;
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
        assert_eq!(d.ca_pid, Pid::new(0x0101));
        assert!(d.private_data.is_empty());
    }

    #[test]
    fn parse_with_private_data() {
        // tag=0x09, len=6, CAID=0x0500, PID=0x0101, private=[0xAA, 0xBB]
        let bytes = [TAG, 6, 0x05, 0x00, 0xE1, 0x01, 0xAA, 0xBB];
        let d = CaDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.ca_system_id, 0x0500);
        assert_eq!(d.ca_pid, Pid::new(0x0101));
        assert_eq!(d.private_data, &[0xAA, 0xBB]);
    }

    /// Regression for r03-W1 (the #972 class): a `ca_pid` over 13 bits must
    /// be rejected, not silently wrapped into a different, wrong PID. The
    /// field is now a `Pid`, so the bad value is unrepresentable: the only
    /// way to build one from a caller-supplied `u16` is `Pid::try_new`, which
    /// rejects it before a descriptor can exist.
    #[test]
    fn ca_pid_over_13_bits_is_unrepresentable() {
        let err = Pid::try_new(0x2101).unwrap_err();
        assert_eq!(err.value, 0x2101);
        assert!(Pid::try_new(0x1FFF).is_ok());
        assert!(Pid::try_new(0x2000).is_err());
        // The wire parse masks to the 13 PID bits and keeps the 3 reserved
        // bits separately, so a parsed descriptor re-serializes identically.
        let bytes = [TAG, 4, 0x05, 0x00, 0xE1, 0x01];
        let d = CaDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.ca_pid, Pid::new(0x0101));
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
        let d = CaDescriptor::new(0x1800, Pid::new(0x0200), &[0xDE, 0xAD]);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = CaDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_round_trip_no_private_data() {
        let d = CaDescriptor::new(0x0500, Pid::new(0x0101), &[]);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = CaDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = CaDescriptor::new(0x0500, Pid::new(0x0101), &[]);
        let mut tiny = vec![0u8; 3];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn descriptor_length_matches_payload() {
        let d = CaDescriptor::new(0x0500, Pid::new(0x0101), &[0xAA]);
        assert_eq!(d.serialized_len() - 2, 5);
    }

    #[test]
    fn ca_system_name_exact_entry() {
        // Exact entry from tsCAS.names [CASystemId].
        assert_eq!(
            ca_system_name(0x0001),
            Some("IPDC SPP Open Security Framework Generic Roaming")
        );
        assert_eq!(ca_system_name(0x004E), None);
    }

    #[test]
    fn ca_system_name_range_entry() {
        // Range entry 0x0100..=0x01FF => "MediaGuard" from tsCAS.names.
        assert_eq!(ca_system_name(0x0100), Some("MediaGuard"));
        assert_eq!(ca_system_name(0x01FF), Some("MediaGuard"));
    }

    #[test]
    fn ca_system_name_unknown() {
        assert_eq!(ca_system_name(0x0000), None);
        assert_eq!(ca_system_name(0x0003), None);
        assert_eq!(ca_system_name(0xDEAD), None);
    }

    /// Every reserved(3) value a real sender writes re-serializes identically
    /// (test-128 writes `000`, the spec value is `111`).
    #[test]
    fn reserved_preserved_for_every_value() {
        for r in 0u8..=7 {
            let bytes = [TAG, 6, 0x05, 0x00, (r << 5) | 0x01, 0x01, 0xAA, 0xBB];
            let d = CaDescriptor::parse(&bytes).unwrap();
            assert_eq!(d.reserved, r);
            let mut buf = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut buf).unwrap();
            assert_eq!(buf, bytes);
        }
    }

    #[test]
    fn default_reserved_is_ones() {
        let d = CaDescriptor::new(0x0500, Pid::new(0x0101), &[]);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [TAG, 4, 0x05, 0x00, 0xE1, 0x01]);
    }
}
