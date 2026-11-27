//! Pin the per-sample output through `isobmff`'s sample-table cursor.
//!
//! The digest table was generated from transmux's pre-move layout and must
//! stay identical now that the cursor produces the same sequence.

use std::path::{Path, PathBuf};

use broadcast_core::Parse;
use isobmff::Limits;
use isobmff::TrackId;
use isobmff::init_segment::{MOOV, MovieBox, SampleTableBox, StblChild, TrackBox};
use isobmff::sample_table::TrackCursor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Row {
    offset: u64,
    size: u64,
    decode_time: u64,
    composition_offset: i64,
    duration: u32,
    sync: bool,
}

fn fixture_roots() -> Vec<(&'static str, PathBuf)> {
    let mut roots = vec![("public", test_fixtures::root().join("mp4"))];
    if let Some(private) = test_fixtures::private("mp4") {
        roots.push(("private", private));
    }
    roots
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e, "mp4" | "m4a" | "mov"))
        {
            out.push(path);
        }
    }
}

fn fixtures() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    for (tier, root) in fixture_roots() {
        let mut files = Vec::new();
        collect_files(&root, &mut files);
        files.sort();
        for file in files {
            let Ok(rel) = file.strip_prefix(&root) else {
                continue;
            };
            out.push((format!("{tier}/{}", rel.display()), file));
        }
    }
    out
}

fn find<'a, T>(
    stbl: &'a SampleTableBox,
    get: impl Fn(&'a StblChild) -> Option<&'a T>,
) -> Option<&'a T> {
    stbl.children.iter().find_map(get)
}

/// Same inclusion gate the transmux digest test uses, then the cursor's rows.
fn track_rows(file: &[u8], trak: &TrackBox) -> Option<Vec<Row>> {
    let stbl = trak.mdia.as_ref()?.minf.as_ref()?.stbl.as_ref()?;
    let stsz = find(stbl, |c| match c {
        StblChild::Stsz(b) => Some(b),
        _ => None,
    })?;
    if stsz.sample_count == 0 {
        return None;
    }
    if stbl
        .children
        .iter()
        .any(|c| matches!(c, StblChild::Stz2(_)))
    {
        return None;
    }
    let track = TrackId::new(trak.tkhd.track_id.value());
    let mut cursor =
        TrackCursor::new(stbl, track, &Limits::default(), Some(file.len() as u64)).ok()?;
    let mut rows = Vec::new();
    for item in &mut cursor {
        let sample = item.ok()?;
        rows.push(Row {
            offset: sample.offset,
            size: u64::from(sample.size),
            decode_time: sample.decode_time,
            composition_offset: i64::from(sample.composition_offset),
            duration: sample.duration,
            sync: sample.sync,
        });
    }
    Some(rows)
}

fn file_digest(file: &[u8]) -> Option<(u32, u64)> {
    let moov_bytes = isobmff::box_types::find_box(file, &MOOV)?;
    let moov = MovieBox::parse(moov_bytes).ok()?;
    let mut hash = FNV_OFFSET;
    let mut count = 0u32;
    let mut included = 0usize;
    for trak in &moov.tracks {
        let Some(rows) = track_rows(file, trak) else {
            continue;
        };
        included += 1;
        hash = fnv_u32(hash, trak.tkhd.track_id.value());
        for row in &rows {
            hash = fnv_u64(hash, row.offset);
            hash = fnv_u64(hash, row.size);
            hash = fnv_u64(hash, row.decode_time);
            hash = fnv_u64(hash, row.composition_offset as u64);
            hash = fnv_u32(hash, row.duration);
            hash = fnv_u8(hash, u8::from(row.sync));
            count = count.saturating_add(1);
        }
    }
    (included > 0).then_some((count, hash))
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv_u8(mut hash: u64, byte: u8) -> u64 {
    hash ^= u64::from(byte);
    hash.wrapping_mul(FNV_PRIME)
}

fn fnv_u32(hash: u64, value: u32) -> u64 {
    value.to_le_bytes().iter().fold(hash, |h, &b| fnv_u8(h, b))
}

fn fnv_u64(hash: u64, value: u64) -> u64 {
    value.to_le_bytes().iter().fold(hash, |h, &b| fnv_u8(h, b))
}

#[test]
fn cursor_matches_the_pre_move_digest_table() {
    let mut generated = Vec::new();
    for (key, path) in fixtures() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        if let Some((count, digest)) = file_digest(&bytes) {
            generated.push((key, count, digest));
        }
    }
    assert!(
        generated.len() >= 20,
        "only {} fixtures with a non-empty stbl were exercised; 20 are required",
        generated.len()
    );
    let expected: std::collections::BTreeMap<&str, (u32, u64)> = DIGESTS
        .iter()
        .map(|&(name, count, digest)| (name, (count, digest)))
        .collect();
    for (key, count, digest) in &generated {
        let Some(&(want_count, want_digest)) = expected.get(key.as_str()) else {
            panic!("fixture {key} is missing from DIGESTS");
        };
        assert_eq!(
            (*count, *digest),
            (want_count, want_digest),
            "fixture {key}: cursor output differs from the pre-move layout"
        );
    }
    assert_eq!(generated.len(), expected.len());
}

const DIGESTS: &[(&str, u32, u64)] = &[
    ("public/boxes/cprt.mp4", 138, 0xc392f59faac11677),
    ("public/boxes/hint-hmhd.mp4", 207, 0xbfaf49d17d4758c1),
    ("public/aac-sgpd.mp4", 131, 0x9f808b3f4836a623),
    ("public/av1.mp4", 12, 0xdabde3d714765d4d),
    ("public/boxes/chnl-ipcm.mp4", 14400, 0x2b1ad55bb77541e5),
    ("public/boxes/loudness.m4a", 89, 0xb1187d0cc35ef16a),
    ("public/boxes/track-description.mp4", 138, 0xbed24d7368c988c),
    ("public/boxes/track-group-ster.mp4", 138, 0x37f0734af6652de0),
    ("public/boxes/tsel.mp4", 138, 0x7702e8e3710f394b),
    ("public/cenc-av-enca.mp4", 55, 0xe5ba35ce954f4cbe),
    ("public/cenc-boxes/aac-cenc.mp4", 88, 0xdae8edaddb88f40e),
    ("public/cenc-boxes/pcm-clear.mp4", 8000, 0xf572aac9e168761c),
    ("public/cenc-boxes/v1-mvhd.mp4", 6, 0xd0715feec8f565c5),
    ("public/cenc-mixed-tracks.mp4", 55, 0xe5ba35ce954f4cbe),
    ("public/cenc.mp4", 15, 0x674b95f313189bc1),
    ("public/clap-vs-crop.mp4", 4, 0x3864bcd0a62518e4),
    ("public/colr-hdr.mp4", 8, 0x0f84495876b93c48),
    ("public/flac.mp4", 10, 0x9fb5067e3b6025fb),
    ("public/h264-aac-prog.mp4", 138, 0xd99ebf595e424749),
    ("public/h264-high.mp4", 15, 0x6972fa5cab4852c3),
    ("public/hevc-main.mp4", 15, 0xc2f52c68546008f5),
    ("public/hevc-vps-timing.mp4", 25, 0xf1063228af5c5f8a),
    ("public/hevc-vui-field-hrd.mp4", 25, 0x614684db68418c4f),
    ("public/hevc-vui-field.mp4", 25, 0x8cf0584a91196d8b),
    ("public/hevc-vui-hrd.mp4", 25, 0xa580c7ecbacc556d),
    ("public/hevc-vui-ntsc.mp4", 25, 0x03612a6d002ee269),
    ("public/opus.mp4", 51, 0x596ceef0da2d0701),
    (
        "public/progressive/av-faststart.mp4",
        206,
        0xede504f0a9d21e8a,
    ),
    ("public/progressive/av-prog.mp4", 206, 0xe001a5530a734e57),
    ("public/stpp.mp4", 1, 0x64901616d0f89f1f),
    ("public/tref-avcp.mp4", 36, 0x7f966936aab8f361),
    ("public/vp9.mp4", 12, 0xc0b9c868fa5831a4),
    (
        "private/camera/gopro-hero6-ble.mp4",
        1675,
        0x8ab5de350213ed19,
    ),
    ("private/camera/gopro-karma.mp4", 1885, 0x0138e9aa4e36a77b),
    (
        "private/camera/gopro-max-heromode.mp4",
        1641,
        0xf9a53544fa3f24b0,
    ),
    (
        "private/quicktime/iphone-x-live-photo.mov",
        117_576,
        0x80b6_7132_6410_23f4,
    ),
    (
        "private/quicktime/lavf-aac-tmcd.mov",
        146,
        0x5637879a7f846e9f,
    ),
    (
        "private/quicktime/lavf-pcm24.mov",
        96000,
        0xea24e6a078b18ae2,
    ),
    ("private/tags/afconvert-aac.m4a", 132, 0xd653a8047085b21a),
    ("private/tags/lavf-tagged.m4a", 132, 0xe27361bec595ff4b),
    (
        "private/tags/mp4box-chapters-nero.m4a",
        132,
        0xe27361bec595ff4b,
    ),
];
