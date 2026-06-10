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
#[allow(missing_docs)]
#[non_exhaustive]
#[rustfmt::skip] // deliberate column-aligned discriminants
pub enum TableId {
    // ── MPEG-2 tables ──────────────────────────────────────────────────────
    Pat                        = 0x00,
    Cat                        = 0x01,
    Pmt                        = 0x02,
    TransportStreamDescription = 0x03,

    // ── DVB SI tables ──────────────────────────────────────────────────────
    NetworkInformationActual   = 0x40,
    NetworkInformationOther    = 0x41,
    ServiceDescriptionActual   = 0x42,
    ServiceDescriptionOther    = 0x46,
    BouquetAssociation         = 0x4A,
    UpdateNotification         = 0x4B,
    IpMacNotification          = 0x4C,
    SatelliteAccess            = 0x4D,
    EventInformationPfActual   = 0x4E,
    EventInformationPfOther    = 0x4F,

    // EIT schedule actual covers 0x50..=0x5F (16 segments).
    // EIT schedule other covers 0x60..=0x6F.

    TimeAndDate                = 0x70,
    RunningStatus              = 0x71,
    Stuffing                   = 0x72,
    TimeOffset                 = 0x73,
    ApplicationInformation     = 0x74,
    Container                  = 0x75,
    RelatedContent             = 0x76,
    ContentIdentifier          = 0x77,
    MpeFec                     = 0x78,
    ResolutionNotification     = 0x79,
    MpeIfec                    = 0x7A,
    ProtectionMessage          = 0x7B,
    DownloadableFontInfo       = 0x7C,
    DiscontinuityInformation   = 0x7E,
    SelectionInformation       = 0x7F,
}

impl TableId {
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
        assert!(TableId::try_from(0x99).is_err());
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
        // 29 defined variants (added ProtectionMessage 0x7B, DownloadableFontInfo 0x7C)
        assert_eq!(matched, 29, "expected 29 matched variants");
    }
}
