//! Every typed sample entry kind, parsed from a real fixture and re-serialized
//! after each fixed field is flipped to a non-default value.
//!
//! The crate's rule is that a parse -> serialize trip is byte-identical for
//! anything it parses; an entry it cannot represent must be a typed error. A
//! field that is normalised or re-derived on write instead of carried fails
//! here.

use broadcast_core::Serialize;
use isobmff::box_types::{BOX_HEADER_MIN_SIZE, BoxIter};
use isobmff::init_segment::sample_entry_variant;
use isobmff::meta_track::{
    TextMetaDataSampleEntry, UriMetaDataSampleEntry, XmlMetaDataSampleEntry,
};
use test_fixtures as fixtures;

/// SampleEntry fixed fields: `reserved[6]` + `data_reference_index`.
const SAMPLE_ENTRY_FIXED_LEN: usize = 8;
/// VisualSampleEntry fixed fields after the box header (§12.1.3).
const VISUAL_FIXED_LEN: usize = 78;
/// AudioSampleEntry version 0 fixed fields after the box header (§12.2.3).
const AUDIO_FIXED_LEN: usize = 28;
/// QuickTime timecode sample entry fixed fields through `number_of_frames`.
const TIMECODE_FIXED_LEN: usize = 25;
/// A byte value unlikely to equal a field's default.
const NON_DEFAULT: u8 = 0xA5;

/// One fixture entry with the fixed-field length its kind carries.
struct Entry {
    label: &'static str,
    bytes: Vec<u8>,
    fixed_len: usize,
}

fn walk(data: &[u8], out: &mut Vec<Vec<u8>>, depth: u32) {
    if depth > SAMPLE_ENTRY_FIXED_LEN as u32 {
        return;
    }
    for item in BoxIter::new(data) {
        let Ok(item) = item else {
            return;
        };
        match &item.header.box_type.0 {
            b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" => walk(item.body, out, depth + 1),
            b"stsd" => {
                let Some(count) = item
                    .body
                    .get(4..8)
                    .and_then(|b| <[u8; 4]>::try_from(b).ok())
                    .map(u32::from_be_bytes)
                else {
                    continue;
                };
                for entry in BoxIter::clamped(item.body.get(8..).unwrap_or_default())
                    .take(usize::try_from(count).unwrap_or(usize::MAX))
                {
                    let Ok(entry) = entry else {
                        break;
                    };
                    out.push(entry.bytes.to_vec());
                }
            }
            _ => {}
        }
    }
}

fn sample_entries(data: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    walk(data, &mut out, 0);
    out
}

fn fixed_len(fourcc: &[u8]) -> usize {
    match fourcc {
        b"avc1" | b"avc2" | b"avc3" | b"avc4" | b"avcp" | b"hvc1" | b"hev1" | b"dvh1" | b"dvhe"
        | b"vvc1" | b"vvi1" | b"mp4v" | b"av01" | b"vp09" => VISUAL_FIXED_LEN,
        b"mp4a" | b"enca" | b"ac-3" | b"ec-3" | b"ac-4" | b"Opus" | b"fLaC" | b"mha1" | b"mha2"
        | b"mhm1" | b"mhm2" | b"dtsc" | b"dtsh" | b"dtsl" | b"dtse" | b"lpcm" | b"in24"
        | b"in32" | b"sowt" | b"twos" | b"raw " | b"fl32" | b"fl64" | b"ima4" | b"MAC3 "
        | b"MAC6 " | b"ulaw" | b"alaw" | b"NONE" => AUDIO_FIXED_LEN,
        b"tmcd" => TIMECODE_FIXED_LEN,
        _ => SAMPLE_ENTRY_FIXED_LEN,
    }
}

fn entries(data: &[u8], label: &'static str) -> Vec<Entry> {
    sample_entries(data)
        .into_iter()
        .filter(|bytes| bytes.len() > BOX_HEADER_MIN_SIZE + fixed_len(&bytes[4..8]))
        .map(|bytes| Entry {
            label,
            fixed_len: fixed_len(&bytes[4..8]),
            bytes,
        })
        .collect()
}

fn audit(entry: &Entry) -> usize {
    let mut checked = 0usize;
    for offset in BOX_HEADER_MIN_SIZE..BOX_HEADER_MIN_SIZE + entry.fixed_len {
        let Some(original) = entry.bytes.get(offset).copied() else {
            break;
        };
        let mut bytes = entry.bytes.clone();
        bytes[offset] = original ^ NON_DEFAULT;
        if let Ok(variant) = sample_entry_variant(&bytes) {
            let out = variant.try_to_bytes().unwrap_or_else(|err| {
                panic!(
                    "{} entry offset {offset}: serialize failed after parse: {err}",
                    entry.label
                )
            });
            assert_eq!(
                out, bytes,
                "{} entry offset {offset}: fixed field not carried",
                entry.label
            );
        }
        checked += 1;
    }
    checked
}

#[test]
fn committed_visual_audio_and_subtitle_entries_are_byte_exact() {
    let mut all = Vec::new();
    for fixture in [
        "mp4/frag/av1.frag.mp4",
        "mp4/frag/hevc-main.frag.mp4",
        "mp4/frag/vvc.frag.mp4",
        "mp4/cmaf/av-frag.mp4",
        "mp4/flac.mp4",
        "mp4/opus.mp4",
        "mp4/stpp.mp4",
        "mp4/cmaf-av-subtitle/av-subtitle-frag.mp4",
    ] {
        let data = fixtures::read(fixture);
        all.extend(entries(&data, fixture));
    }
    let mut checked = 0usize;
    for entry in &all {
        checked += audit(entry);
    }
    assert!(checked > 0, "at least one fixed byte was exercised");
}

#[test]
fn private_quicktime_and_metadata_entries_are_byte_exact() {
    let who = "sample entry audit";
    let mut all = Vec::new();
    for fixture in [
        "mp4/quicktime/lavf-pcm24.mov",
        "mp4/quicktime/iphone-x-live-photo.mov",
        "mp4/quicktime/lavf-aac-tmcd.mov",
    ] {
        let Some(data) = fixtures::require_bytes(fixture, who) else {
            return;
        };
        all.extend(entries(&data, fixture));
    }
    for entry in &all {
        audit(entry);
    }
}

#[test]
fn metadata_entries_without_a_fixture_carry_reserved() {
    let built = [
        XmlMetaDataSampleEntry::new("", "urn:example")
            .try_to_bytes()
            .expect("metx serializes"),
        TextMetaDataSampleEntry::new("", "text/plain")
            .try_to_bytes()
            .expect("mett serializes"),
        UriMetaDataSampleEntry::new("urn:example")
            .try_to_bytes()
            .expect("urim serializes"),
    ];
    for bytes in built {
        for offset in BOX_HEADER_MIN_SIZE..BOX_HEADER_MIN_SIZE + SAMPLE_ENTRY_FIXED_LEN {
            let mut mutated = bytes.clone();
            mutated[offset] ^= NON_DEFAULT;
            if let Ok(variant) = sample_entry_variant(&mutated) {
                let out = variant.try_to_bytes().expect("metadata entry serializes");
                assert_eq!(out, mutated, "metadata entry offset {offset} not carried");
            }
        }
    }
}
