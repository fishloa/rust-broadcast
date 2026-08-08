//! Timecode Component — SMPTE ST 377-1:2019 Annex B §B.17
//! (`docs/st377-1.md`): a Structural Component carrying a timecode
//! reference (byte 14/15 = `0x01`/`0x14`).
//!
//! Inherits the Structural Component base properties (B.8): Data
//! Definition and Duration.  Adds Start Timecode, Rounded Timecode
//! Base, and Drop Frame flag.

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

// ── Timecode Component own tags (B.17) ──────────────────────────────────

/// Local tag: Start Timecode (B.17) — Int64.
pub const TAG_START_TIMECODE: u16 = 0x1501;
/// Local tag: Rounded Timecode Base (B.17) — UInt16.
pub const TAG_ROUNDED_TIMECODE_BASE: u16 = 0x1502;
/// Local tag: Drop Frame (B.17) — Boolean (UInt8).
pub const TAG_DROP_FRAME: u16 = 0x1503;

const KNOWN_TAGS: [u16; 8] = [
    crate::sets::TAG_INSTANCE_UID,
    crate::sets::TAG_GENERATION_UID,
    crate::sets::TAG_OBJECT_CLASS,
    TAG_DATA_DEFINITION,
    TAG_DURATION,
    TAG_START_TIMECODE,
    TAG_ROUNDED_TIMECODE_BASE,
    TAG_DROP_FRAME,
];

/// The Timecode Component Set — SMPTE ST 377-1:2019 Annex B §B.17 (byte
/// 14/15 = `0x01`/`0x14`): supplies a timecode reference inside a Track
/// Sequence (typically in a Timecode Track).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimecodeComponent {
    /// Interchange Object (A.1) base properties.
    pub interchange: InterchangeObjectFields,
    /// Data Definition (`0x0201`, Req) — UL identifying the essence kind
    /// (usually Timecode Data Definition).
    pub data_definition: UlBytes,
    /// Duration (`0x0202`, Opt) — edit-unit count.
    pub duration: Option<i64>,
    /// Start Timecode (`0x1501`, Req) — the initial timecode value (in
    /// edit units counted from midnight).
    pub start_timecode: i64,
    /// Rounded Timecode Base (`0x1502`, Req) — the integer frame rate
    /// the timecode runs at (e.g. 25, 30).
    pub rounded_timecode_base: u16,
    /// Drop Frame (`0x1503`, Req) — whether drop-frame counting applies
    /// (NTSC 29.97).
    pub drop_frame: bool,
    /// Unrecognized properties preserved for round-trip fidelity.
    pub dark: Vec<(u16, Vec<u8>)>,
}

impl<'a> Parse<'a> for TimecodeComponent {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let set = LocalSet::parse(bytes)?;
        if set.kind() != StructuralSetKind::TimecodeComponent {
            return Err(Error::KeyPrefixMismatch {
                what: "Timecode Component (Table 17)",
            });
        }
        let items = &set.items;
        let interchange = InterchangeObjectFields::decode(items, "Timecode Component")?;
        let data_definition = get_required_fixed::<16>(
            items,
            TAG_DATA_DEFINITION,
            "Data Definition",
            "Timecode Component",
        )?;
        let duration =
            get_optional_fixed::<8>(items, TAG_DURATION, "Duration")?.map(i64::from_be_bytes);
        let start_timecode = i64::from_be_bytes(get_required_fixed::<8>(
            items,
            TAG_START_TIMECODE,
            "Start Timecode",
            "Timecode Component",
        )?);
        let rounded_timecode_base = u16::from_be_bytes(get_required_fixed::<2>(
            items,
            TAG_ROUNDED_TIMECODE_BASE,
            "Rounded Timecode Base",
            "Timecode Component",
        )?);
        let drop_frame =
            get_required_fixed::<1>(items, TAG_DROP_FRAME, "Drop Frame", "Timecode Component")?[0]
                != 0;
        let dark = collect_dark(items, &KNOWN_TAGS);

        Ok(TimecodeComponent {
            interchange,
            data_definition,
            duration,
            start_timecode,
            rounded_timecode_base,
            drop_frame,
            dark,
        })
    }
}

impl TimecodeComponent {
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
            TAG_START_TIMECODE,
            self.start_timecode.to_be_bytes(),
        ));
        out.push(LocalSetOwnedItem::fixed(
            TAG_ROUNDED_TIMECODE_BASE,
            self.rounded_timecode_base.to_be_bytes(),
        ));
        out.push(LocalSetOwnedItem::fixed(
            TAG_DROP_FRAME,
            [u8::from(self.drop_frame)],
        ));
        out
    }
}

impl Serialize for TimecodeComponent {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let (key, items) = finish_owned_set(
            StructuralSetKind::TimecodeComponent,
            self.owned_items(),
            &self.dark,
        );
        owned_set_serialized_len(key, &items)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let (key, items) = finish_owned_set(
            StructuralSetKind::TimecodeComponent,
            self.owned_items(),
            &self.dark,
        );
        serialize_owned_set(key, &items, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SMPTE-RP 224 Timecode data definition UL (test placeholder).
    const TIMECODE_DD: UlBytes = [
        0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x01, 0x03, 0x02, 0x01, 0x01, 0x00, 0x00,
        0x00,
    ];

    fn sample() -> TimecodeComponent {
        TimecodeComponent {
            interchange: InterchangeObjectFields {
                instance_uid: [0x01; 16],
                generation_uid: None,
                object_class: None,
            },
            data_definition: TIMECODE_DD,
            duration: Some(250),
            start_timecode: 90000,
            rounded_timecode_base: 25,
            drop_frame: false,
            dark: Vec::new(),
        }
    }

    #[test]
    fn round_trip() {
        let tc = sample();
        let bytes = tc.to_bytes();
        let parsed = TimecodeComponent::parse(&bytes).unwrap();
        assert_eq!(parsed, tc);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn drop_frame_true_round_trips() {
        let mut tc = sample();
        tc.drop_frame = true;
        tc.rounded_timecode_base = 30;
        let bytes = tc.to_bytes();
        let parsed = TimecodeComponent::parse(&bytes).unwrap();
        assert!(parsed.drop_frame);
        assert_eq!(parsed.rounded_timecode_base, 30);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn no_duration_round_trip() {
        let mut tc = sample();
        tc.duration = None;
        let bytes = tc.to_bytes();
        let parsed = TimecodeComponent::parse(&bytes).unwrap();
        assert_eq!(parsed.duration, None);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn dark_preserved() {
        let mut tc = sample();
        tc.dark = alloc::vec![(0x9004, alloc::vec![0xCA, 0xFE])];
        let bytes = tc.to_bytes();
        let parsed = TimecodeComponent::parse(&bytes).unwrap();
        assert_eq!(parsed.dark, tc.dark);
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
            TimecodeComponent::parse(&bytes),
            Err(Error::KeyPrefixMismatch { .. })
        ));
    }

    #[test]
    fn mutation_changes_serialized_bytes() {
        let mut tc = sample();
        let before = tc.to_bytes();
        tc.start_timecode = 1800000;
        let after = tc.to_bytes();
        assert_ne!(before, after);
        assert_eq!(
            TimecodeComponent::parse(&after).unwrap().start_timecode,
            1800000
        );
    }
}
