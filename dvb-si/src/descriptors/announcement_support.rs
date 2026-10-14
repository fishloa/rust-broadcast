//! Announcement Support Descriptor — ETSI EN 300 468 §6.2.3 (tag 0x6E, Table 17, PDF p. 56).
//!
//! Carried inside the SDT. Signals which announcement types (emergency,
//! traffic, news, weather, …) a service supports and where each is carried.
//! Body layout (Table 17):
//!
//! ```text
//! announcement_support_indicator 16
//! for (i=0;i<N;i++) {
//!   announcement_type   4
//!   reserved_future_use 1
//!   reference_type      3
//!   if (reference_type == 0x01 || 0x02 || 0x03) {
//!     original_network_id 16
//!     transport_stream_id 16
//!     service_id          16
//!     component_tag        8
//!   }
//! }
//! ```
//!
//! The conditional reference fields (onid/tsid/sid/component_tag) are present
//! only for reference_type 1, 2, 3 (verified against PDF p. 56). They are
//! modelled as an `Option<AnnouncementReference>`.

use super::descriptor_body;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for announcement_support_descriptor.
pub const TAG: u8 = 0x6E;
/// Length of the header (tag byte + length byte).
pub const HEADER_LEN: usize = 2;
/// Length of the announcement_support_indicator field.
pub const INDICATOR_LEN: usize = 2;
/// Length of the announcement_type/reserved/reference_type byte.
pub const TYPE_BYTE_LEN: usize = 1;
/// Length of the conditional reference block: onid(2)+tsid(2)+sid(2)+component_tag(1).
pub const REFERENCE_LEN: usize = 7;

const ANNOUNCEMENT_TYPE_MASK: u8 = 0xF0;
const ANNOUNCEMENT_TYPE_SHIFT: u8 = 4;
const RESERVED_BIT_MASK: u8 = 0x08;
const RESERVED_BIT_SHIFT: u8 = 3;
/// Spec default of the entry's 1-bit `reserved_future_use`: `1` (EN 300 468
/// V1.4.1-V1.20.1 Table 17, global convention; audit
/// `dvb-si/docs/reserved-bits-edition-audit.md` §6.1, class A, stable).
pub const RESERVED_FUTURE_USE_DEFAULT: u8 = 1;
const REFERENCE_TYPE_MASK: u8 = 0x07;
/// Max value of the 4-bit announcement_type field.
pub const ANNOUNCEMENT_TYPE_MAX: u8 = 0x0F;
/// Max value of the 3-bit reference_type field.
pub const REFERENCE_TYPE_MAX: u8 = 0x07;

/// Announcement type — ETSI EN 300 468 Table 19.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AnnouncementType {
    /// 0x00 — Emergency alarm.
    EmergencyAlarm,
    /// 0x01 — Road Traffic flash.
    RoadTrafficFlash,
    /// 0x02 — Public Transport flash.
    PublicTransportFlash,
    /// 0x03 — Warning message.
    WarningMessage,
    /// 0x04 — News flash.
    NewsFlash,
    /// 0x05 — Weather flash.
    WeatherFlash,
    /// 0x06 — Event announcement.
    EventAnnouncement,
    /// 0x07 — Personal call.
    PersonalCall,
    /// Reserved/unallocated wire value, preserved verbatim for round-trip.
    Reserved(u8),
}

impl AnnouncementType {
    #[must_use]
    /// Creates a value from a wire byte, preserving every possible
    /// byte value for lossless round-trip.
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::EmergencyAlarm,
            0x01 => Self::RoadTrafficFlash,
            0x02 => Self::PublicTransportFlash,
            0x03 => Self::WarningMessage,
            0x04 => Self::NewsFlash,
            0x05 => Self::WeatherFlash,
            0x06 => Self::EventAnnouncement,
            0x07 => Self::PersonalCall,
            v => Self::Reserved(v),
        }
    }

    #[must_use]
    /// Returns the wire byte for this value.
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::EmergencyAlarm => 0x00,
            Self::RoadTrafficFlash => 0x01,
            Self::PublicTransportFlash => 0x02,
            Self::WarningMessage => 0x03,
            Self::NewsFlash => 0x04,
            Self::WeatherFlash => 0x05,
            Self::EventAnnouncement => 0x06,
            Self::PersonalCall => 0x07,
            Self::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Returns a human-readable spec name for this value.
    pub fn name(self) -> &'static str {
        match self {
            Self::EmergencyAlarm => "Emergency alarm",
            Self::RoadTrafficFlash => "Road Traffic flash",
            Self::PublicTransportFlash => "Public Transport flash",
            Self::WarningMessage => "Warning message",
            Self::NewsFlash => "News flash",
            Self::WeatherFlash => "Weather flash",
            Self::EventAnnouncement => "Event announcement",
            Self::PersonalCall => "Personal call",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(AnnouncementType, Reserved);

/// Reference type — ETSI EN 300 468 Table 20.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ReferenceType {
    /// 0x00 — broadcast in the usual audio stream of the service.
    UsualAudioStream,
    /// 0x01 — broadcast in a separate audio stream that is part of the service.
    SeparateAudioStream,
    /// 0x02 — broadcast by a different service within the same DVB transport
    /// stream.
    DifferentServiceSameTs,
    /// 0x03 — broadcast by a different service within a different DVB transport
    /// stream.
    DifferentServiceDifferentTs,
    /// Reserved/unallocated wire value, preserved verbatim for round-trip.
    Reserved(u8),
}

impl ReferenceType {
    #[must_use]
    /// Creates a value from a wire byte, preserving every possible
    /// byte value for lossless round-trip.
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::UsualAudioStream,
            0x01 => Self::SeparateAudioStream,
            0x02 => Self::DifferentServiceSameTs,
            0x03 => Self::DifferentServiceDifferentTs,
            v => Self::Reserved(v),
        }
    }

    #[must_use]
    /// Returns the wire byte for this value.
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::UsualAudioStream => 0x00,
            Self::SeparateAudioStream => 0x01,
            Self::DifferentServiceSameTs => 0x02,
            Self::DifferentServiceDifferentTs => 0x03,
            Self::Reserved(v) => v,
        }
    }

    #[must_use]
    /// Returns a human-readable spec name for this value.
    pub fn name(self) -> &'static str {
        match self {
            Self::UsualAudioStream => "broadcast in the usual audio stream of the service",
            Self::SeparateAudioStream => {
                "broadcast in a separate audio stream that is part of the service"
            }
            Self::DifferentServiceSameTs => {
                "broadcast by a different service within the same DVB transport stream"
            }
            Self::DifferentServiceDifferentTs => {
                "broadcast by a different service within a different DVB transport stream"
            }
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(ReferenceType, Reserved);

/// Conditional reference block, present for reference_type 1, 2, 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AnnouncementReference {
    /// original_network_id of the carrying TS.
    pub original_network_id: u16,
    /// transport_stream_id of the carrying TS.
    pub transport_stream_id: u16,
    /// service_id carrying the announcement.
    pub service_id: u16,
    /// component_tag identifying the elementary stream.
    pub component_tag: u8,
}

/// One announcement entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AnnouncementEntry {
    /// `reserved_future_use`(1) between announcement_type and reference_type
    /// (1 bit, right-aligned). Captured on parse and re-emitted verbatim;
    /// [`AnnouncementEntry::new`] defaults to [`RESERVED_FUTURE_USE_DEFAULT`].
    pub reserved_future_use: u8,
    /// 4-bit announcement_type (Table 19).
    pub announcement_type: AnnouncementType,
    /// 3-bit reference_type (Table 20).
    pub reference_type: ReferenceType,
    /// Reference block, present iff reference_type ∈ {1,2,3}.
    pub reference: Option<AnnouncementReference>,
}

impl AnnouncementEntry {
    /// Build an entry with the spec-default reserved bit.
    #[must_use]
    pub const fn new(
        announcement_type: AnnouncementType,
        reference_type: ReferenceType,
        reference: Option<AnnouncementReference>,
    ) -> Self {
        Self {
            reserved_future_use: RESERVED_FUTURE_USE_DEFAULT,
            announcement_type,
            reference_type,
            reference,
        }
    }
}

/// Announcement Support Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AnnouncementSupportDescriptor {
    /// 16-bit announcement_support_indicator flag field (TS 101 154 C.4.3).
    pub announcement_support_indicator: u16,
    /// Announcement entries in wire order.
    pub entries: Vec<AnnouncementEntry>,
}

#[inline]
fn reference_present(reference_type: u8) -> bool {
    matches!(reference_type, 0x01..=0x03)
}

impl<'a> Parse<'a> for AnnouncementSupportDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "AnnouncementSupportDescriptor",
            "unexpected tag for announcement_support_descriptor",
        )?;
        let (ind_bytes, mut rest) =
            body.split_first_chunk::<INDICATOR_LEN>()
                .ok_or(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "body too short (need announcement_support_indicator)",
                })?;
        let announcement_support_indicator = u16::from_be_bytes(*ind_bytes);
        let mut entries = Vec::new();
        while !rest.is_empty() {
            let (type_byte, after_type) =
                rest.split_first_chunk::<TYPE_BYTE_LEN>()
                    .ok_or(Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "announcement reference block truncated",
                    })?;
            let flags = type_byte[0];
            // reserved_future_use captured below (preserved on serialize).
            let announcement_type = AnnouncementType::from_u8(
                (flags & ANNOUNCEMENT_TYPE_MASK) >> ANNOUNCEMENT_TYPE_SHIFT,
            );
            let reference_type = ReferenceType::from_u8(flags & REFERENCE_TYPE_MASK);
            let (reference, after_ref) = if reference_present(reference_type.to_u8()) {
                let (ref_bytes, tail) = after_type.split_first_chunk::<REFERENCE_LEN>().ok_or(
                    Error::InvalidDescriptor {
                        tag: TAG,
                        reason: "announcement reference block truncated",
                    },
                )?;
                let r = AnnouncementReference {
                    original_network_id: u16::from_be_bytes([ref_bytes[0], ref_bytes[1]]),
                    transport_stream_id: u16::from_be_bytes([ref_bytes[2], ref_bytes[3]]),
                    service_id: u16::from_be_bytes([ref_bytes[4], ref_bytes[5]]),
                    component_tag: ref_bytes[6],
                };
                (Some(r), tail)
            } else {
                (None, after_type)
            };
            rest = after_ref;
            entries.push(AnnouncementEntry {
                reserved_future_use: (flags & RESERVED_BIT_MASK) >> RESERVED_BIT_SHIFT,
                announcement_type,
                reference_type,
                reference,
            });
        }
        Ok(Self {
            announcement_support_indicator,
            entries,
        })
    }
}

impl AnnouncementSupportDescriptor {
    fn body_len(&self) -> usize {
        INDICATOR_LEN
            + self
                .entries
                .iter()
                .map(|e| {
                    TYPE_BYTE_LEN
                        + if reference_present(e.reference_type.to_u8()) {
                            REFERENCE_LEN
                        } else {
                            0
                        }
                })
                .sum::<usize>()
    }
}

impl Serialize for AnnouncementSupportDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.body_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        for e in &self.entries {
            if e.announcement_type.to_u8() > ANNOUNCEMENT_TYPE_MAX {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "announcement_type exceeds 4 bits",
                });
            }
            if e.reference_type.to_u8() > REFERENCE_TYPE_MAX {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "reference_type exceeds 3 bits",
                });
            }
            // A reference must be present exactly when reference_type ∈ {1,2,3}.
            if reference_present(e.reference_type.to_u8()) != e.reference.is_some() {
                return Err(Error::InvalidDescriptor {
                    tag: TAG,
                    reason: "reference presence does not match reference_type",
                });
            }
        }
        let body_len = self.body_len();
        if body_len > u8::MAX as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "announcement_support_descriptor body exceeds 255 bytes",
            });
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        crate::descriptors::write_descriptor_header(buf, TAG, body_len)?;
        buf[HEADER_LEN..HEADER_LEN + INDICATOR_LEN]
            .copy_from_slice(&self.announcement_support_indicator.to_be_bytes());
        let mut pos = HEADER_LEN + INDICATOR_LEN;
        for e in &self.entries {
            // reserved_future_use preserved from parse (default 1).
            let flags = (broadcast_common::len::fit_mask(
                e.announcement_type.to_u8(),
                ANNOUNCEMENT_TYPE_MASK >> ANNOUNCEMENT_TYPE_SHIFT,
                "announcement_type",
            )? << ANNOUNCEMENT_TYPE_SHIFT)
                | (broadcast_common::len::fit_mask(
                    e.reserved_future_use,
                    RESERVED_BIT_MASK >> RESERVED_BIT_SHIFT,
                    "reserved_future_use",
                )? << RESERVED_BIT_SHIFT)
                | broadcast_common::len::fit_mask(
                    e.reference_type.to_u8(),
                    REFERENCE_TYPE_MASK,
                    "reference_type",
                )?;
            buf[pos] = flags;
            pos += TYPE_BYTE_LEN;
            if let Some(r) = &e.reference {
                buf[pos..pos + 2].copy_from_slice(&r.original_network_id.to_be_bytes());
                buf[pos + 2..pos + 4].copy_from_slice(&r.transport_stream_id.to_be_bytes());
                buf[pos + 4..pos + 6].copy_from_slice(&r.service_id.to_be_bytes());
                buf[pos + 6] = r.component_tag;
                pos += REFERENCE_LEN;
            }
        }
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for AnnouncementSupportDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "ANNOUNCEMENT_SUPPORT";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_entry_without_reference() {
        // indicator=0x0040, one entry: type=0x06 (event), reserved=1, ref_type=0x00 (usual audio)
        // flags = 0110 1 000 = 0x68
        let bytes = [TAG, 3, 0x00, 0x40, 0x68];
        let d = AnnouncementSupportDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.announcement_support_indicator, 0x0040);
        assert_eq!(d.entries.len(), 1);
        assert_eq!(
            d.entries[0].announcement_type,
            AnnouncementType::EventAnnouncement
        );
        assert_eq!(d.entries[0].reference_type, ReferenceType::UsualAudioStream);
        assert!(d.entries[0].reference.is_none());
    }

    #[test]
    fn parse_entry_with_reference() {
        // indicator=0x0001, entry: type=0x01, ref_type=0x02 → reference present.
        // flags = 0001 1 010 = 0x1A
        let bytes = [
            TAG, 10, 0x00, 0x01,
            0x1A, // onid=0xAABB, tsid=0xCCDD, sid=0xEEFF, component_tag=0x09
            0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x09,
        ];
        let d = AnnouncementSupportDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 1);
        assert_eq!(
            d.entries[0].announcement_type,
            AnnouncementType::RoadTrafficFlash
        );
        assert_eq!(
            d.entries[0].reference_type,
            ReferenceType::DifferentServiceSameTs
        );
        let r = d.entries[0].reference.unwrap();
        assert_eq!(r.original_network_id, 0xAABB);
        assert_eq!(r.transport_stream_id, 0xCCDD);
        assert_eq!(r.service_id, 0xEEFF);
        assert_eq!(r.component_tag, 0x09);
    }

    #[test]
    fn parse_mixed_entries() {
        // entry1 ref_type=0 (no ref), entry2 ref_type=3 (ref present)
        let bytes = [
            TAG, 11, 0x12, 0x34, // indicator
            0x40, // type=4 ref_type=0
            0x53, // type=5 reserved=0 ref_type=3 -> 0101 0 011
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
        ];
        let d = AnnouncementSupportDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries.len(), 2);
        assert!(d.entries[0].reference.is_none());
        assert_eq!(
            d.entries[1].reference_type,
            ReferenceType::DifferentServiceDifferentTs
        );
        assert!(d.entries[1].reference.is_some());
    }

    #[test]
    fn parse_ignores_reserved_bit() {
        // reserved bit clear (0x60 vs 0x68): still parses.
        let bytes = [TAG, 3, 0x00, 0x00, 0x60];
        let d = AnnouncementSupportDescriptor::parse(&bytes).unwrap();
        assert_eq!(
            d.entries[0].announcement_type,
            AnnouncementType::EventAnnouncement
        );
        assert_eq!(d.entries[0].reference_type, ReferenceType::UsualAudioStream);
    }

    /// reserved_future_use(1) of every entry round-trips for both values;
    /// parsed values are preserved, only hand-built ones carry the default.
    #[test]
    fn reserved_preserved() {
        for r in 0u8..=1 {
            // type=6, ref_type=0 / type=1, ref_type=2 + 7-byte reference
            let e0 = 0x60 | (r << 3);
            let e1 = 0x12 | (r << 3);
            let bytes = [
                TAG, 11, 0x00, 0x40, e0, e1, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x09,
            ];
            let d = AnnouncementSupportDescriptor::parse(&bytes).unwrap();
            assert_eq!(d.entries[0].reserved_future_use, r);
            assert_eq!(d.entries[1].reserved_future_use, r);
            let mut buf = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut buf).unwrap();
            assert_eq!(buf, bytes);
        }
    }

    #[test]
    fn default_is_spec_value() {
        let e = AnnouncementEntry::new(
            AnnouncementType::EventAnnouncement,
            ReferenceType::UsualAudioStream,
            None,
        );
        let d = AnnouncementSupportDescriptor {
            announcement_support_indicator: 0,
            entries: vec![e],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [TAG, 3, 0x00, 0x00, 0x68]);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            AnnouncementSupportDescriptor::parse(&[0x6F, 2, 0, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x6F, .. }
        ));
    }

    #[test]
    fn parse_rejects_body_too_short_for_indicator() {
        assert!(matches!(
            AnnouncementSupportDescriptor::parse(&[TAG, 1, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_reference_truncated() {
        // ref_type=1 but no reference bytes.
        let bytes = [TAG, 3, 0x00, 0x00, 0x09]; // 0000 1 001
        assert!(matches!(
            AnnouncementSupportDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_buffer_shorter_than_length() {
        let bytes = [TAG, 5, 0x00, 0x00];
        assert!(matches!(
            AnnouncementSupportDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip_mixed() {
        let d = AnnouncementSupportDescriptor {
            announcement_support_indicator: 0xBEEF,
            entries: vec![
                AnnouncementEntry::new(
                    AnnouncementType::NewsFlash,
                    ReferenceType::UsualAudioStream,
                    None,
                ),
                AnnouncementEntry::new(
                    AnnouncementType::RoadTrafficFlash,
                    ReferenceType::DifferentServiceSameTs,
                    Some(AnnouncementReference {
                        original_network_id: 0xAABB,
                        transport_stream_id: 0xCCDD,
                        service_id: 0xEEFF,
                        component_tag: 0x09,
                    }),
                ),
            ],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(AnnouncementSupportDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = AnnouncementSupportDescriptor {
            announcement_support_indicator: 0,
            entries: vec![],
        };
        let mut buf = vec![0u8; 3];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[test]
    fn serialize_rejects_over_range_announcement_type() {
        let d = AnnouncementSupportDescriptor {
            announcement_support_indicator: 0,
            entries: vec![AnnouncementEntry::new(
                AnnouncementType::Reserved(0x10),
                ReferenceType::UsualAudioStream,
                None,
            )],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn serialize_rejects_reference_mismatch() {
        // ref_type=2 demands a reference, but None supplied.
        let d = AnnouncementSupportDescriptor {
            announcement_support_indicator: 0,
            entries: vec![AnnouncementEntry::new(
                AnnouncementType::Reserved(0),
                ReferenceType::DifferentServiceSameTs,
                None,
            )],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = AnnouncementSupportDescriptor {
            announcement_support_indicator: 0xBEEF,
            entries: vec![AnnouncementEntry::new(
                AnnouncementType::RoadTrafficFlash,
                ReferenceType::DifferentServiceSameTs,
                Some(AnnouncementReference {
                    original_network_id: 0xAABB,
                    transport_stream_id: 0xCCDD,
                    service_id: 0xEEFF,
                    component_tag: 0x09,
                }),
            )],
        };
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn announcement_type_full_range_round_trip() {
        for b in 0..=0xFF_u8 {
            let at = AnnouncementType::from_u8(b);
            assert_eq!(at.to_u8(), b, "round-trip failed for byte 0x{b:02X}");
        }
    }

    #[test]
    fn reference_type_full_range_round_trip() {
        for b in 0..=0xFF_u8 {
            let rt = ReferenceType::from_u8(b);
            assert_eq!(rt.to_u8(), b, "round-trip failed for byte 0x{b:02X}");
        }
    }
}
