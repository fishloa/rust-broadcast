//! Source Clip — SMPTE ST 377-1:2019 Annex B §B.10 (`docs/st377-1.md`):
//! a Structural Component that references a contiguous range of essence
//! from a Source Package's Track (byte 14/15 = `0x01`/`0x11`).
//!
//! Inherits the Structural Component base properties (B.8): Data
//! Definition and Duration.  Adds Start Position, Source Package ID
//! (UMID), and Source Track ID.

extern crate alloc;

use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};
use crate::local_set::{LocalSet, StructuralSetKind};
use crate::sets::{
    InterchangeObjectFields, LocalSetOwnedItem, collect_dark, finish_owned_set, get_optional_fixed,
    get_required_fixed, owned_set_serialized_len, serialize_owned_set,
};
use crate::types::{PackageId, UlBytes};

// ── Structural Component base tags (B.8) — shared with sequence.rs ──────

/// Local tag: Data Definition (B.8).
pub const TAG_DATA_DEFINITION: u16 = 0x0201;
/// Local tag: Duration (B.8).
pub const TAG_DURATION: u16 = 0x0202;

// ── Source Clip own tags (B.10) ─────────────────────────────────────────

/// Local tag: Source Package ID (B.10) — 32-byte UMID (PackageRef).
pub const TAG_SOURCE_PACKAGE_ID: u16 = 0x1101;
/// Local tag: Source Track ID (B.10) — UInt32.
pub const TAG_SOURCE_TRACK_ID: u16 = 0x1102;
/// Local tag: Start Position (B.10) — Position / Int64.
pub const TAG_START_POSITION: u16 = 0x1201;

const KNOWN_TAGS: [u16; 8] = [
    crate::sets::TAG_INSTANCE_UID,
    crate::sets::TAG_GENERATION_UID,
    crate::sets::TAG_OBJECT_CLASS,
    TAG_DATA_DEFINITION,
    TAG_DURATION,
    TAG_SOURCE_PACKAGE_ID,
    TAG_SOURCE_TRACK_ID,
    TAG_START_POSITION,
];

/// The Source Clip Set — SMPTE ST 377-1:2019 Annex B §B.10 (byte 14/15
/// = `0x01`/`0x11`): references a contiguous span of essence from a
/// Source Package Track, identified by its UMID, Track ID, and a start
/// position within that track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceClip {
    /// Interchange Object (A.1) base properties.
    pub interchange: InterchangeObjectFields,
    /// Data Definition (`0x0201`, Req) — UL identifying the essence kind.
    pub data_definition: UlBytes,
    /// Duration (`0x0202`, Opt) — edit-unit count for this clip.
    pub duration: Option<i64>,
    /// Start Position (`0x1201`, Req) — the position in the referenced
    /// Track where this clip starts.
    pub start_position: i64,
    /// Source Package ID (`0x1101`, Req) — the UMID of the referenced
    /// Source Package (all-zero terminates the chain).
    pub source_package_id: PackageId,
    /// Source Track ID (`0x1102`, Req) — the Track ID within the
    /// referenced Source Package.
    pub source_track_id: u32,
    /// Unrecognized properties preserved for round-trip fidelity.
    pub dark: Vec<(u16, Vec<u8>)>,
}

impl<'a> Parse<'a> for SourceClip {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let set = LocalSet::parse(bytes)?;
        if set.kind() != StructuralSetKind::SourceClip {
            return Err(Error::KeyPrefixMismatch {
                what: "Source Clip (Table 17)",
            });
        }
        let items = &set.items;
        let interchange = InterchangeObjectFields::decode(items, "Source Clip")?;
        let data_definition =
            get_required_fixed::<16>(items, TAG_DATA_DEFINITION, "Data Definition", "Source Clip")?;
        let duration =
            get_optional_fixed::<8>(items, TAG_DURATION, "Duration")?.map(i64::from_be_bytes);
        let start_position = i64::from_be_bytes(get_required_fixed::<8>(
            items,
            TAG_START_POSITION,
            "Start Position",
            "Source Clip",
        )?);
        let source_package_id = PackageId(get_required_fixed::<32>(
            items,
            TAG_SOURCE_PACKAGE_ID,
            "Source Package ID",
            "Source Clip",
        )?);
        let source_track_id = u32::from_be_bytes(get_required_fixed::<4>(
            items,
            TAG_SOURCE_TRACK_ID,
            "Source Track ID",
            "Source Clip",
        )?);
        let dark = collect_dark(items, &KNOWN_TAGS);

        Ok(SourceClip {
            interchange,
            data_definition,
            duration,
            start_position,
            source_package_id,
            source_track_id,
            dark,
        })
    }
}

impl SourceClip {
    fn owned_items(&self) -> Vec<LocalSetOwnedItem> {
        let mut out = Vec::new();
        self.interchange.encode_into(&mut out);
        out.push(LocalSetOwnedItem::fixed(
            TAG_DATA_DEFINITION,
            self.data_definition,
        ));
        if let Some(d) = self.duration {
            out.push(LocalSetOwnedItem::fixed(TAG_DURATION, d.to_be_bytes()));
        }
        out.push(LocalSetOwnedItem::fixed(
            TAG_START_POSITION,
            self.start_position.to_be_bytes(),
        ));
        out.push(LocalSetOwnedItem::fixed(
            TAG_SOURCE_PACKAGE_ID,
            self.source_package_id.0,
        ));
        out.push(LocalSetOwnedItem::fixed(
            TAG_SOURCE_TRACK_ID,
            self.source_track_id.to_be_bytes(),
        ));
        out
    }
}

impl Serialize for SourceClip {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let (key, items) = finish_owned_set(
            StructuralSetKind::SourceClip,
            self.owned_items(),
            &self.dark,
        );
        owned_set_serialized_len(key, &items)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let (key, items) = finish_owned_set(
            StructuralSetKind::SourceClip,
            self.owned_items(),
            &self.dark,
        );
        serialize_owned_set(key, &items, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SMPTE-RP 224 Picture essence data definition UL (test placeholder).
    const PICTURE_DD: UlBytes = [
        0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x01, 0x03, 0x02, 0x02, 0x01, 0x00, 0x00,
        0x00,
    ];

    fn sample() -> SourceClip {
        SourceClip {
            interchange: InterchangeObjectFields {
                instance_uid: [0x01; 16],
                generation_uid: None,
                object_class: None,
            },
            data_definition: PICTURE_DD,
            duration: Some(250),
            start_position: 0,
            source_package_id: PackageId([0x02; 32]),
            source_track_id: 1,
            dark: Vec::new(),
        }
    }

    #[test]
    fn round_trip() {
        let sc = sample();
        let bytes = sc.to_bytes();
        let parsed = SourceClip::parse(&bytes).unwrap();
        assert_eq!(parsed, sc);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn no_duration_round_trip() {
        let mut sc = sample();
        sc.duration = None;
        let bytes = sc.to_bytes();
        let parsed = SourceClip::parse(&bytes).unwrap();
        assert_eq!(parsed.duration, None);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn null_source_terminates_chain() {
        let mut sc = sample();
        sc.source_package_id = PackageId::NULL;
        sc.source_track_id = 0;
        let bytes = sc.to_bytes();
        let parsed = SourceClip::parse(&bytes).unwrap();
        assert!(parsed.source_package_id.is_null());
    }

    #[test]
    fn dark_preserved() {
        let mut sc = sample();
        sc.dark = alloc::vec![(0x9003, alloc::vec![0xDE, 0xAD])];
        let bytes = sc.to_bytes();
        let parsed = SourceClip::parse(&bytes).unwrap();
        assert_eq!(parsed.dark, sc.dark);
    }

    #[test]
    fn wrong_kind_rejected() {
        let key = LocalSet::build_key(
            StructuralSetKind::Sequence,
            crate::local_set::ItemLengthMode::TwoByte,
        );
        let set = LocalSet {
            key,
            items: Vec::new(),
        };
        let bytes = set.to_bytes();
        assert!(matches!(
            SourceClip::parse(&bytes),
            Err(Error::KeyPrefixMismatch { .. })
        ));
    }

    #[test]
    fn mutation_changes_serialized_bytes() {
        let mut sc = sample();
        let before = sc.to_bytes();
        sc.start_position = 100;
        let after = sc.to_bytes();
        assert_ne!(before, after);
        assert_eq!(SourceClip::parse(&after).unwrap().start_position, 100);
    }
}
