//! Sequence — SMPTE ST 377-1:2019 Annex B §B.8/B.9 (`docs/st377-1.md`):
//! an ordered collection of Structural Components forming a Track's
//! content (byte 14/15 = `0x01`/`0x0F`).
//!
//! Inherits the Structural Component base properties (B.8): Data
//! Definition and (optional) Duration.  Adds an ordered Array of strong
//! references to its child Structural Components.

extern crate alloc;

use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};
use crate::local_set::{LocalSet, StructuralSetKind};
use crate::sets::{
    InterchangeObjectFields, LocalSetOwnedItem, collect_dark, finish_owned_set, get_optional_fixed,
    get_required_fixed, get_required_raw, owned_set_serialized_len, serialize_owned_set,
};
use crate::types::{StrongRef, UlBytes, parse_uid_batch, serialize_uid_batch};

// ── Structural Component base tags (B.8) ────────────────────────────────

/// Local tag: Data Definition (B.8) — UL identifying the kind of data
/// (Picture, Sound, Data Definition, etc.).
pub const TAG_DATA_DEFINITION: u16 = 0x0201;
/// Local tag: Duration (B.8) — Int64, optional (omitted for Static
/// Tracks).
pub const TAG_DURATION: u16 = 0x0202;

// ── Sequence own tag (B.9) ──────────────────────────────────────────────

/// Local tag: Structural Components (B.9) — Array of StrongRef to the
/// child components.
pub const TAG_STRUCTURAL_COMPONENTS: u16 = 0x1001;

const KNOWN_TAGS: [u16; 6] = [
    crate::sets::TAG_INSTANCE_UID,
    crate::sets::TAG_GENERATION_UID,
    crate::sets::TAG_OBJECT_CLASS,
    TAG_DATA_DEFINITION,
    TAG_DURATION,
    TAG_STRUCTURAL_COMPONENTS,
];

/// The Sequence Set — SMPTE ST 377-1:2019 Annex B §B.9 (byte 14/15 =
/// `0x01`/`0x0F`): the single child of every Track, holding an ordered
/// list of Structural Components (Source Clips, Timecode Components,
/// Fillers, etc.) that compose the track's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sequence {
    /// Interchange Object (A.1) base properties.
    pub interchange: InterchangeObjectFields,
    /// Data Definition (`0x0201`, Req) — UL identifying the essence kind.
    pub data_definition: UlBytes,
    /// Duration (`0x0202`, Opt) — total duration in edit units; omitted
    /// (None) on Static Tracks.
    pub duration: Option<i64>,
    /// Structural Components (`0x1001`, Req) — ordered strong references
    /// to child components.
    pub structural_components: Vec<StrongRef>,
    /// Unrecognized properties preserved for round-trip fidelity.
    pub dark: Vec<(u16, Vec<u8>)>,
}

impl<'a> Parse<'a> for Sequence {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let set = LocalSet::parse(bytes)?;
        if set.kind() != StructuralSetKind::Sequence {
            return Err(Error::KeyPrefixMismatch {
                what: "Sequence (Table 17)",
            });
        }
        let items = &set.items;
        let interchange = InterchangeObjectFields::decode(items, "Sequence")?;
        let data_definition =
            get_required_fixed::<16>(items, TAG_DATA_DEFINITION, "Data Definition", "Sequence")?;
        let duration =
            get_optional_fixed::<8>(items, TAG_DURATION, "Duration")?.map(i64::from_be_bytes);
        let structural_components = parse_uid_batch(get_required_raw(
            items,
            TAG_STRUCTURAL_COMPONENTS,
            "Structural Components",
            "Sequence",
        )?)?;
        let dark = collect_dark(items, &KNOWN_TAGS);

        Ok(Sequence {
            interchange,
            data_definition,
            duration,
            structural_components,
            dark,
        })
    }
}

impl Sequence {
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
        out.push(LocalSetOwnedItem::owned(
            TAG_STRUCTURAL_COMPONENTS,
            serialize_uid_batch(&self.structural_components),
        ));
        out
    }
}

impl Serialize for Sequence {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let (key, items) =
            finish_owned_set(StructuralSetKind::Sequence, self.owned_items(), &self.dark);
        owned_set_serialized_len(key, &items)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let (key, items) =
            finish_owned_set(StructuralSetKind::Sequence, self.owned_items(), &self.dark);
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

    fn sample() -> Sequence {
        Sequence {
            interchange: InterchangeObjectFields {
                instance_uid: [0x01; 16],
                generation_uid: None,
                object_class: None,
            },
            data_definition: PICTURE_DD,
            duration: Some(250),
            structural_components: alloc::vec![[0x02; 16], [0x03; 16]],
            dark: Vec::new(),
        }
    }

    #[test]
    fn round_trip() {
        let seq = sample();
        let bytes = seq.to_bytes();
        let parsed = Sequence::parse(&bytes).unwrap();
        assert_eq!(parsed, seq);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn no_duration_round_trip() {
        let mut seq = sample();
        seq.duration = None;
        let bytes = seq.to_bytes();
        let parsed = Sequence::parse(&bytes).unwrap();
        assert_eq!(parsed.duration, None);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn dark_preserved() {
        let mut seq = sample();
        seq.dark = alloc::vec![(0x9002, alloc::vec![0xFF])];
        let bytes = seq.to_bytes();
        let parsed = Sequence::parse(&bytes).unwrap();
        assert_eq!(parsed.dark, seq.dark);
    }

    #[test]
    fn wrong_kind_rejected() {
        let key = LocalSet::build_key(
            StructuralSetKind::SourceClip,
            crate::local_set::ItemLengthMode::TwoByte,
        );
        let set = LocalSet {
            key,
            items: Vec::new(),
        };
        let bytes = set.to_bytes();
        assert!(matches!(
            Sequence::parse(&bytes),
            Err(Error::KeyPrefixMismatch { .. })
        ));
    }

    #[test]
    fn mutation_changes_serialized_bytes() {
        let mut seq = sample();
        let before = seq.to_bytes();
        seq.structural_components.push([0x04; 16]);
        let after = seq.to_bytes();
        assert_ne!(before, after);
        assert_eq!(
            Sequence::parse(&after).unwrap().structural_components.len(),
            3
        );
    }
}
