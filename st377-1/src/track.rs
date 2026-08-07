//! Timeline Track, Event Track, and Static Track — SMPTE ST 377-1:2019
//! Annex B §B.6/B.12/B.13/B.14 (`docs/st377-1.md`): the three concrete
//! Track kinds used inside Material and Source Packages.
//!
//! All three inherit Generic Track properties (B.6): Track ID, Track
//! Number, Track Name, and a Sequence strong reference.
//!
//! `TimelineTrack` (byte 14/15 = `0x01`/`0x3B`) adds Edit Rate + Origin.
//! `EventTrack`    (byte 14/15 = `0x01`/`0x39`) adds Event Edit Rate +
//!                 Event Origin.
//! `StaticTrack`   (byte 14/15 = `0x01`/`0x3A`) has no additional
//!                 properties.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use broadcast_common::{Parse, Serialize};

use crate::error::{Error, Result};
use crate::local_set::{LocalSet, StructuralSetKind};
use crate::sets::{
    InterchangeObjectFields, LocalSetOwnedItem, collect_dark, finish_owned_set, get_optional_fixed,
    get_optional_raw, get_required_fixed, get_required_raw, owned_set_serialized_len,
    serialize_owned_set,
};
use crate::types::{RATIONAL_LEN, Rational, StrongRef, decode_utf16_be, encode_utf16_be};

// ── Generic Track local tags (B.6) ─────────────────────────────────────

/// Local tag: Track ID (B.6).
pub const TAG_TRACK_ID: u16 = 0x4801;
/// Local tag: Track Name (B.6) — UTF-16 string, optional.
pub const TAG_TRACK_NAME: u16 = 0x4802;
/// Local tag: Sequence (B.6) — StrongRef.
pub const TAG_SEQUENCE: u16 = 0x4803;
/// Local tag: Track Number (B.6).
pub const TAG_TRACK_NUMBER: u16 = 0x4804;

// ── Timeline Track additional tags (B.12) ───────────────────────────────

/// Local tag: Edit Rate (B.12) — Rational (8 bytes).
pub const TAG_EDIT_RATE: u16 = 0x4B01;
/// Local tag: Origin (B.12) — Position / Int64.
pub const TAG_ORIGIN: u16 = 0x4B02;

// ── Event Track additional tags (B.13) ──────────────────────────────────

/// Local tag: Event Edit Rate (B.13) — Rational (8 bytes).
pub const TAG_EVENT_EDIT_RATE: u16 = 0x4901;
/// Local tag: Event Origin (B.13) — Position / Int64, optional (default 0).
pub const TAG_EVENT_ORIGIN: u16 = 0x4902;

// ── Known-tag tables ────────────────────────────────────────────────────

const TIMELINE_KNOWN_TAGS: [u16; 9] = [
    crate::sets::TAG_INSTANCE_UID,
    crate::sets::TAG_GENERATION_UID,
    crate::sets::TAG_OBJECT_CLASS,
    TAG_TRACK_ID,
    TAG_TRACK_NAME,
    TAG_SEQUENCE,
    TAG_TRACK_NUMBER,
    TAG_EDIT_RATE,
    TAG_ORIGIN,
];

const EVENT_KNOWN_TAGS: [u16; 9] = [
    crate::sets::TAG_INSTANCE_UID,
    crate::sets::TAG_GENERATION_UID,
    crate::sets::TAG_OBJECT_CLASS,
    TAG_TRACK_ID,
    TAG_TRACK_NAME,
    TAG_SEQUENCE,
    TAG_TRACK_NUMBER,
    TAG_EVENT_EDIT_RATE,
    TAG_EVENT_ORIGIN,
];

const STATIC_KNOWN_TAGS: [u16; 7] = [
    crate::sets::TAG_INSTANCE_UID,
    crate::sets::TAG_GENERATION_UID,
    crate::sets::TAG_OBJECT_CLASS,
    TAG_TRACK_ID,
    TAG_TRACK_NAME,
    TAG_SEQUENCE,
    TAG_TRACK_NUMBER,
];

// ═══════════════════════════════════════════════════════════════════════
// TimelineTrack
// ═══════════════════════════════════════════════════════════════════════

/// The Timeline Track Set — SMPTE ST 377-1:2019 Annex B §B.12 (byte
/// 14/15 = `0x01`/`0x3B`): a timed track with a fixed edit rate and
/// origin position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineTrack {
    /// Interchange Object (A.1) base properties.
    pub interchange: InterchangeObjectFields,
    /// Track ID (`0x4801`, Req).
    pub track_id: u32,
    /// Track Number (`0x4804`, Req, default 0).
    pub track_number: u32,
    /// Track Name (`0x4802`, Opt) — human-readable name.
    pub track_name: Option<String>,
    /// Sequence (`0x4803`, Req) — strong reference to a Sequence.
    pub sequence: StrongRef,
    /// Edit Rate (`0x4B01`, Req) — the track's time base.
    pub edit_rate: Rational,
    /// Origin (`0x4B02`, Req) — the position of the first edit unit.
    pub origin: i64,
    /// Unrecognized properties preserved for round-trip fidelity.
    pub dark: Vec<(u16, Vec<u8>)>,
}

impl<'a> Parse<'a> for TimelineTrack {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let set = LocalSet::parse(bytes)?;
        if set.kind() != StructuralSetKind::TimelineTrack {
            return Err(Error::KeyPrefixMismatch {
                what: "Timeline Track (Table 17)",
            });
        }
        let items = &set.items;
        let interchange = InterchangeObjectFields::decode(items, "Timeline Track")?;
        let track_id = u32::from_be_bytes(get_required_fixed::<4>(
            items,
            TAG_TRACK_ID,
            "Track ID",
            "Timeline Track",
        )?);
        let track_number = u32::from_be_bytes(get_required_fixed::<4>(
            items,
            TAG_TRACK_NUMBER,
            "Track Number",
            "Timeline Track",
        )?);
        let track_name = get_optional_raw(items, TAG_TRACK_NAME)
            .map(decode_utf16_be)
            .transpose()
            .map_err(|_| Error::InvalidUtf16 {
                tag: TAG_TRACK_NAME,
                name: "Track Name",
            })?;
        let sequence = get_required_fixed::<16>(items, TAG_SEQUENCE, "Sequence", "Timeline Track")?;
        let edit_rate = Rational::parse(get_required_raw(
            items,
            TAG_EDIT_RATE,
            "Edit Rate",
            "Timeline Track",
        )?)?;
        let origin = i64::from_be_bytes(get_required_fixed::<8>(
            items,
            TAG_ORIGIN,
            "Origin",
            "Timeline Track",
        )?);
        let dark = collect_dark(items, &TIMELINE_KNOWN_TAGS);

        Ok(TimelineTrack {
            interchange,
            track_id,
            track_number,
            track_name,
            sequence,
            edit_rate,
            origin,
            dark,
        })
    }
}

impl TimelineTrack {
    fn owned_items(&self) -> Vec<LocalSetOwnedItem> {
        let mut out = Vec::new();
        self.interchange.encode_into(&mut out);
        out.push(LocalSetOwnedItem::fixed(
            TAG_TRACK_ID,
            self.track_id.to_be_bytes(),
        ));
        out.push(LocalSetOwnedItem::fixed(
            TAG_TRACK_NUMBER,
            self.track_number.to_be_bytes(),
        ));
        if let Some(n) = &self.track_name {
            out.push(LocalSetOwnedItem::owned(TAG_TRACK_NAME, encode_utf16_be(n)));
        }
        out.push(LocalSetOwnedItem::fixed(TAG_SEQUENCE, self.sequence));
        {
            let mut buf = [0u8; RATIONAL_LEN];
            self.edit_rate
                .serialize_into(&mut buf)
                .expect("fixed-size buffer");
            out.push(LocalSetOwnedItem::owned(TAG_EDIT_RATE, buf.to_vec()));
        }
        out.push(LocalSetOwnedItem::fixed(
            TAG_ORIGIN,
            self.origin.to_be_bytes(),
        ));
        out
    }
}

impl Serialize for TimelineTrack {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let (key, items) = finish_owned_set(
            StructuralSetKind::TimelineTrack,
            self.owned_items(),
            &self.dark,
        );
        owned_set_serialized_len(key, &items)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let (key, items) = finish_owned_set(
            StructuralSetKind::TimelineTrack,
            self.owned_items(),
            &self.dark,
        );
        serialize_owned_set(key, &items, buf)
    }
}

// ═══════════════════════════════════════════════════════════════════════
// EventTrack
// ═══════════════════════════════════════════════════════════════════════

/// The Event Track (DM) Set — SMPTE ST 377-1:2019 Annex B §B.13 (byte
/// 14/15 = `0x01`/`0x39`): a DM event-driven track with an edit rate
/// and optional origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventTrack {
    /// Interchange Object (A.1) base properties.
    pub interchange: InterchangeObjectFields,
    /// Track ID (`0x4801`, Req).
    pub track_id: u32,
    /// Track Number (`0x4804`, Req, default 0).
    pub track_number: u32,
    /// Track Name (`0x4802`, Opt).
    pub track_name: Option<String>,
    /// Sequence (`0x4803`, Req) — strong reference to a Sequence.
    pub sequence: StrongRef,
    /// Event Edit Rate (`0x4901`, Req).
    pub event_edit_rate: Rational,
    /// Event Origin (`0x4902`, Opt, default 0).
    pub event_origin: Option<i64>,
    /// Unrecognized properties preserved for round-trip fidelity.
    pub dark: Vec<(u16, Vec<u8>)>,
}

impl<'a> Parse<'a> for EventTrack {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let set = LocalSet::parse(bytes)?;
        if set.kind() != StructuralSetKind::EventTrackDm {
            return Err(Error::KeyPrefixMismatch {
                what: "Event Track (Table 17)",
            });
        }
        let items = &set.items;
        let interchange = InterchangeObjectFields::decode(items, "Event Track")?;
        let track_id = u32::from_be_bytes(get_required_fixed::<4>(
            items,
            TAG_TRACK_ID,
            "Track ID",
            "Event Track",
        )?);
        let track_number = u32::from_be_bytes(get_required_fixed::<4>(
            items,
            TAG_TRACK_NUMBER,
            "Track Number",
            "Event Track",
        )?);
        let track_name = get_optional_raw(items, TAG_TRACK_NAME)
            .map(decode_utf16_be)
            .transpose()
            .map_err(|_| Error::InvalidUtf16 {
                tag: TAG_TRACK_NAME,
                name: "Track Name",
            })?;
        let sequence = get_required_fixed::<16>(items, TAG_SEQUENCE, "Sequence", "Event Track")?;
        let event_edit_rate = Rational::parse(get_required_raw(
            items,
            TAG_EVENT_EDIT_RATE,
            "Event Edit Rate",
            "Event Track",
        )?)?;
        let event_origin = get_optional_fixed::<8>(items, TAG_EVENT_ORIGIN, "Event Origin")?
            .map(i64::from_be_bytes);
        let dark = collect_dark(items, &EVENT_KNOWN_TAGS);

        Ok(EventTrack {
            interchange,
            track_id,
            track_number,
            track_name,
            sequence,
            event_edit_rate,
            event_origin,
            dark,
        })
    }
}

impl EventTrack {
    fn owned_items(&self) -> Vec<LocalSetOwnedItem> {
        let mut out = Vec::new();
        self.interchange.encode_into(&mut out);
        out.push(LocalSetOwnedItem::fixed(
            TAG_TRACK_ID,
            self.track_id.to_be_bytes(),
        ));
        out.push(LocalSetOwnedItem::fixed(
            TAG_TRACK_NUMBER,
            self.track_number.to_be_bytes(),
        ));
        if let Some(n) = &self.track_name {
            out.push(LocalSetOwnedItem::owned(TAG_TRACK_NAME, encode_utf16_be(n)));
        }
        out.push(LocalSetOwnedItem::fixed(TAG_SEQUENCE, self.sequence));
        {
            let mut buf = [0u8; RATIONAL_LEN];
            self.event_edit_rate
                .serialize_into(&mut buf)
                .expect("fixed-size buffer");
            out.push(LocalSetOwnedItem::owned(TAG_EVENT_EDIT_RATE, buf.to_vec()));
        }
        if let Some(o) = self.event_origin {
            out.push(LocalSetOwnedItem::fixed(TAG_EVENT_ORIGIN, o.to_be_bytes()));
        }
        out
    }
}

impl Serialize for EventTrack {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let (key, items) = finish_owned_set(
            StructuralSetKind::EventTrackDm,
            self.owned_items(),
            &self.dark,
        );
        owned_set_serialized_len(key, &items)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let (key, items) = finish_owned_set(
            StructuralSetKind::EventTrackDm,
            self.owned_items(),
            &self.dark,
        );
        serialize_owned_set(key, &items, buf)
    }
}

// ═══════════════════════════════════════════════════════════════════════
// StaticTrack
// ═══════════════════════════════════════════════════════════════════════

/// The Static Track (DM) Set — SMPTE ST 377-1:2019 Annex B §B.14 (byte
/// 14/15 = `0x01`/`0x3A`): a DM track with no temporal extent.  Carries
/// only the Generic Track properties (B.6) — no additional fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticTrack {
    /// Interchange Object (A.1) base properties.
    pub interchange: InterchangeObjectFields,
    /// Track ID (`0x4801`, Req).
    pub track_id: u32,
    /// Track Number (`0x4804`, Req, default 0).
    pub track_number: u32,
    /// Track Name (`0x4802`, Opt).
    pub track_name: Option<String>,
    /// Sequence (`0x4803`, Req) — strong reference to a Sequence.
    pub sequence: StrongRef,
    /// Unrecognized properties preserved for round-trip fidelity.
    pub dark: Vec<(u16, Vec<u8>)>,
}

impl<'a> Parse<'a> for StaticTrack {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let set = LocalSet::parse(bytes)?;
        if set.kind() != StructuralSetKind::StaticTrackDm {
            return Err(Error::KeyPrefixMismatch {
                what: "Static Track (Table 17)",
            });
        }
        let items = &set.items;
        let interchange = InterchangeObjectFields::decode(items, "Static Track")?;
        let track_id = u32::from_be_bytes(get_required_fixed::<4>(
            items,
            TAG_TRACK_ID,
            "Track ID",
            "Static Track",
        )?);
        let track_number = u32::from_be_bytes(get_required_fixed::<4>(
            items,
            TAG_TRACK_NUMBER,
            "Track Number",
            "Static Track",
        )?);
        let track_name = get_optional_raw(items, TAG_TRACK_NAME)
            .map(decode_utf16_be)
            .transpose()
            .map_err(|_| Error::InvalidUtf16 {
                tag: TAG_TRACK_NAME,
                name: "Track Name",
            })?;
        let sequence = get_required_fixed::<16>(items, TAG_SEQUENCE, "Sequence", "Static Track")?;
        let dark = collect_dark(items, &STATIC_KNOWN_TAGS);

        Ok(StaticTrack {
            interchange,
            track_id,
            track_number,
            track_name,
            sequence,
            dark,
        })
    }
}

impl StaticTrack {
    fn owned_items(&self) -> Vec<LocalSetOwnedItem> {
        let mut out = Vec::new();
        self.interchange.encode_into(&mut out);
        out.push(LocalSetOwnedItem::fixed(
            TAG_TRACK_ID,
            self.track_id.to_be_bytes(),
        ));
        out.push(LocalSetOwnedItem::fixed(
            TAG_TRACK_NUMBER,
            self.track_number.to_be_bytes(),
        ));
        if let Some(n) = &self.track_name {
            out.push(LocalSetOwnedItem::owned(TAG_TRACK_NAME, encode_utf16_be(n)));
        }
        out.push(LocalSetOwnedItem::fixed(TAG_SEQUENCE, self.sequence));
        out
    }
}

impl Serialize for StaticTrack {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let (key, items) = finish_owned_set(
            StructuralSetKind::StaticTrackDm,
            self.owned_items(),
            &self.dark,
        );
        owned_set_serialized_len(key, &items)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let (key, items) = finish_owned_set(
            StructuralSetKind::StaticTrackDm,
            self.owned_items(),
            &self.dark,
        );
        serialize_owned_set(key, &items, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_timeline_track() -> TimelineTrack {
        TimelineTrack {
            interchange: InterchangeObjectFields {
                instance_uid: [0x01; 16],
                generation_uid: None,
                object_class: None,
            },
            track_id: 1,
            track_number: 0x15010100,
            track_name: Some(String::from("Video")),
            sequence: [0x02; 16],
            edit_rate: Rational {
                numerator: 25,
                denominator: 1,
            },
            origin: 0,
            dark: Vec::new(),
        }
    }

    fn sample_event_track() -> EventTrack {
        EventTrack {
            interchange: InterchangeObjectFields {
                instance_uid: [0x10; 16],
                generation_uid: None,
                object_class: None,
            },
            track_id: 3,
            track_number: 0,
            track_name: None,
            sequence: [0x20; 16],
            event_edit_rate: Rational {
                numerator: 25,
                denominator: 1,
            },
            event_origin: Some(100),
            dark: Vec::new(),
        }
    }

    fn sample_static_track() -> StaticTrack {
        StaticTrack {
            interchange: InterchangeObjectFields {
                instance_uid: [0x30; 16],
                generation_uid: None,
                object_class: None,
            },
            track_id: 4,
            track_number: 0,
            track_name: None,
            sequence: [0x40; 16],
            dark: Vec::new(),
        }
    }

    #[test]
    fn timeline_track_round_trip() {
        let tt = sample_timeline_track();
        let bytes = tt.to_bytes();
        let parsed = TimelineTrack::parse(&bytes).unwrap();
        assert_eq!(parsed, tt);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn timeline_track_no_name_round_trip() {
        let mut tt = sample_timeline_track();
        tt.track_name = None;
        let bytes = tt.to_bytes();
        let parsed = TimelineTrack::parse(&bytes).unwrap();
        assert_eq!(parsed.track_name, None);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn timeline_track_dark_preserved() {
        let mut tt = sample_timeline_track();
        tt.dark = alloc::vec![(0x8001, alloc::vec![0xAB])];
        let bytes = tt.to_bytes();
        let parsed = TimelineTrack::parse(&bytes).unwrap();
        assert_eq!(parsed.dark, tt.dark);
    }

    #[test]
    fn event_track_round_trip() {
        let et = sample_event_track();
        let bytes = et.to_bytes();
        let parsed = EventTrack::parse(&bytes).unwrap();
        assert_eq!(parsed, et);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn event_track_no_origin_round_trip() {
        let mut et = sample_event_track();
        et.event_origin = None;
        let bytes = et.to_bytes();
        let parsed = EventTrack::parse(&bytes).unwrap();
        assert_eq!(parsed.event_origin, None);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn static_track_round_trip() {
        let st = sample_static_track();
        let bytes = st.to_bytes();
        let parsed = StaticTrack::parse(&bytes).unwrap();
        assert_eq!(parsed, st);
        assert_eq!(parsed.to_bytes(), bytes);
    }

    #[test]
    fn wrong_kind_rejected_timeline() {
        let st = sample_static_track();
        let bytes = st.to_bytes();
        assert!(matches!(
            TimelineTrack::parse(&bytes),
            Err(Error::KeyPrefixMismatch { .. })
        ));
    }

    #[test]
    fn wrong_kind_rejected_event() {
        let tt = sample_timeline_track();
        let bytes = tt.to_bytes();
        assert!(matches!(
            EventTrack::parse(&bytes),
            Err(Error::KeyPrefixMismatch { .. })
        ));
    }

    #[test]
    fn wrong_kind_rejected_static() {
        let et = sample_event_track();
        let bytes = et.to_bytes();
        assert!(matches!(
            StaticTrack::parse(&bytes),
            Err(Error::KeyPrefixMismatch { .. })
        ));
    }

    #[test]
    fn mutation_changes_serialized_bytes() {
        let mut tt = sample_timeline_track();
        let before = tt.to_bytes();
        tt.origin = 42;
        let after = tt.to_bytes();
        assert_ne!(before, after);
        assert_eq!(TimelineTrack::parse(&after).unwrap().origin, 42);
    }
}
