//! `TableId` enum: typed `table_id` byte values.
//!
//! Source: ETSI EN 300 468 §5.1.3 Table 2 plus ISO/IEC 13818-1 for MPEG tables.

use num_enum::TryFromPrimitive;

/// Typed `table_id` enumeration.
///
/// Variants use Rust CamelCase in 4.0. They intentionally do not carry the
/// `Section` suffix; that suffix belongs to parser types such as
/// [`crate::tables::nit::NitSection`].
///
/// Tables that occupy a range of values (EIT schedule 0x50..=0x5F and 0x60..=0x6F)
/// are not listed as enum variants; see [`Self::eit_schedule_actual_segment`] and
/// [`Self::eit_schedule_other_segment`] instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, TryFromPrimitive)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(u8)]
#[non_exhaustive]
#[rustfmt::skip] // deliberate column-aligned discriminants
pub enum TableId {
    // ── MPEG-2 tables ──────────────────────────────────────────────────────
    /// `Pat` (table_id 0x00).
    Pat                        = 0x00,
    /// `Cat` (table_id 0x01).
    Cat                        = 0x01,
    /// `Pmt` (table_id 0x02).
    Pmt                        = 0x02,
    /// `TransportStreamDescription` (table_id 0x03).
    TransportStreamDescription = 0x03,
    /// `Iso14496SceneDescription` (table_id 0x04).
    Iso14496SceneDescription   = 0x04,
    /// `Iso14496ObjectDescriptor` (table_id 0x05).
    Iso14496ObjectDescriptor   = 0x05,
    /// `Metadata` (table_id 0x06).
    Metadata                   = 0x06,
    /// `Iso14496` (table_id 0x08).
    Iso14496                   = 0x08,
    /// `GreenAccessUnit` (table_id 0x09).
    GreenAccessUnit            = 0x09,

    // ── DVB SI tables ──────────────────────────────────────────────────────
    /// `NetworkInformationActual` (table_id 0x40).
    NetworkInformationActual   = 0x40,
    /// `NetworkInformationOther` (table_id 0x41).
    NetworkInformationOther    = 0x41,
    /// `ServiceDescriptionActual` (table_id 0x42).
    ServiceDescriptionActual   = 0x42,
    /// `ServiceDescriptionOther` (table_id 0x46).
    ServiceDescriptionOther    = 0x46,
    /// `BouquetAssociation` (table_id 0x4A).
    BouquetAssociation         = 0x4A,
    /// `UpdateNotification` (table_id 0x4B).
    UpdateNotification         = 0x4B,
    /// `IpMacNotification` (table_id 0x4C).
    IpMacNotification          = 0x4C,
    /// `SatelliteAccess` (table_id 0x4D).
    SatelliteAccess            = 0x4D,
    /// `EventInformationPfActual` (table_id 0x4E).
    EventInformationPfActual   = 0x4E,
    /// `EventInformationPfOther` (table_id 0x4F).
    EventInformationPfOther    = 0x4F,

    // EIT schedule actual covers 0x50..=0x5F (16 segments).
    // EIT schedule other covers 0x60..=0x6F.

    /// `TimeAndDate` (table_id 0x70).
    TimeAndDate                = 0x70,
    /// `RunningStatus` (table_id 0x71).
    RunningStatus              = 0x71,
    /// `Stuffing` (table_id 0x72).
    Stuffing                   = 0x72,
    /// `TimeOffset` (table_id 0x73).
    TimeOffset                 = 0x73,
    /// `ApplicationInformation` (table_id 0x74).
    ApplicationInformation     = 0x74,
    /// `Container` (table_id 0x75).
    Container                  = 0x75,
    /// `RelatedContent` (table_id 0x76).
    RelatedContent             = 0x76,
    /// `ContentIdentifier` (table_id 0x77).
    ContentIdentifier          = 0x77,
    /// `MpeFec` (table_id 0x78).
    MpeFec                     = 0x78,
    /// `ResolutionNotification` (table_id 0x79).
    ResolutionNotification     = 0x79,
    /// `MpeIfec` (table_id 0x7A).
    MpeIfec                    = 0x7A,
    /// `ProtectionMessage` (table_id 0x7B).
    ProtectionMessage          = 0x7B,
    /// `DownloadableFontInfo` (table_id 0x7C).
    DownloadableFontInfo       = 0x7C,
    /// `DiscontinuityInformation` (table_id 0x7E).
    DiscontinuityInformation   = 0x7E,
    /// `SelectionInformation` (table_id 0x7F).
    SelectionInformation       = 0x7F,
}

impl TableId {
    /// Human-readable spec name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Pat => "program_association_section",
            Self::Cat => "conditional_access_section",
            Self::Pmt => "program_map_section",
            Self::TransportStreamDescription => "transport_stream_description_section",
            Self::Iso14496SceneDescription => "ISO_IEC_14496_scene_description_section",
            Self::Iso14496ObjectDescriptor => "ISO_IEC_14496_object_descriptor_section",
            Self::Metadata => "Metadata_section",
            Self::Iso14496 => "ISO_IEC_14496_section",
            Self::GreenAccessUnit => "ISO/IEC 23001-11 green access unit section",
            Self::NetworkInformationActual => "network_information_section - actual network",
            Self::NetworkInformationOther => "network_information_section - other network",
            Self::ServiceDescriptionActual => {
                "service_description_section - actual DVB transport stream"
            }
            Self::ServiceDescriptionOther => {
                "service_description_section - other DVB transport stream"
            }
            Self::BouquetAssociation => "bouquet_association_section",
            Self::UpdateNotification => "update notification table section",
            Self::IpMacNotification => "IP/MAC_notification_section",
            Self::SatelliteAccess => "satellite_access_section",
            Self::EventInformationPfActual => {
                "event_information_section - actual DVB transport stream, present/following"
            }
            Self::EventInformationPfOther => {
                "event_information_section - other DVB transport stream, present/following"
            }
            Self::TimeAndDate => "time_date_section",
            Self::RunningStatus => "running_status_section",
            Self::Stuffing => "stuffing_section",
            Self::TimeOffset => "time_offset_section",
            Self::ApplicationInformation => "application information section",
            Self::Container => "container section",
            Self::RelatedContent => "related content section",
            Self::ContentIdentifier => "content identifier section",
            Self::MpeFec => "MPE-FEC section",
            Self::ResolutionNotification => "resolution provider notification section",
            Self::MpeIfec => "MPE-IFEC section",
            Self::ProtectionMessage => "protection message section",
            Self::DownloadableFontInfo => "downloadable font info section",
            Self::DiscontinuityInformation => "discontinuity_information_section",
            Self::SelectionInformation => "selection_information_section",
        }
    }

    /// If `v` is an EIT-schedule-actual `table_id` (0x50..=0x5F), return its
    /// segment index 0..=15. Otherwise `None`.
    #[must_use]
    pub const fn eit_schedule_actual_segment(v: u8) -> Option<u8> {
        if v >= 0x50 && v <= 0x5F {
            Some(v - 0x50)
        } else {
            None
        }
    }

    /// If `v` is an EIT-schedule-other `table_id` (0x60..=0x6F), return its
    /// segment index 0..=15. Otherwise `None`.
    #[must_use]
    pub const fn eit_schedule_other_segment(v: u8) -> Option<u8> {
        if v >= 0x60 && v <= 0x6F {
            Some(v - 0x60)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values_round_trip() {
        for id in [
            TableId::Pat,
            TableId::NetworkInformationActual,
            TableId::ServiceDescriptionActual,
            TableId::EventInformationPfActual,
            TableId::TimeAndDate,
            TableId::SelectionInformation,
        ] {
            let byte = id as u8;
            assert_eq!(TableId::try_from(byte), Ok(id));
        }
    }

    #[test]
    fn eit_schedule_actual_segment_range() {
        assert_eq!(TableId::eit_schedule_actual_segment(0x4F), None);
        assert_eq!(TableId::eit_schedule_actual_segment(0x50), Some(0));
        assert_eq!(TableId::eit_schedule_actual_segment(0x5F), Some(0x0F));
        assert_eq!(TableId::eit_schedule_actual_segment(0x60), None);
    }

    #[test]
    fn eit_schedule_other_segment_range() {
        assert_eq!(TableId::eit_schedule_other_segment(0x5F), None);
        assert_eq!(TableId::eit_schedule_other_segment(0x60), Some(0));
        assert_eq!(TableId::eit_schedule_other_segment(0x6F), Some(0x0F));
        assert_eq!(TableId::eit_schedule_other_segment(0x70), None);
    }

    #[test]
    fn unknown_value_rejected() {
        assert!(matches!(
            TableId::try_from(0x99),
            Err(num_enum::TryFromPrimitiveError { .. })
        ));
    }

    #[test]
    fn exhaustive_byte_sweep() {
        let mut matched = 0u16;
        for byte in 0u8..=0xFF {
            if let Ok(id) = TableId::try_from(byte) {
                assert_eq!(id as u8, byte, "round-trip failed for {byte:#04x}");
                matched += 1;
            }
        }
        // 34 defined variants (29 + the five typed MPEG-2 sections 0x04/05/06/08/09)
        assert_eq!(matched, 34, "expected 34 matched variants");
    }
}

broadcast_common::impl_spec_display!(TableId);
