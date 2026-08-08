//! Filler — SMPTE ST 377-1:2019 Annex B §B.11 (`docs/st377-1.md`): a
//! Structural Component that occupies a gap in a Sequence's timeline
//! (byte 14/15 = `0x01`/`0x09`).
//!
//! Inherits the Structural Component base properties (B.8): Data
//! Definition and Duration.  No additional properties.

extern crate alloc;

use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};
use crate::local_set::{LocalSet, StructuralSetKind};
use crate::sets::{
    InterchangeObjectFields, LocalSetOwnedItem, collect_dark, finish_owned_set, get_optional_fixed,
    get_required_fixed, owned_set_serialized_len, serialize_owned_set,
};
use crate::types::UlBytes;

// ── Structural Component base tags (B.8) ────────────────────────────────

/// Local tag: Data Definition (B.8).
pub const TAG_DATA_DEFINITION: u16 = 0x0201;
/// Local tag: Duration (B.8).
pub const TAG_DURATION: u16 = 0x0202;

const KNOWN_TAGS: [u16; 5] = [
    crate::sets::TAG_INSTANCE_UID,
    crate::sets::TAG_GENERATION_UID,
    crate::sets::TAG_OBJECT_CLASS,
    TAG_DATA_DEFINITION,
    TAG_DURATION,
];

/// The Filler Set — SMPTE ST 377-1:2019 Annex B §B.11 (byte 14/15 =
/// `0x01`/`0x09`): a gap placeholder inside a Sequence, consuming a
/// specified duration of the track's timeline without referencing any
/// source essence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FillerComponent {
    /// Interchange Object (A.1) base properties.
    pub interchange: InterchangeObjectFields,
    /// Data Definition (`0x0201`, Req) — UL identifying the essence kind.
    pub data_definition: UlBytes,
    /// Duration (`0x0202`, Opt) — edit-unit count for this gap.
    pub duration: Option<i64>,
    /// Unrecognized properties preserved for round-trip fidelity.
    pub dark: Vec<(u16, Vec<u8>)>,
}

impl<'a> Parse<'a> for FillerComponent {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let set = LocalSet::parse(bytes)?;
        if set.kind() != StructuralSetKind::Filler {
            return Err(Error::KeyPrefixMismatch {
                what: "Filler (Table 17)",
            });
        }
        let items = &set.items;
        let interchange = InterchangeObjectFields::decode(items, "Filler")?;
        let data_definition =
            get_required_fixed::<16>(items, TAG_DATA_DEFINITION, "Data Definition", "Filler")?;
        let duration =
            get_optional_fixed::<8>(items, TAG_DURATION, "Duration")?.map(i64::from_be_bytes);
        let dark = collect_dark(items, &KNOWN_TAGS);

        Ok(FillerComponent {
            interchange,
            data_definition,
            duration,
            dark,
        })
    }
}

impl FillerComponent {
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
        out
    }
}

impl Serialize for FillerComponent {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let (key, items) =
            finish_owned_set(StructuralSetKind::Filler, self.owned_items(), &self.dark);
        owned_set_serialized_len(key, &items)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let (key, items) =
            finish_owned_set(StructuralSetKind::Filler, self.owned_items(), &self.dark);
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

    fn sample() -> FillerComponent {
        FillerComponent {
            interchange: InterchangeObjectFields {
                instance_uid: [0x01; 16],
                generation_uid: None,
                object_class: None,
            },
            data_definition: PICTURE_DD,
            duration: Some(50),
            dark: Vec::new(),
        }
    }

    #[test]
    fn round_trip() {
        let filler = sample();
        let bytes = filler.to_bytes();
        let parsed = FillerComponent::parse(&bytes).unwrap();
        assert_eq!(parsed, filler);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn no_duration_round_trip() {
        let mut filler = sample();
        filler.duration = None;
        let bytes = filler.to_bytes();
        let parsed = FillerComponent::parse(&bytes).unwrap();
        assert_eq!(parsed.duration, None);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn dark_preserved() {
        let mut filler = sample();
        filler.dark = alloc::vec![(0x9005, alloc::vec![0x42])];
        let bytes = filler.to_bytes();
        let parsed = FillerComponent::parse(&bytes).unwrap();
        assert_eq!(parsed.dark, filler.dark);
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
            FillerComponent::parse(&bytes),
            Err(Error::KeyPrefixMismatch { .. })
        ));
    }

    #[test]
    fn mutation_changes_serialized_bytes() {
        let mut filler = sample();
        let before = filler.to_bytes();
        filler.duration = Some(100);
        let after = filler.to_bytes();
        assert_ne!(before, after);
        assert_eq!(FillerComponent::parse(&after).unwrap().duration, Some(100));
    }
}
