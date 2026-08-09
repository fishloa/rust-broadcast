//! Codepoint (`CP`) delivery-object semantics — A/331 Annex A §A.3.6, Table
//! A.3.6, transcribed at `atsc3/docs/a331-route.md` §4.
//!
//! The LCT header's Codepoint field (`CP`, 8 bits — [`rmt_flute::LctHeader::codepoint`])
//! indicates the type of delivery object carried, either directly (`CP`
//! 0-9) or, for `CP` 128-255, by indirection through the signalling XML's
//! `SrcFlow.Payload@codePoint`-matched element (out of scope of this binary
//! crate — see `atsc3/docs/a331-route.md` §5's `SrcFlow` table).

use broadcast_common::impl_spec_display;

// ---------------------------------------------------------------------------
// FormatId — Table A.3.2 (delivery object format)
// ---------------------------------------------------------------------------

/// Delivery Object Format ID — A/331 Table A.3.2 (`SrcFlow.Payload@formatId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FormatId {
    /// `1` — File Mode (§A.3.3.2): a complete file or byte-range, described
    /// by an Extended FDT.
    FileMode,
    /// `2` — Entity Mode (§A.3.3.3): an HTTP/1.1 representation (entity/
    /// payload/response headers accompany the object).
    EntityMode,
    /// `3` — Unsigned Package Mode (§A.3.3.4): a `multipart/related` (RFC
    /// 2387) group of files.
    UnsignedPackageMode,
    /// `4` — Signed Package Mode (§A.3.3.5): as Unsigned Package Mode, but
    /// `multipart/signed` (S/MIME, RFC 5751).
    SignedPackageMode,
    /// `0`, or `>= 5` — ATSC Reserved.
    Reserved(u8),
}

impl FormatId {
    /// Decode a `formatId` value (Table A.3.2).
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => FormatId::FileMode,
            2 => FormatId::EntityMode,
            3 => FormatId::UnsignedPackageMode,
            4 => FormatId::SignedPackageMode,
            other => FormatId::Reserved(other),
        }
    }

    /// The wire `formatId` byte for this value.
    pub fn to_u8(self) -> u8 {
        match self {
            FormatId::FileMode => 1,
            FormatId::EntityMode => 2,
            FormatId::UnsignedPackageMode => 3,
            FormatId::SignedPackageMode => 4,
            FormatId::Reserved(v) => v,
        }
    }

    /// Spec label (Table A.3.2's "Meaning" column).
    pub fn name(&self) -> &'static str {
        match self {
            FormatId::FileMode => "File Mode",
            FormatId::EntityMode => "Entity Mode",
            FormatId::UnsignedPackageMode => "Unsigned Package Mode",
            FormatId::SignedPackageMode => "Signed Package Mode",
            FormatId::Reserved(_) => "reserved",
        }
    }
}

impl_spec_display!(FormatId, Reserved);

// ---------------------------------------------------------------------------
// FragMode — §A.3.6 detailed text (`SrcFlow.Payload@frag`)
// ---------------------------------------------------------------------------

/// Fragmentation mode — A/331 §A.3.6 (`SrcFlow.Payload@frag`, Table A.3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FragMode {
    /// `0` — arbitrary byte-boundary fragmentation.
    Arbitrary,
    /// `1` — application-specific, sample-based: one or more complete
    /// ISOBMFF samples (ISO/IEC 14496-12), used for MDE mode carrying an
    /// `mdat` box.
    SampleBased,
    /// `2` — application-specific, box-collection: one or more complete
    /// ISOBMFF boxes starting with a Random Access Point (e.g. `styp`/
    /// `sidx`/`moof`), used for MDE mode.
    BoxCollection,
    /// `3-255` — ATSC Reserved.
    Reserved(u8),
}

impl FragMode {
    /// Decode a `frag` value.
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => FragMode::Arbitrary,
            1 => FragMode::SampleBased,
            2 => FragMode::BoxCollection,
            other => FragMode::Reserved(other),
        }
    }

    /// The wire `frag` byte for this value.
    pub fn to_u8(self) -> u8 {
        match self {
            FragMode::Arbitrary => 0,
            FragMode::SampleBased => 1,
            FragMode::BoxCollection => 2,
            FragMode::Reserved(v) => v,
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            FragMode::Arbitrary => "arbitrary byte-boundary fragmentation",
            FragMode::SampleBased => "application-specific, sample-based",
            FragMode::BoxCollection => "application-specific, box-collection",
            FragMode::Reserved(_) => "reserved",
        }
    }
}

impl_spec_display!(FragMode, Reserved);

// ---------------------------------------------------------------------------
// CodepointSemantics — the (formatId, frag, order) triple Table A.3.6 gives
// directly for CP 0-9
// ---------------------------------------------------------------------------

/// The `(formatId, frag, order)` triple Table A.3.6 gives directly for `CP`
/// values 1-9 (indirected through `SrcFlow.Payload` for `CP` 128-255; not
/// applicable to `CP` 0 or 10-127, both "ATSC Reserved").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CodepointSemantics {
    /// Delivery object format.
    pub format_id: FormatId,
    /// Fragmentation mode.
    pub frag: FragMode,
    /// In-generation-order delivery indication.
    pub order: bool,
}

// ---------------------------------------------------------------------------
// Codepoint — Table A.3.6
// ---------------------------------------------------------------------------

/// The LCT Codepoint (`CP`) field's ROUTE-defined semantics — A/331 Table
/// A.3.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Codepoint {
    /// `1` — NRT, File Mode.
    NrtFileMode,
    /// `2` — NRT, Entity Mode.
    NrtEntityMode,
    /// `3` — NRT, Unsigned Package Mode.
    NrtUnsignedPackageMode,
    /// `4` — NRT, Signed Package Mode.
    NrtSignedPackageMode,
    /// `5` — new Initialization Segment (IS), timeline changed.
    NewInitSegmentTimelineChanged,
    /// `6` — new Initialization Segment, timeline continued.
    NewInitSegmentTimelineContinued,
    /// `7` — redundant Initialization Segment.
    RedundantInitSegment,
    /// `8` — Media Segment, File Mode.
    MediaSegmentFileMode,
    /// `9` — Media Segment, Entity Mode.
    MediaSegmentEntityMode,
    /// `0`, or `10-127` — ATSC Reserved (not used).
    Reserved(u8),
    /// `128-255` — attributes given by the `SrcFlow.Payload` element whose
    /// `@codePoint` matches this value (out of scope of this binary crate).
    Indirect(u8),
}

impl Codepoint {
    /// Decode an LCT `CP` byte (Table A.3.6).
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Codepoint::NrtFileMode,
            2 => Codepoint::NrtEntityMode,
            3 => Codepoint::NrtUnsignedPackageMode,
            4 => Codepoint::NrtSignedPackageMode,
            5 => Codepoint::NewInitSegmentTimelineChanged,
            6 => Codepoint::NewInitSegmentTimelineContinued,
            7 => Codepoint::RedundantInitSegment,
            8 => Codepoint::MediaSegmentFileMode,
            9 => Codepoint::MediaSegmentEntityMode,
            0 | 10..=127 => Codepoint::Reserved(v),
            other => Codepoint::Indirect(other),
        }
    }

    /// The wire `CP` byte for this value.
    pub fn to_u8(self) -> u8 {
        match self {
            Codepoint::NrtFileMode => 1,
            Codepoint::NrtEntityMode => 2,
            Codepoint::NrtUnsignedPackageMode => 3,
            Codepoint::NrtSignedPackageMode => 4,
            Codepoint::NewInitSegmentTimelineChanged => 5,
            Codepoint::NewInitSegmentTimelineContinued => 6,
            Codepoint::RedundantInitSegment => 7,
            Codepoint::MediaSegmentFileMode => 8,
            Codepoint::MediaSegmentEntityMode => 9,
            Codepoint::Reserved(v) => v,
            Codepoint::Indirect(v) => v,
        }
    }

    /// Spec label (Table A.3.6's "Semantics" column).
    pub fn name(&self) -> &'static str {
        match self {
            Codepoint::NrtFileMode => "NRT, File Mode",
            Codepoint::NrtEntityMode => "NRT, Entity Mode",
            Codepoint::NrtUnsignedPackageMode => "NRT, Unsigned Package Mode",
            Codepoint::NrtSignedPackageMode => "NRT, Signed Package Mode",
            Codepoint::NewInitSegmentTimelineChanged => {
                "new Initialization Segment, timeline changed"
            }
            Codepoint::NewInitSegmentTimelineContinued => {
                "new Initialization Segment, timeline continued"
            }
            Codepoint::RedundantInitSegment => "redundant Initialization Segment",
            Codepoint::MediaSegmentFileMode => "Media Segment, File Mode",
            Codepoint::MediaSegmentEntityMode => "Media Segment, Entity Mode",
            Codepoint::Reserved(_) => "reserved",
            Codepoint::Indirect(_) => "indirect (SrcFlow.Payload@codePoint)",
        }
    }

    /// The `(formatId, frag, order)` triple Table A.3.6 gives directly for
    /// `CP` 1-9. `None` for `CP` 0/10-127 (reserved, no semantics defined)
    /// and for `CP` 128-255 (semantics come from the signalling XML's
    /// `SrcFlow.Payload` element, not from the `CP` value alone).
    pub fn known_semantics(&self) -> Option<CodepointSemantics> {
        use Codepoint::*;
        let (format_id, frag, order) = match self {
            NrtFileMode => (FormatId::FileMode, FragMode::Arbitrary, true),
            NrtEntityMode => (FormatId::EntityMode, FragMode::Arbitrary, true),
            NrtUnsignedPackageMode => (FormatId::UnsignedPackageMode, FragMode::Arbitrary, true),
            NrtSignedPackageMode => (FormatId::SignedPackageMode, FragMode::Arbitrary, true),
            NewInitSegmentTimelineChanged => (FormatId::FileMode, FragMode::Arbitrary, true),
            NewInitSegmentTimelineContinued => (FormatId::FileMode, FragMode::Arbitrary, true),
            RedundantInitSegment => (FormatId::FileMode, FragMode::Arbitrary, true),
            MediaSegmentFileMode => (FormatId::FileMode, FragMode::SampleBased, true),
            MediaSegmentEntityMode => (FormatId::EntityMode, FragMode::SampleBased, true),
            Reserved(_) | Indirect(_) => return None,
        };
        Some(CodepointSemantics {
            format_id,
            frag,
            order,
        })
    }

    /// `true` for `CP` 128-255 — semantics are indirected through the
    /// signalling XML's `SrcFlow.Payload@codePoint`, not given by `CP` alone.
    pub fn is_indirect(&self) -> bool {
        matches!(self, Codepoint::Indirect(_))
    }
}

impl_spec_display!(Codepoint, Reserved, Indirect);

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn format_id_round_trips_known_values() {
        for v in [1u8, 2, 3, 4] {
            assert_eq!(FormatId::from_u8(v).to_u8(), v);
        }
        assert_eq!(FormatId::from_u8(0), FormatId::Reserved(0));
        assert_eq!(FormatId::from_u8(200), FormatId::Reserved(200));
    }

    #[test]
    fn frag_mode_round_trips_known_values() {
        for v in [0u8, 1, 2] {
            assert_eq!(FragMode::from_u8(v).to_u8(), v);
        }
        assert_eq!(FragMode::from_u8(3), FragMode::Reserved(3));
        assert_eq!(FragMode::from_u8(255), FragMode::Reserved(255));
    }

    #[test]
    fn codepoint_round_trips_every_defined_value() {
        for v in 0u8..=255 {
            assert_eq!(Codepoint::from_u8(v).to_u8(), v, "CP {v}");
        }
    }

    #[test]
    fn table_a_3_6_known_semantics_match_the_spec_table() {
        // CP 1-4: NRT modes, frag=0 (arbitrary), order=true.
        for (cp, fmt) in [
            (Codepoint::NrtFileMode, FormatId::FileMode),
            (Codepoint::NrtEntityMode, FormatId::EntityMode),
            (
                Codepoint::NrtUnsignedPackageMode,
                FormatId::UnsignedPackageMode,
            ),
            (Codepoint::NrtSignedPackageMode, FormatId::SignedPackageMode),
        ] {
            let s = cp.known_semantics().unwrap();
            assert_eq!(s.format_id, fmt);
            assert_eq!(s.frag, FragMode::Arbitrary);
            assert!(s.order);
        }

        // CP 5-7: File Mode, frag=0, order=true.
        for cp in [
            Codepoint::NewInitSegmentTimelineChanged,
            Codepoint::NewInitSegmentTimelineContinued,
            Codepoint::RedundantInitSegment,
        ] {
            let s = cp.known_semantics().unwrap();
            assert_eq!(s.format_id, FormatId::FileMode);
            assert_eq!(s.frag, FragMode::Arbitrary);
            assert!(s.order);
        }

        // CP 8: File Mode, frag=1 (sample-based).
        let s8 = Codepoint::MediaSegmentFileMode.known_semantics().unwrap();
        assert_eq!(s8.format_id, FormatId::FileMode);
        assert_eq!(s8.frag, FragMode::SampleBased);

        // CP 9: Entity Mode, frag=1 (sample-based).
        let s9 = Codepoint::MediaSegmentEntityMode.known_semantics().unwrap();
        assert_eq!(s9.format_id, FormatId::EntityMode);
        assert_eq!(s9.frag, FragMode::SampleBased);
    }

    #[test]
    fn reserved_and_indirect_have_no_known_semantics() {
        assert_eq!(Codepoint::from_u8(0).known_semantics(), None);
        assert_eq!(Codepoint::from_u8(50).known_semantics(), None);
        assert_eq!(Codepoint::from_u8(128).known_semantics(), None);
        assert_eq!(Codepoint::from_u8(255).known_semantics(), None);
    }

    #[test]
    fn is_indirect_covers_exactly_128_to_255() {
        assert!(!Codepoint::from_u8(9).is_indirect());
        assert!(!Codepoint::from_u8(50).is_indirect());
        assert!(Codepoint::from_u8(128).is_indirect());
        assert!(Codepoint::from_u8(255).is_indirect());
    }

    #[test]
    fn display_uses_name() {
        assert_eq!(Codepoint::NrtFileMode.to_string(), "NRT, File Mode");
        assert_eq!(Codepoint::from_u8(50).to_string(), "reserved(0x32)");
        assert_eq!(
            Codepoint::from_u8(128).to_string(),
            "indirect (SrcFlow.Payload@codePoint)(0x80)"
        );
        assert_eq!(FormatId::FileMode.to_string(), "File Mode");
        assert_eq!(
            FragMode::SampleBased.to_string(),
            "application-specific, sample-based"
        );
    }

    #[test]
    fn mutating_cp_changes_the_decoded_semantics() {
        let a = Codepoint::from_u8(8);
        let b = Codepoint::from_u8(9);
        assert_ne!(a, b);
        assert_ne!(a.known_semantics(), b.known_semantics());
    }
}
