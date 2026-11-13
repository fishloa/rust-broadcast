//! Segmentation assignment tables — ANSI/SCTE 35 2023r1 §10.3.3.1:
//! `device_restrictions` (Table 21), `segmentation_upid_type` (Table 22), and
//! `segmentation_type_id` (Table 23).
//!
//! These map the raw wire bytes to their spec-assigned names so callers never
//! re-implement the lookup tables. Each enum round-trips via `from_*`/`u8::from`
//! and any value not in the table is carried through `Reserved(raw)` losslessly.

/// `device_restrictions` — §10.3.3.1, Table 21 (2 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[rustfmt::skip] // deliberate column-aligned discriminants
#[non_exhaustive]
pub enum DeviceRestrictions {
    /// Restricted for device group 0 (out-of-band defined).
    RestrictGroup0 = 0b00,
    /// Restricted for device group 1.
    RestrictGroup1 = 0b01,
    /// Restricted for device group 2.
    RestrictGroup2 = 0b10,
    /// No device restrictions.
    None           = 0b11,
}

impl DeviceRestrictions {
    /// Decode the 2-bit field (only the low 2 bits are used).
    #[must_use]
    pub fn from_bits(v: u8) -> Self {
        match v & 0b11 {
            0b00 => Self::RestrictGroup0,
            0b01 => Self::RestrictGroup1,
            0b10 => Self::RestrictGroup2,
            _ => Self::None,
        }
    }

    /// The 2-bit wire value.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self as u8
    }

    /// Human-readable spec label (ANSI/SCTE 35 2023r1 Table 21).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::RestrictGroup0 => "restricted for device group 0",
            Self::RestrictGroup1 => "restricted for device group 1",
            Self::RestrictGroup2 => "restricted for device group 2",
            Self::None => "no device restrictions",
        }
    }
}
broadcast_common::impl_spec_display!(DeviceRestrictions);

/// `segmentation_upid_type` — §10.3.3.1, Table 22 (8 bits).
///
/// Unrecognised / reserved values are carried as [`SegmentationUpidType::Reserved`]
/// so the wire byte round-trips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[rustfmt::skip] // deliberate column-aligned table (values in from_u8/to_u8)
#[non_exhaustive]
pub enum SegmentationUpidType {
    /// NotUsed (0x00).
    NotUsed,               // 0x00
    /// UserDefinedDeprecated (0x01).
    UserDefinedDeprecated, // 0x01
    /// Isci (0x02).
    Isci,                  // 0x02
    /// AdId (0x03).
    AdId,                  // 0x03
    /// Umid (0x04).
    Umid,                  // 0x04
    /// IsanDeprecated (0x05).
    IsanDeprecated,        // 0x05
    /// Isan (0x06).
    Isan,                  // 0x06
    /// Tid (0x07).
    Tid,                   // 0x07
    /// Ti (0x08).
    Ti,                    // 0x08
    /// Adi (0x09).
    Adi,                   // 0x09
    /// Eidr (0x0A).
    Eidr,                  // 0x0A
    /// AtscContentIdentifier (0x0B).
    AtscContentIdentifier, // 0x0B
    /// Mpu (0x0C).
    Mpu,                   // 0x0C
    /// Mid (0x0D).
    Mid,                   // 0x0D
    /// AdsInformation (0x0E).
    AdsInformation,        // 0x0E
    /// Uri (0x0F).
    Uri,                   // 0x0F
    /// Uuid (0x10).
    Uuid,                  // 0x10
    /// Scr (0x11).
    Scr,                   // 0x11
    /// Any reserved / future value (0x12..=0xFF), carried verbatim.
    Reserved(u8),
}

impl SegmentationUpidType {
    /// Decode the 8-bit `segmentation_upid_type` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::NotUsed,
            0x01 => Self::UserDefinedDeprecated,
            0x02 => Self::Isci,
            0x03 => Self::AdId,
            0x04 => Self::Umid,
            0x05 => Self::IsanDeprecated,
            0x06 => Self::Isan,
            0x07 => Self::Tid,
            0x08 => Self::Ti,
            0x09 => Self::Adi,
            0x0A => Self::Eidr,
            0x0B => Self::AtscContentIdentifier,
            0x0C => Self::Mpu,
            0x0D => Self::Mid,
            0x0E => Self::AdsInformation,
            0x0F => Self::Uri,
            0x10 => Self::Uuid,
            0x11 => Self::Scr,
            other => Self::Reserved(other),
        }
    }

    /// The 8-bit wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::NotUsed => 0x00,
            Self::UserDefinedDeprecated => 0x01,
            Self::Isci => 0x02,
            Self::AdId => 0x03,
            Self::Umid => 0x04,
            Self::IsanDeprecated => 0x05,
            Self::Isan => 0x06,
            Self::Tid => 0x07,
            Self::Ti => 0x08,
            Self::Adi => 0x09,
            Self::Eidr => 0x0A,
            Self::AtscContentIdentifier => 0x0B,
            Self::Mpu => 0x0C,
            Self::Mid => 0x0D,
            Self::AdsInformation => 0x0E,
            Self::Uri => 0x0F,
            Self::Uuid => 0x10,
            Self::Scr => 0x11,
            Self::Reserved(v) => v,
        }
    }

    /// Human-readable spec label (ANSI/SCTE 35 2023r1 Table 22).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::NotUsed => "Not Used",
            Self::UserDefinedDeprecated => "User Defined (deprecated)",
            Self::Isci => "ISCI",
            Self::AdId => "Ad-ID",
            Self::Umid => "UMID",
            Self::IsanDeprecated => "ISAN (deprecated)",
            Self::Isan => "ISAN",
            Self::Tid => "TID",
            Self::Ti => "TI",
            Self::Adi => "ADI",
            Self::Eidr => "EIDR",
            Self::AtscContentIdentifier => "ATSC Content Identifier",
            Self::Mpu => "MPU()",
            Self::Mid => "MID()",
            Self::AdsInformation => "ADS Information",
            Self::Uri => "URI",
            Self::Uuid => "UUID",
            Self::Scr => "SCR",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(SegmentationUpidType, Reserved);

/// `segmentation_type_id` — §10.3.3.1, Table 23 (8 bits).
///
/// Unrecognised / reserved values are carried as
/// [`SegmentationTypeId::Reserved`] so the wire byte round-trips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[rustfmt::skip] // deliberate column-aligned table (values in from_u8/to_u8)
#[non_exhaustive]
pub enum SegmentationTypeId {
    /// NotIndicated (0x00).
    NotIndicated,                                // 0x00
    /// ContentIdentification (0x01).
    ContentIdentification,                       // 0x01
    /// Private (0x02).
    Private,                                     // 0x02
    /// ProgramStart (0x10).
    ProgramStart,                                // 0x10
    /// ProgramEnd (0x11).
    ProgramEnd,                                  // 0x11
    /// ProgramEarlyTermination (0x12).
    ProgramEarlyTermination,                     // 0x12
    /// ProgramBreakaway (0x13).
    ProgramBreakaway,                            // 0x13
    /// ProgramResumption (0x14).
    ProgramResumption,                           // 0x14
    /// ProgramRunoverPlanned (0x15).
    ProgramRunoverPlanned,                       // 0x15
    /// ProgramRunoverUnplanned (0x16).
    ProgramRunoverUnplanned,                     // 0x16
    /// ProgramOverlapStart (0x17).
    ProgramOverlapStart,                         // 0x17
    /// ProgramBlackoutOverride (0x18).
    ProgramBlackoutOverride,                     // 0x18
    /// ProgramJoin (0x19).
    ProgramJoin,                                 // 0x19
    /// ProgramImmediateResumption (0x1A).
    ProgramImmediateResumption,                  // 0x1A
    /// ChapterStart (0x20).
    ChapterStart,                                // 0x20
    /// ChapterEnd (0x21).
    ChapterEnd,                                  // 0x21
    /// BreakStart (0x22).
    BreakStart,                                  // 0x22
    /// BreakEnd (0x23).
    BreakEnd,                                     // 0x23
    /// OpeningCreditStart (0x24).
    OpeningCreditStart,                          // 0x24
    /// OpeningCreditEnd (0x25).
    OpeningCreditEnd,                            // 0x25
    /// ClosingCreditStart (0x26).
    ClosingCreditStart,                          // 0x26
    /// ClosingCreditEnd (0x27).
    ClosingCreditEnd,                            // 0x27
    /// ProviderAdvertisementStart (0x30).
    ProviderAdvertisementStart,                  // 0x30
    /// ProviderAdvertisementEnd (0x31).
    ProviderAdvertisementEnd,                    // 0x31
    /// DistributorAdvertisementStart (0x32).
    DistributorAdvertisementStart,               // 0x32
    /// DistributorAdvertisementEnd (0x33).
    DistributorAdvertisementEnd,                 // 0x33
    /// ProviderPlacementOpportunityStart (0x34).
    ProviderPlacementOpportunityStart,           // 0x34
    /// ProviderPlacementOpportunityEnd (0x35).
    ProviderPlacementOpportunityEnd,             // 0x35
    /// DistributorPlacementOpportunityStart (0x36).
    DistributorPlacementOpportunityStart,        // 0x36
    /// DistributorPlacementOpportunityEnd (0x37).
    DistributorPlacementOpportunityEnd,          // 0x37
    /// ProviderOverlayPlacementOpportunityStart (0x38).
    ProviderOverlayPlacementOpportunityStart,    // 0x38
    /// ProviderOverlayPlacementOpportunityEnd (0x39).
    ProviderOverlayPlacementOpportunityEnd,      // 0x39
    /// DistributorOverlayPlacementOpportunityStart (0x3A).
    DistributorOverlayPlacementOpportunityStart, // 0x3A
    /// DistributorOverlayPlacementOpportunityEnd (0x3B).
    DistributorOverlayPlacementOpportunityEnd,   // 0x3B
    /// ProviderPromoStart (0x3C).
    ProviderPromoStart,                          // 0x3C
    /// ProviderPromoEnd (0x3D).
    ProviderPromoEnd,                            // 0x3D
    /// DistributorPromoStart (0x3E).
    DistributorPromoStart,                       // 0x3E
    /// DistributorPromoEnd (0x3F).
    DistributorPromoEnd,                         // 0x3F
    /// UnscheduledEventStart (0x40).
    UnscheduledEventStart,                       // 0x40
    /// UnscheduledEventEnd (0x41).
    UnscheduledEventEnd,                         // 0x41
    /// AlternateContentOpportunityStart (0x42).
    AlternateContentOpportunityStart,            // 0x42
    /// AlternateContentOpportunityEnd (0x43).
    AlternateContentOpportunityEnd,              // 0x43
    /// ProviderAdBlockStart (0x44).
    ProviderAdBlockStart,                        // 0x44
    /// ProviderAdBlockEnd (0x45).
    ProviderAdBlockEnd,                          // 0x45
    /// DistributorAdBlockStart (0x46).
    DistributorAdBlockStart,                     // 0x46
    /// DistributorAdBlockEnd (0x47).
    DistributorAdBlockEnd,                       // 0x47
    /// NetworkStart (0x50).
    NetworkStart,                                // 0x50
    /// NetworkEnd (0x51).
    NetworkEnd,                                  // 0x51
    /// Any reserved / unassigned value, carried verbatim.
    Reserved(u8),
}

#[rustfmt::skip] // mirror the column-aligned enum above
impl SegmentationTypeId {
    /// Decode the 8-bit `segmentation_type_id` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::NotIndicated,
            0x01 => Self::ContentIdentification,
            0x02 => Self::Private,
            0x10 => Self::ProgramStart,
            0x11 => Self::ProgramEnd,
            0x12 => Self::ProgramEarlyTermination,
            0x13 => Self::ProgramBreakaway,
            0x14 => Self::ProgramResumption,
            0x15 => Self::ProgramRunoverPlanned,
            0x16 => Self::ProgramRunoverUnplanned,
            0x17 => Self::ProgramOverlapStart,
            0x18 => Self::ProgramBlackoutOverride,
            0x19 => Self::ProgramJoin,
            0x1A => Self::ProgramImmediateResumption,
            0x20 => Self::ChapterStart,
            0x21 => Self::ChapterEnd,
            0x22 => Self::BreakStart,
            0x23 => Self::BreakEnd,
            0x24 => Self::OpeningCreditStart,
            0x25 => Self::OpeningCreditEnd,
            0x26 => Self::ClosingCreditStart,
            0x27 => Self::ClosingCreditEnd,
            0x30 => Self::ProviderAdvertisementStart,
            0x31 => Self::ProviderAdvertisementEnd,
            0x32 => Self::DistributorAdvertisementStart,
            0x33 => Self::DistributorAdvertisementEnd,
            0x34 => Self::ProviderPlacementOpportunityStart,
            0x35 => Self::ProviderPlacementOpportunityEnd,
            0x36 => Self::DistributorPlacementOpportunityStart,
            0x37 => Self::DistributorPlacementOpportunityEnd,
            0x38 => Self::ProviderOverlayPlacementOpportunityStart,
            0x39 => Self::ProviderOverlayPlacementOpportunityEnd,
            0x3A => Self::DistributorOverlayPlacementOpportunityStart,
            0x3B => Self::DistributorOverlayPlacementOpportunityEnd,
            0x3C => Self::ProviderPromoStart,
            0x3D => Self::ProviderPromoEnd,
            0x3E => Self::DistributorPromoStart,
            0x3F => Self::DistributorPromoEnd,
            0x40 => Self::UnscheduledEventStart,
            0x41 => Self::UnscheduledEventEnd,
            0x42 => Self::AlternateContentOpportunityStart,
            0x43 => Self::AlternateContentOpportunityEnd,
            0x44 => Self::ProviderAdBlockStart,
            0x45 => Self::ProviderAdBlockEnd,
            0x46 => Self::DistributorAdBlockStart,
            0x47 => Self::DistributorAdBlockEnd,
            0x50 => Self::NetworkStart,
            0x51 => Self::NetworkEnd,
            other => Self::Reserved(other),
        }
    }

    /// The 8-bit wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::NotIndicated => 0x00,
            Self::ContentIdentification => 0x01,
            Self::Private => 0x02,
            Self::ProgramStart => 0x10,
            Self::ProgramEnd => 0x11,
            Self::ProgramEarlyTermination => 0x12,
            Self::ProgramBreakaway => 0x13,
            Self::ProgramResumption => 0x14,
            Self::ProgramRunoverPlanned => 0x15,
            Self::ProgramRunoverUnplanned => 0x16,
            Self::ProgramOverlapStart => 0x17,
            Self::ProgramBlackoutOverride => 0x18,
            Self::ProgramJoin => 0x19,
            Self::ProgramImmediateResumption => 0x1A,
            Self::ChapterStart => 0x20,
            Self::ChapterEnd => 0x21,
            Self::BreakStart => 0x22,
            Self::BreakEnd => 0x23,
            Self::OpeningCreditStart => 0x24,
            Self::OpeningCreditEnd => 0x25,
            Self::ClosingCreditStart => 0x26,
            Self::ClosingCreditEnd => 0x27,
            Self::ProviderAdvertisementStart => 0x30,
            Self::ProviderAdvertisementEnd => 0x31,
            Self::DistributorAdvertisementStart => 0x32,
            Self::DistributorAdvertisementEnd => 0x33,
            Self::ProviderPlacementOpportunityStart => 0x34,
            Self::ProviderPlacementOpportunityEnd => 0x35,
            Self::DistributorPlacementOpportunityStart => 0x36,
            Self::DistributorPlacementOpportunityEnd => 0x37,
            Self::ProviderOverlayPlacementOpportunityStart => 0x38,
            Self::ProviderOverlayPlacementOpportunityEnd => 0x39,
            Self::DistributorOverlayPlacementOpportunityStart => 0x3A,
            Self::DistributorOverlayPlacementOpportunityEnd => 0x3B,
            Self::ProviderPromoStart => 0x3C,
            Self::ProviderPromoEnd => 0x3D,
            Self::DistributorPromoStart => 0x3E,
            Self::DistributorPromoEnd => 0x3F,
            Self::UnscheduledEventStart => 0x40,
            Self::UnscheduledEventEnd => 0x41,
            Self::AlternateContentOpportunityStart => 0x42,
            Self::AlternateContentOpportunityEnd => 0x43,
            Self::ProviderAdBlockStart => 0x44,
            Self::ProviderAdBlockEnd => 0x45,
            Self::DistributorAdBlockStart => 0x46,
            Self::DistributorAdBlockEnd => 0x47,
            Self::NetworkStart => 0x50,
            Self::NetworkEnd => 0x51,
            Self::Reserved(v) => v,
        }
    }

    /// True for the eight types (Table 23) that may carry the optional
    /// `sub_segment_num` / `sub_segments_expected` pair: the Provider and
    /// Distributor Advertisement, Placement Opportunity, Overlay Placement
    /// Opportunity and Ad Block `Start` types.
    #[must_use]
    pub fn has_sub_segments(self) -> bool {
        matches!(
            self,
            Self::ProviderAdvertisementStart
                | Self::DistributorAdvertisementStart
                | Self::ProviderPlacementOpportunityStart
                | Self::DistributorPlacementOpportunityStart
                | Self::ProviderOverlayPlacementOpportunityStart
                | Self::DistributorOverlayPlacementOpportunityStart
                | Self::ProviderAdBlockStart
                | Self::DistributorAdBlockStart
        )
    }
}

impl SegmentationTypeId {
    /// Human-readable spec label (ANSI/SCTE 35 2023r1 Table 23).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::NotIndicated => "Not Indicated",
            Self::ContentIdentification => "Content Identification",
            Self::Private => "Private",
            Self::ProgramStart => "Program Start",
            Self::ProgramEnd => "Program End",
            Self::ProgramEarlyTermination => "Program Early Termination",
            Self::ProgramBreakaway => "Program Breakaway",
            Self::ProgramResumption => "Program Resumption",
            Self::ProgramRunoverPlanned => "Program Runover Planned",
            Self::ProgramRunoverUnplanned => "Program Runover Unplanned",
            Self::ProgramOverlapStart => "Program Overlap Start",
            Self::ProgramBlackoutOverride => "Program Blackout Override",
            Self::ProgramJoin => "Program Join",
            Self::ProgramImmediateResumption => "Program Immediate Resumption",
            Self::ChapterStart => "Chapter Start",
            Self::ChapterEnd => "Chapter End",
            Self::BreakStart => "Break Start",
            Self::BreakEnd => "Break End",
            Self::OpeningCreditStart => "Opening Credit Start",
            Self::OpeningCreditEnd => "Opening Credit End",
            Self::ClosingCreditStart => "Closing Credit Start",
            Self::ClosingCreditEnd => "Closing Credit End",
            Self::ProviderAdvertisementStart => "Provider Advertisement Start",
            Self::ProviderAdvertisementEnd => "Provider Advertisement End",
            Self::DistributorAdvertisementStart => "Distributor Advertisement Start",
            Self::DistributorAdvertisementEnd => "Distributor Advertisement End",
            Self::ProviderPlacementOpportunityStart => "Provider Placement Opportunity Start",
            Self::ProviderPlacementOpportunityEnd => "Provider Placement Opportunity End",
            Self::DistributorPlacementOpportunityStart => "Distributor Placement Opportunity Start",
            Self::DistributorPlacementOpportunityEnd => "Distributor Placement Opportunity End",
            Self::ProviderOverlayPlacementOpportunityStart => {
                "Provider Overlay Placement Opportunity Start"
            }
            Self::ProviderOverlayPlacementOpportunityEnd => {
                "Provider Overlay Placement Opportunity End"
            }
            Self::DistributorOverlayPlacementOpportunityStart => {
                "Distributor Overlay Placement Opportunity Start"
            }
            Self::DistributorOverlayPlacementOpportunityEnd => {
                "Distributor Overlay Placement Opportunity End"
            }
            Self::ProviderPromoStart => "Provider Promo Start",
            Self::ProviderPromoEnd => "Provider Promo End",
            Self::DistributorPromoStart => "Distributor Promo Start",
            Self::DistributorPromoEnd => "Distributor Promo End",
            Self::UnscheduledEventStart => "Unscheduled Event Start",
            Self::UnscheduledEventEnd => "Unscheduled Event End",
            Self::AlternateContentOpportunityStart => "Alternate Content Opportunity Start",
            Self::AlternateContentOpportunityEnd => "Alternate Content Opportunity End",
            Self::ProviderAdBlockStart => "Provider Ad Block Start",
            Self::ProviderAdBlockEnd => "Provider Ad Block End",
            Self::DistributorAdBlockStart => "Distributor Ad Block Start",
            Self::DistributorAdBlockEnd => "Distributor Ad Block End",
            Self::NetworkStart => "Network Start",
            Self::NetworkEnd => "Network End",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(SegmentationTypeId, Reserved);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upid_type_round_trips_all_bytes() {
        for v in 0u8..=255 {
            assert_eq!(SegmentationUpidType::from_u8(v).to_u8(), v);
        }
    }

    #[test]
    fn type_id_round_trips_all_bytes() {
        for v in 0u8..=255 {
            assert_eq!(SegmentationTypeId::from_u8(v).to_u8(), v);
        }
    }

    #[test]
    fn device_restrictions_round_trips() {
        for v in 0u8..=3 {
            assert_eq!(DeviceRestrictions::from_bits(v).bits(), v);
        }
    }

    #[test]
    fn named_values_match_spec() {
        assert_eq!(
            SegmentationTypeId::ProviderPlacementOpportunityStart.to_u8(),
            0x34
        );
        assert_eq!(SegmentationUpidType::Mpu.to_u8(), 0x0C);
    }

    #[test]
    fn has_sub_segments_is_exactly_the_eight_table_23_types() {
        const SUB_SEGMENT_TYPES: [u8; 8] = [0x30, 0x32, 0x34, 0x36, 0x38, 0x3A, 0x44, 0x46];
        for v in 0..=u8::MAX {
            assert_eq!(
                SegmentationTypeId::from_u8(v).has_sub_segments(),
                SUB_SEGMENT_TYPES.contains(&v),
                "segmentation_type_id {v:#04x}"
            );
        }
    }
}
