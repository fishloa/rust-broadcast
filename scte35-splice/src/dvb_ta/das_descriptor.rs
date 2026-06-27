//! DVB_DAS_descriptor() — ETSI TS 103 752-1 V1.2.1 §5.3.5.16, Table 1 + Table 2
//! (tag 0xF0, identifier "DVB_").
//!
//! **NEW binary syntax** (DVB Targeted Advertising Part 1). A DVB-private SCTE 35
//! *splice descriptor*: it rides in the `splice_descriptor()` loop of a base
//! SCTE 35 `splice_info_section()` exactly like any other splice descriptor, and
//! it lets a `splice_insert()` command optionally carry the placement-opportunity
//! typing and UPID that a `segmentation_descriptor()` would otherwise convey
//! ("for full equivalence between `splice_insert()` and `segmentation_descriptor()`
//! methods", §5.3.5.16).
//!
//! It shares the standard SCTE 35 private-descriptor framing handled by
//! [`crate::descriptors::header`] (tag + length + 32-bit identifier), then adds
//! `break_num` (8), `breaks_expected` (8), a reserved nibble + 4-bit
//! [`EquivalentSegmentationType`], and a trailing variable-length `upid`.

use crate::descriptors::header::{self, HEADER_LEN};
use crate::error::{Error, Result};
use crate::traits::SpliceDescriptorDef;
use dvb_common::{Parse, Serialize};

/// `splice_descriptor_tag` for `DVB_DAS_descriptor()` (§5.3.5.16). Shall be
/// `0xF0` (the SCTE 35 private/DVB tag).
pub const TAG: u8 = 0xF0;

/// `identifier` for `DVB_DAS_descriptor()` (§5.3.5.16). Shall be `0x4456425F`
/// (ASCII `"DVB_"`).
pub const DVB_IDENTIFIER: u32 = 0x4456_425F;

/// Bytes of the fixed body between the header and the variable `upid`:
/// `break_num` (1) + `breaks_expected` (1) + reserved/equivalent_segmentation_type (1).
const FIXED_BODY_LEN: usize = 3;

/// `equivalent_segmentation_type` — §5.3.5.16, Table 2 (4 bits).
///
/// Identifies the `segmentation_type` that would be used for the equivalent
/// `segmentation_descriptor` in a `time_signal()` command. Unrecognised /
/// reserved values (0x5–0xF) are carried as [`EquivalentSegmentationType::Reserved`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum EquivalentSegmentationType {
    /// `0x0` — no equivalent.
    NoEquivalent,
    /// `0x1` — Distributor Placement Opportunity (DPO).
    DistributorPlacementOpportunity,
    /// `0x2` — Provider Placement Opportunity (PPO).
    ProviderPlacementOpportunity,
    /// `0x3` — Distributor Advertisement (DA).
    DistributorAdvertisement,
    /// `0x4` — Provider Advertisement (PA).
    ProviderAdvertisement,
    /// Any reserved value (`0x5`–`0xF`), carried verbatim.
    Reserved(u8),
}

impl EquivalentSegmentationType {
    /// Decode the 4-bit `equivalent_segmentation_type` field (only the low
    /// nibble is used).
    #[must_use]
    pub fn from_bits(v: u8) -> Self {
        match v & 0x0F {
            0x0 => Self::NoEquivalent,
            0x1 => Self::DistributorPlacementOpportunity,
            0x2 => Self::ProviderPlacementOpportunity,
            0x3 => Self::DistributorAdvertisement,
            0x4 => Self::ProviderAdvertisement,
            other => Self::Reserved(other),
        }
    }

    /// The 4-bit wire value (low nibble).
    #[must_use]
    pub const fn bits(self) -> u8 {
        match self {
            Self::NoEquivalent => 0x0,
            Self::DistributorPlacementOpportunity => 0x1,
            Self::ProviderPlacementOpportunity => 0x2,
            Self::DistributorAdvertisement => 0x3,
            Self::ProviderAdvertisement => 0x4,
            Self::Reserved(v) => v & 0x0F,
        }
    }

    /// Human-readable spec label (ETSI TS 103 752-1 Table 2).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::NoEquivalent => "no equivalent",
            Self::DistributorPlacementOpportunity => "Distributor Placement Opportunity (DPO)",
            Self::ProviderPlacementOpportunity => "Provider Placement Opportunity (PPO)",
            Self::DistributorAdvertisement => "Distributor Advertisement (DA)",
            Self::ProviderAdvertisement => "Provider Advertisement (PA)",
            Self::Reserved(_) => "reserved",
        }
    }
}
dvb_common::impl_spec_display!(EquivalentSegmentationType, Reserved);

/// `DVB_DAS_descriptor()` — §5.3.5.16, Table 1.
///
/// The `upid` is the trailing variable-length field; it borrows from the parsed
/// buffer (the crate's zero-copy posture). Per §5.3.5.11 it is a URI in the
/// `urn:<reverse-domain>:<identifier>` form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DvbDasDescriptor<'a> {
    /// 32-bit `identifier`; shall be [`DVB_IDENTIFIER`] (`"DVB_"`).
    pub identifier: u32,
    /// `break_num` (8) — position of the break within the programme; `0` if
    /// unused.
    pub break_num: u8,
    /// `breaks_expected` (8) — number of breaks expected within the programme;
    /// `0` if unused.
    pub breaks_expected: u8,
    /// `equivalent_segmentation_type` (4) — Table 2.
    pub equivalent_segmentation_type: EquivalentSegmentationType,
    /// `upid` (`N*8`) — variable-length UPID (URI per §5.3.5.11), borrowed.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub upid: &'a [u8],
}

impl<'a> DvbDasDescriptor<'a> {
    /// Build a `DVB_DAS_descriptor()` with the mandated `"DVB_"` identifier.
    #[must_use]
    pub fn new(
        break_num: u8,
        breaks_expected: u8,
        equivalent_segmentation_type: EquivalentSegmentationType,
        upid: &'a [u8],
    ) -> Self {
        Self {
            identifier: DVB_IDENTIFIER,
            break_num,
            breaks_expected,
            equivalent_segmentation_type,
            upid,
        }
    }
}

impl<'a> Parse<'a> for DvbDasDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let (identifier, body) = header::descriptor_body(bytes, TAG, "DVB_DAS_descriptor")?;
        if body.len() < FIXED_BODY_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN + FIXED_BODY_LEN,
                have: bytes.len(),
                what: "DVB_DAS_descriptor body",
            });
        }
        let break_num = body[0];
        let breaks_expected = body[1];
        // body[2]: reserved (high nibble) + equivalent_segmentation_type (low).
        let equivalent_segmentation_type = EquivalentSegmentationType::from_bits(body[2] & 0x0F);
        let upid = &body[FIXED_BODY_LEN..];
        Ok(Self {
            identifier,
            break_num,
            breaks_expected,
            equivalent_segmentation_type,
            upid,
        })
    }
}

impl Serialize for DvbDasDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + FIXED_BODY_LEN + self.upid.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let body_len = FIXED_BODY_LEN + self.upid.len();
        // `descriptor_length` is a u8 covering `identifier(4) + body_len`.
        // Max allowed body_len = 255 − 4 = 251; a longer upid would make
        // `4 + body_len ≥ 256`, silently truncating the length byte.
        if body_len > 251 {
            return Err(Error::InvalidValue {
                field: "DVB_DAS_descriptor.descriptor_length",
                reason: "upid too long: descriptor_length would overflow u8",
            });
        }
        header::write_header(buf, TAG, self.identifier, body_len);
        buf[HEADER_LEN] = self.break_num;
        buf[HEADER_LEN + 1] = self.breaks_expected;
        // reserved nibble = 0xF (reserved bits set to 1 per crate policy), then
        // the 4-bit equivalent_segmentation_type in the low nibble.
        buf[HEADER_LEN + 2] = 0xF0 | self.equivalent_segmentation_type.bits();
        buf[HEADER_LEN + FIXED_BODY_LEN..need].copy_from_slice(self.upid);
        Ok(need)
    }
}

impl<'a> SpliceDescriptorDef<'a> for DvbDasDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "DVB_DAS";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_hand_computed_wire() {
        let upid = b"urn:com.broadcaster:112210F47DE98115";
        let d = DvbDasDescriptor::new(
            0x01,
            0x03,
            EquivalentSegmentationType::ProviderPlacementOpportunity,
            upid,
        );
        let bytes = d.to_bytes();

        // Hand-computed wire layout:
        //   tag = 0xF0
        //   descriptor_length = 4 (identifier) + 3 (fixed) + upid.len()
        //   identifier = 0x44 0x56 0x42 0x5F  ("DVB_")
        //   break_num = 0x01
        //   breaks_expected = 0x03
        //   reserved|eqseg = 0xF2  (reserved nibble 0xF + PPO 0x2)
        //   upid...
        assert_eq!(bytes[0], 0xF0);
        assert_eq!(bytes[1] as usize, 4 + FIXED_BODY_LEN + upid.len());
        assert_eq!(&bytes[2..6], &[0x44, 0x56, 0x42, 0x5F]);
        assert_eq!(bytes[6], 0x01);
        assert_eq!(bytes[7], 0x03);
        assert_eq!(bytes[8], 0xF2);
        assert_eq!(&bytes[9..], &upid[..]);

        let back = DvbDasDescriptor::parse(&bytes).unwrap();
        assert_eq!(d, back);
        assert_eq!(back.to_bytes(), bytes);
        assert_eq!(back.identifier, DVB_IDENTIFIER);
    }

    #[test]
    fn field_mutation_bites() {
        let upid = b"urn:tv.acme:abc";
        let a = DvbDasDescriptor::new(0, 0, EquivalentSegmentationType::NoEquivalent, upid);
        let b = DvbDasDescriptor::new(1, 0, EquivalentSegmentationType::NoEquivalent, upid);
        assert_ne!(a.to_bytes(), b.to_bytes());
        let c = DvbDasDescriptor::new(
            0,
            0,
            EquivalentSegmentationType::DistributorAdvertisement,
            upid,
        );
        assert_ne!(a.to_bytes(), c.to_bytes());
        // A different UPID changes the wire too.
        let d = DvbDasDescriptor::new(0, 0, EquivalentSegmentationType::NoEquivalent, b"urn:x:y");
        assert_ne!(a.to_bytes(), d.to_bytes());
    }

    #[test]
    fn empty_upid_round_trips() {
        let d = DvbDasDescriptor::new(0, 0, EquivalentSegmentationType::NoEquivalent, &[]);
        let bytes = d.to_bytes();
        assert_eq!(bytes[1] as usize, 4 + FIXED_BODY_LEN);
        let back = DvbDasDescriptor::parse(&bytes).unwrap();
        assert_eq!(d, back);
    }

    #[test]
    fn equivalent_type_all_nibbles_round_trip() {
        for v in 0u8..=0x0F {
            assert_eq!(EquivalentSegmentationType::from_bits(v).bits(), v);
        }
    }

    #[test]
    fn rejects_wrong_tag() {
        let bytes = [0x01, 0x07, 0x44, 0x56, 0x42, 0x5F, 0x00, 0x00, 0xF0];
        assert!(matches!(
            DvbDasDescriptor::parse(&bytes).unwrap_err(),
            Error::UnexpectedDescriptorTag { tag: 0x01, .. }
        ));
    }

    /// A UPID of 252 bytes would make `descriptor_length = 4 + 3 + 252 = 259`,
    /// overflowing a u8. The serializer must reject this.
    ///
    /// Regression test for P1 overflow guard: verify this returns `Err` before
    /// the fix is applied, and `Err` (with the correct variant) after.
    #[test]
    fn serialize_rejects_upid_too_long() {
        let long_upid = vec![0u8; 252];
        let d = DvbDasDescriptor::new(0, 0, EquivalentSegmentationType::NoEquivalent, &long_upid);
        let err = d
            .serialize_into(&mut vec![0u8; d.serialized_len()])
            .unwrap_err();
        assert!(
            matches!(
                err,
                Error::InvalidValue {
                    field: "DVB_DAS_descriptor.descriptor_length",
                    ..
                }
            ),
            "expected InvalidValue for descriptor_length overflow, got {err:?}"
        );
    }

    /// 248-byte upid is the largest valid size: `descriptor_length = 4 + 3 + 248 = 255 = u8::MAX`.
    #[test]
    fn serialize_accepts_max_valid_upid() {
        let max_upid = vec![b'x'; 248];
        let d = DvbDasDescriptor::new(0, 0, EquivalentSegmentationType::NoEquivalent, &max_upid);
        let bytes = d.to_bytes();
        assert_eq!(bytes[1], 255u8); // descriptor_length = 4 (identifier) + 3 (fixed) + 248 (upid)
        let back = DvbDasDescriptor::parse(&bytes).unwrap();
        assert_eq!(back.upid, max_upid.as_slice());
    }
}
