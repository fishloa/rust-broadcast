//! DVB-TA profiling of base SCTE 35 — ETSI TS 103 752-1 V1.2.1 §5.3.4–5.3.5
//! (constraints only; **no new wire syntax**).
//!
//! These clauses do not define new wire structures. They pin the values / usage
//! of fields that base ANSI/SCTE 35 already defines and that this crate already
//! parses (`splice_info_section()`, `splice_insert()`, `time_signal()`,
//! `segmentation_descriptor()`). This module provides typed helpers layered over
//! the existing [`SegmentationTypeId`] so
//! callers can classify and validate Placement Opportunity (PPO/DPO) signalling
//! without re-encoding the spec's `segmentation_type_id` table.

use crate::descriptors::{SegmentationDescriptor, SegmentationTypeId, SegmentationUpidType};

/// Maximum DVB DAS `splice_info_section()` length in bytes when carried directly
/// on a TS PID (§5.3.4.2, per SCTE 35).
pub const MAX_SECTION_LEN_TS: usize = 4096;

/// `segmentation_upid_type` mandated for the DVB-TA `segmentation_descriptor()`
/// profile (§5.3.5.10): `0x0F` (URI, IETF RFC 3986).
pub const PROFILE_UPID_TYPE: SegmentationUpidType = SegmentationUpidType::Uri;

/// A Placement Opportunity boundary classification derived from a base SCTE 35
/// `segmentation_type_id` per §5.3.5.4. The four applicable PO type IDs are the
/// base SCTE 35 values `0x34`–`0x37`; DVB-TA simply mandates their use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum PlacementOpportunity {
    /// `0x34` — Provider Placement Opportunity Start.
    ProviderStart,
    /// `0x35` — Provider Placement Opportunity End.
    ProviderEnd,
    /// `0x36` — Distributor Placement Opportunity Start.
    DistributorStart,
    /// `0x37` — Distributor Placement Opportunity End.
    DistributorEnd,
}

impl PlacementOpportunity {
    /// Classify a [`SegmentationTypeId`] as a DVB-TA Placement Opportunity
    /// boundary, or `None` if it is not one of the four PO type IDs (§5.3.5.4).
    #[must_use]
    pub fn from_segmentation_type_id(id: SegmentationTypeId) -> Option<Self> {
        match id {
            SegmentationTypeId::ProviderPlacementOpportunityStart => Some(Self::ProviderStart),
            SegmentationTypeId::ProviderPlacementOpportunityEnd => Some(Self::ProviderEnd),
            SegmentationTypeId::DistributorPlacementOpportunityStart => {
                Some(Self::DistributorStart)
            }
            SegmentationTypeId::DistributorPlacementOpportunityEnd => Some(Self::DistributorEnd),
            _ => None,
        }
    }

    /// The corresponding base SCTE 35 [`SegmentationTypeId`].
    #[must_use]
    pub fn segmentation_type_id(self) -> SegmentationTypeId {
        match self {
            Self::ProviderStart => SegmentationTypeId::ProviderPlacementOpportunityStart,
            Self::ProviderEnd => SegmentationTypeId::ProviderPlacementOpportunityEnd,
            Self::DistributorStart => SegmentationTypeId::DistributorPlacementOpportunityStart,
            Self::DistributorEnd => SegmentationTypeId::DistributorPlacementOpportunityEnd,
        }
    }

    /// `true` for the two Start boundaries (`0x34`, `0x36`). A Start carries the
    /// PO information; the matching End conveys no extra info (§5.3.5.4).
    #[must_use]
    pub fn is_start(self) -> bool {
        matches!(self, Self::ProviderStart | Self::DistributorStart)
    }

    /// `true` for a Provider PO (`0x34`/`0x35`), `false` for a Distributor PO.
    #[must_use]
    pub fn is_provider(self) -> bool {
        matches!(self, Self::ProviderStart | Self::ProviderEnd)
    }

    /// Human-readable spec label (ETSI TS 103 752-1 §5.3.5.4 / SCTE 35 Table 23).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ProviderStart => "Provider Placement Opportunity Start",
            Self::ProviderEnd => "Provider Placement Opportunity End",
            Self::DistributorStart => "Distributor Placement Opportunity Start",
            Self::DistributorEnd => "Distributor Placement Opportunity End",
        }
    }
}
dvb_common::impl_spec_display!(PlacementOpportunity);

/// Outcome of validating a `segmentation_descriptor()` against the DVB-TA
/// profile constraints (§5.3.5). A returned variant pins the first failing
/// constraint; [`ProfileViolation::Ok`] means all checked constraints hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ProfileViolation {
    /// All checked DVB-TA constraints hold.
    Ok,
    /// `segmentation_event_cancel_indicator` was set; shall be `0` (§5.3.5.3).
    EventCancelled,
    /// `segmentation_duration_flag` was clear on a Start; shall be `1`
    /// (§5.3.5.5).
    DurationMissing,
    /// `segmentation_upid_type` was not `0x0F` (URI); shall be `0x0F`
    /// (§5.3.5.10).
    UpidTypeNotUri,
}

impl ProfileViolation {
    /// Human-readable spec label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::EventCancelled => "segmentation_event_cancel_indicator set (§5.3.5.3)",
            Self::DurationMissing => "segmentation_duration_flag clear on a Start (§5.3.5.5)",
            Self::UpidTypeNotUri => "segmentation_upid_type is not 0x0F URI (§5.3.5.10)",
        }
    }
}
dvb_common::impl_spec_display!(ProfileViolation);

/// Validate a `segmentation_descriptor()` against the DVB-TA Placement
/// Opportunity profile (§5.3.5). Only PO-typed descriptors (`0x34`–`0x37`) are
/// validated against the duration/UPID rules; for any other type only the
/// universal cancel constraint (§5.3.5.3) is checked.
#[must_use]
pub fn validate_po_segmentation(d: &SegmentationDescriptor<'_>) -> ProfileViolation {
    // §5.3.5.3 — event cancellation not permitted for DVB DAS.
    if d.segmentation_event_cancel_indicator {
        return ProfileViolation::EventCancelled;
    }
    let po = PlacementOpportunity::from_segmentation_type_id(d.segmentation_type_id);
    if let Some(po) = po {
        // §5.3.5.10 — UPID type shall be 0x0F (URI).
        if d.segmentation_upid_type != PROFILE_UPID_TYPE {
            return ProfileViolation::UpidTypeNotUri;
        }
        // §5.3.5.5 — duration shall be specified (Start messages).
        if po.is_start() && d.segmentation_duration.is_none() {
            return ProfileViolation::DurationMissing;
        }
    }
    ProfileViolation::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_the_four_po_type_ids() {
        assert_eq!(
            PlacementOpportunity::from_segmentation_type_id(
                SegmentationTypeId::ProviderPlacementOpportunityStart
            ),
            Some(PlacementOpportunity::ProviderStart)
        );
        assert_eq!(
            PlacementOpportunity::from_segmentation_type_id(
                SegmentationTypeId::DistributorPlacementOpportunityEnd
            ),
            Some(PlacementOpportunity::DistributorEnd)
        );
        // A non-PO type is not classified.
        assert_eq!(
            PlacementOpportunity::from_segmentation_type_id(SegmentationTypeId::ProgramStart),
            None
        );
    }

    #[test]
    fn po_round_trips_type_id_and_flags() {
        for po in [
            PlacementOpportunity::ProviderStart,
            PlacementOpportunity::ProviderEnd,
            PlacementOpportunity::DistributorStart,
            PlacementOpportunity::DistributorEnd,
        ] {
            let id = po.segmentation_type_id();
            assert_eq!(
                PlacementOpportunity::from_segmentation_type_id(id),
                Some(po)
            );
        }
        assert!(PlacementOpportunity::ProviderStart.is_start());
        assert!(!PlacementOpportunity::ProviderEnd.is_start());
        assert!(PlacementOpportunity::ProviderStart.is_provider());
        assert!(!PlacementOpportunity::DistributorStart.is_provider());
        // The mandated PO type IDs are the SCTE 35 0x34..=0x37 block.
        assert_eq!(
            PlacementOpportunity::ProviderStart
                .segmentation_type_id()
                .to_u8(),
            0x34
        );
        assert_eq!(
            PlacementOpportunity::DistributorEnd
                .segmentation_type_id()
                .to_u8(),
            0x37
        );
    }
}
