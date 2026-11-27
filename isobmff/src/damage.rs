//! The crate's report enums as the workspace's one report,
//! [`broadcast_core::damage::Damage`].
//!
//! The readers publish content they skipped through `Reader::step`'s
//! [`Step::Damage`](crate::reader::Step::Damage) and `ItemReader::step`'s
//! [`ItemStep::Damage`](crate::item_reader::ItemStep::Damage), so the channel
//! speaks one vocabulary. [`Warning`], [`ImageFinding`] and [`BrandFinding`]
//! stay as the crate's typed detail and convert here into a `Damage`:
//!
//! - `what` is the fault class, the same stable token the typed enum's
//!   `name()` reports; for a box kept uninterpreted it is the parser's reason
//!   class ([`OpaqueReason::name`](crate::error::OpaqueReason::name));
//! - `fourcc` is the four-character code of the box or atom involved;
//! - the byte range is the fault's place in the input, with a zero length for
//!   a point of loss;
//! - `recoverable_bytes` says whether the bytes are still carried verbatim.
//!
//! An item-scoped report has no track: its item or group identifier goes in
//! [`Damage::with_ordinal`], the location's format-agnostic unit number.

use broadcast_core::damage::{ByteRange as DamageByteRange, Damage, DamageKind, Severity};

use crate::image_brands::{BrandFinding, ImageFinding};
use crate::reader::Warning;

impl Warning {
    /// This fault as the workspace's one report.
    #[must_use]
    pub fn to_damage(self) -> Damage {
        let what = match &self {
            Self::UnrepresentableBox { reason, .. } => reason.name(),
            other => other.name(),
        };
        let (kind, severity, fourcc, range, recoverable) = match self {
            Self::ClampedBox { box_type, offset } => (
                DamageKind::MetadataSkipped,
                Severity::Informational,
                Some(box_type),
                Some(DamageByteRange::new(offset, 0)),
                true,
            ),
            Self::UnparseableSampleEntry { fourcc, offset } => (
                DamageKind::MetadataUninterpreted,
                Severity::Degraded,
                Some(fourcc),
                Some(DamageByteRange::new(offset, 0)),
                true,
            ),
            Self::WaveWithoutEsds { offset } => (
                DamageKind::MetadataUninterpreted,
                Severity::Informational,
                Some(*b"wave"),
                Some(DamageByteRange::new(offset, 0)),
                true,
            ),
            Self::ZeroMovieTimescale { offset } => (
                DamageKind::TimelineRepaired,
                Severity::Degraded,
                Some(*b"mvhd"),
                Some(DamageByteRange::new(offset, 0)),
                false,
            ),
            Self::TruncatedMdat { offset } => (
                DamageKind::InputTruncated,
                Severity::Degraded,
                Some(*b"mdat"),
                Some(DamageByteRange::new(offset, 0)),
                false,
            ),
            Self::TruncatedFragment { offset } => (
                DamageKind::InputTruncated,
                Severity::Degraded,
                Some(*b"moof"),
                Some(DamageByteRange::new(offset, 0)),
                false,
            ),
            Self::MalformedFragment { offset } => (
                DamageKind::Resynchronised,
                Severity::Degraded,
                Some(*b"moof"),
                Some(DamageByteRange::new(offset, 0)),
                false,
            ),
            Self::TrailingBytes {
                box_type,
                offset,
                carried,
            } => (
                DamageKind::MetadataUninterpreted,
                Severity::Informational,
                Some(box_type),
                Some(DamageByteRange::new(offset, carried)),
                true,
            ),
            Self::Resynchronised { offset } => (
                DamageKind::Resynchronised,
                Severity::Degraded,
                None,
                Some(DamageByteRange::new(offset, 0)),
                false,
            ),
            Self::RebasedFragment { offset } => (
                DamageKind::TimelineRepaired,
                Severity::Degraded,
                None,
                Some(DamageByteRange::new(offset, 0)),
                false,
            ),
            Self::SynthesizedBox { box_type, offset } => (
                DamageKind::MetadataSkipped,
                Severity::Degraded,
                Some(box_type),
                Some(DamageByteRange::new(offset, 0)),
                false,
            ),
            Self::UnrepresentableBox {
                box_type, offset, ..
            } => (
                DamageKind::MetadataUninterpreted,
                Severity::Informational,
                Some(box_type),
                Some(DamageByteRange::new(offset, 0)),
                true,
            ),
        };
        let mut damage = Damage::new(kind, severity, what).with_recoverable_bytes(recoverable);
        if let Some(range) = range {
            damage = damage.at_byte_range(range);
        }
        if let Some(fourcc) = fourcc {
            damage = damage.with_fourcc(fourcc);
        }
        damage
    }
}

impl From<Warning> for Damage {
    fn from(warning: Warning) -> Self {
        warning.to_damage()
    }
}

impl ImageFinding {
    /// This finding as the workspace's one report.
    ///
    /// A finding that names a box or a place in the file carries that byte
    /// range; a finding that names an item or a group carries its identifier
    /// as the report's ordinal.
    #[must_use]
    pub fn to_damage(self) -> Damage {
        let what = match &self {
            Self::UnrepresentableBox { reason, .. } => reason.name(),
            other => other.name(),
        };
        let (kind, severity, fourcc, range, recoverable, ordinal) = match self {
            Self::UnrepresentableBox {
                box_type, offset, ..
            } => (
                DamageKind::MetadataUninterpreted,
                Severity::Informational,
                Some(box_type),
                Some(DamageByteRange::new(offset, 0)),
                true,
                None,
            ),
            Self::TrailingBytes {
                box_type,
                offset,
                carried,
            } => (
                DamageKind::MetadataUninterpreted,
                Severity::Informational,
                Some(box_type),
                Some(DamageByteRange::new(offset, carried)),
                true,
                None,
            ),
            Self::ExtraMetaBox { offset } => (
                DamageKind::MetadataUninterpreted,
                Severity::Informational,
                Some(*b"meta"),
                Some(DamageByteRange::new(offset, 0)),
                true,
                None,
            ),
            Self::TruncatedBox { box_type, offset } => (
                DamageKind::InputTruncated,
                Severity::Degraded,
                Some(box_type),
                Some(DamageByteRange::new(offset, 0)),
                false,
                None,
            ),
            Self::ItemDataDropped {
                item,
                offset,
                length,
            } => (
                DamageKind::MediaUnitDropped,
                Severity::Degraded,
                None,
                Some(DamageByteRange::new(offset, length)),
                false,
                Some(u64::from(item.get())),
            ),
            Self::ToneMapWrongInputCount { item, .. }
            | Self::RegionNamesMissingImage { item, .. }
            | Self::RegionMaskReferenceCount { item, .. }
            | Self::GainMapZeroDenominator { item, .. } => (
                DamageKind::MetadataSkipped,
                Severity::Informational,
                None,
                None,
                false,
                Some(u64::from(item.get())),
            ),
            Self::GroupNamesMissingEntity { group, .. }
            | Self::StereoGroupWrongEntityCount { group, .. }
            | Self::StereoFallbackGroupWrongEntityCount { group, .. }
            | Self::AudioImageGroupWrongEntityCount { group, .. } => (
                DamageKind::MetadataSkipped,
                Severity::Informational,
                None,
                None,
                false,
                Some(u64::from(group.get())),
            ),
        };
        let mut damage = Damage::new(kind, severity, what).with_recoverable_bytes(recoverable);
        if let Some(range) = range {
            damage = damage.at_byte_range(range);
        }
        if let Some(fourcc) = fourcc {
            damage = damage.with_fourcc(fourcc);
        }
        if let Some(ordinal) = ordinal {
            damage = damage.with_ordinal(ordinal);
        }
        damage
    }
}

impl From<ImageFinding> for Damage {
    fn from(finding: ImageFinding) -> Self {
        finding.to_damage()
    }
}

impl BrandFinding {
    /// This finding as the workspace's one report, about the file's `ftyp`.
    #[must_use]
    pub fn to_damage(self) -> Damage {
        Damage::new(
            DamageKind::MetadataSkipped,
            Severity::Informational,
            self.name(),
        )
        .with_fourcc(*b"ftyp")
    }
}

impl From<BrandFinding> for Damage {
    fn from(finding: BrandFinding) -> Self {
        finding.to_damage()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::OpaqueReason;

    #[test]
    fn every_warning_carries_its_place_the_code_and_its_recoverability() {
        let cases = [
            Warning::ClampedBox {
                box_type: *b"trak",
                offset: 16,
            },
            Warning::UnparseableSampleEntry {
                fourcc: *b"avc1",
                offset: 32,
            },
            Warning::WaveWithoutEsds { offset: 48 },
            Warning::ZeroMovieTimescale { offset: 64 },
            Warning::TruncatedMdat { offset: 80 },
            Warning::TruncatedFragment { offset: 96 },
            Warning::MalformedFragment { offset: 112 },
            Warning::TrailingBytes {
                box_type: *b"stsz",
                offset: 128,
                carried: 4,
            },
            Warning::UnrepresentableBox {
                box_type: *b"trgr",
                offset: 144,
                reason: OpaqueReason::BufferTooShort,
            },
        ];
        for warning in cases {
            let expected_offset = warning.position();
            let expected_name = match &warning {
                Warning::UnrepresentableBox { reason, .. } => reason.name(),
                other => other.name(),
            };
            let damage = warning.to_damage();
            assert_eq!(damage.what(), expected_name, "the fault class");
            assert_eq!(
                damage.location().byte_range().map(|range| range.start()),
                Some(expected_offset),
                "the place in the input"
            );
            assert!(damage.fourcc().is_some(), "the four-character code");
            assert_eq!(Damage::from(warning), damage);
        }
    }

    #[test]
    fn an_uninterpretable_box_reports_the_parser_reason() {
        let damage = Warning::UnrepresentableBox {
            box_type: *b"trgr",
            offset: 144,
            reason: OpaqueReason::NotReproducible,
        }
        .to_damage();
        assert_eq!(damage.what(), OpaqueReason::NotReproducible.name());
        assert_eq!(damage.fourcc(), Some(*b"trgr"));
        assert!(damage.recoverable_bytes());
        assert_eq!(damage.kind(), DamageKind::MetadataUninterpreted);
    }

    #[test]
    fn a_truncated_tail_is_degraded_and_not_recoverable() {
        let damage = Warning::TruncatedMdat { offset: 1_024 }.to_damage();
        assert_eq!(damage.kind(), DamageKind::InputTruncated);
        assert_eq!(damage.severity(), Severity::Degraded);
        assert!(!damage.recoverable_bytes());
    }

    #[test]
    fn a_finding_names_its_item_or_group_as_the_ordinal() {
        let damage = ImageFinding::ToneMapWrongInputCount {
            item: crate::item::ItemId::new(7),
            count: 3,
        }
        .to_damage();
        assert_eq!(damage.what(), "tmap-wrong-input-count");
        assert_eq!(damage.location().ordinal(), Some(7));
        assert_eq!(damage.kind(), DamageKind::MetadataSkipped);
    }

    #[test]
    fn a_brand_finding_is_located_at_the_file_type_box() {
        let damage = BrandFinding::Msf1WithoutIso8.to_damage();
        assert_eq!(damage.fourcc(), Some(*b"ftyp"));
        assert_eq!(damage.what(), "msf1-without-iso8");
    }
}
