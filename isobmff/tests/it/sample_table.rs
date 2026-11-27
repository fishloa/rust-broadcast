//! The lazy sample-table cursor: differential digest against the pre-move
//! layout, hostile-table limits, and seeking.

use std::path::{Path, PathBuf};

use broadcast_core::Parse;
use isobmff::Limits;
use isobmff::TrackId;
use isobmff::init_segment::{MOOV, MovieBox, SampleTableBox, StblChild, TrackBox};
use isobmff::sample_table::TrackCursor;

#[global_allocator]
static GLOBAL: test_alloc::ThreadCapped<ALLOC_BUDGET> = test_alloc::ThreadCapped::new();

/// Per-thread live-allocation budget. A sample-table cursor that allocated in
/// proportion to a hostile `sample_count` (~4 billion entries) would abort
/// here instead of exhausting the host.
const ALLOC_BUDGET: usize = 256 << 20;

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
    ("public/aac-prol.mp4", 131, 0x9f808b3f4836a623),
    ("public/aac-sgpd.mp4", 131, 0x9f808b3f4836a623),
    ("public/av1.mp4", 12, 0xdabde3d714765d4d),
    ("public/boxes/chnl-ipcm.mp4", 14400, 0x2b1ad55bb77541e5),
    ("public/boxes/cprt.mp4", 138, 0xc392f59faac11677),
    ("public/boxes/hint-hmhd.mp4", 207, 0xbfaf49d17d4758c1),
    ("public/boxes/hint-latm.mp4", 38, 0x8133c95a4c1329b4),
    ("public/boxes/hint-mtu-copy.mp4", 230, 0x74b6d72b73f6237a),
    ("public/boxes/hint-offsets.mp4", 207, 0xe4b4b348839aa554),
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
    ("public/quicktime/qt-h263.mov", 1, 0xf174416030b74019),
    ("public/quicktime/qt-mjpeg.mov", 1, 0x45ffb90f01490efa),
    ("public/quicktime/qt-pcm-s16le.mov", 800, 0x6952d21c0fc44120),
    ("public/quicktime/qt-pcm-s24le.mov", 800, 0x6459263dccb6f6e3),
    ("public/quicktime/qt-prores-ap4h.mov", 1, 0xaac021a1ece99b00),
    ("public/quicktime/qt-prores-ap4x.mov", 1, 0xdd332e1c80cbba15),
    ("public/quicktime/qt-prores-apch.mov", 1, 0xbcb91f85c66fcb50),
    ("public/quicktime/qt-prores-apcn.mov", 1, 0xddc15c7881fa8b35),
    ("public/quicktime/qt-prores-apco.mov", 1, 0x76688f1dbef23c68),
    ("public/quicktime/qt-prores-apcs.mov", 1, 0x2ed1986f051f0a53),
    ("public/quicktime/qt-svq1.mov", 1, 0x54ef33d0cfa2d461),
];

use isobmff::Error;
use isobmff::init_segment::{
    ChunkOffsetBox, CompactSampleSizeBox, SampleSizeBox, SampleToChunkBox, StscEntry, SyncSampleBox,
};
use isobmff::timing::{CompositionOffsetBox, CttsEntry, SttsEntry, TimeToSampleBox};

fn stts(entries: &[(u32, u32)]) -> StblChild {
    StblChild::Stts(TimeToSampleBox {
        version: 0,
        flags: 0,
        entries: entries
            .iter()
            .map(|&(sample_count, sample_delta)| SttsEntry {
                sample_count,
                sample_delta,
            })
            .collect(),
    })
}

fn ctts(entries: &[(u32, i32)]) -> StblChild {
    StblChild::Ctts(CompositionOffsetBox {
        version: 1,
        flags: 0,
        entries: entries
            .iter()
            .map(|&(sample_count, sample_offset)| CttsEntry {
                sample_count,
                sample_offset,
            })
            .collect(),
    })
}

fn stsc(entries: &[(u32, u32, u32)]) -> StblChild {
    StblChild::Stsc(SampleToChunkBox {
        version: 0,
        flags: 0,
        entries: entries
            .iter()
            .map(
                |&(first_chunk, samples_per_chunk, sample_description_index)| StscEntry {
                    first_chunk,
                    samples_per_chunk,
                    sample_description_index,
                },
            )
            .collect(),
    })
}

fn stsz_uniform(size: u32, count: u32) -> StblChild {
    StblChild::Stsz(SampleSizeBox {
        version: 0,
        flags: 0,
        sample_size: size,
        sample_count: count,
        entries: Vec::new(),
    })
}

fn stco(offsets: &[u32]) -> StblChild {
    StblChild::Stco(ChunkOffsetBox {
        version: 0,
        flags: 0,
        entries: offsets.to_vec(),
    })
}

fn stss(entries: &[u32]) -> StblChild {
    StblChild::Stss(SyncSampleBox {
        version: 0,
        flags: 0,
        entries: entries.to_vec(),
    })
}

fn table(children: Vec<StblChild>) -> SampleTableBox {
    SampleTableBox { children }
}

fn cursor(stbl: &SampleTableBox, file_len: Option<u64>) -> Result<TrackCursor<'_>, Error> {
    TrackCursor::new(stbl, TrackId::new(1), &Limits::default(), file_len)
}

#[test]
fn a_sample_count_above_the_limit_is_rejected_without_allocating() {
    // ~4 billion samples; allocating an entry per sample would be tens of
    // gigabytes and abort under the capped allocator above.
    let stbl = table(vec![
        stts(&[(1, 1)]),
        stsc(&[(1, 1, 1)]),
        stsz_uniform(1, 4_000_000_000),
        stco(&[0]),
    ]);
    match cursor(&stbl, Some(1 << 40)) {
        Err(Error::LimitExceeded {
            what: "samples per track",
            ..
        }) => {}
        other => panic!("expected a samples-per-track limit error, got {other:?}"),
    }
    // A lower cap rejects a count that is legal under the default.
    let limits = Limits::default().with_max_samples_per_track(3);
    let stbl = table(vec![
        stts(&[(4, 1)]),
        stsc(&[(1, 4, 1)]),
        stsz_uniform(1, 4),
        stco(&[0]),
    ]);
    assert!(matches!(
        TrackCursor::new(&stbl, TrackId::new(1), &limits, Some(100)),
        Err(Error::LimitExceeded {
            what: "samples per track",
            limit: 3
        })
    ));
}

#[test]
fn a_sample_size_above_the_limit_is_rejected() {
    let stbl = table(vec![
        stts(&[(2, 1)]),
        stsc(&[(1, 2, 1)]),
        stsz_uniform(1 << 30, 2),
        stco(&[0]),
    ]);
    match cursor(&stbl, Some(1 << 40)) {
        Err(Error::LimitExceeded {
            what: "sample size",
            ..
        }) => {}
        other => panic!("expected a sample-size limit error, got {other:?}"),
    }
}

#[test]
fn non_ascending_first_chunk_is_a_typed_error() {
    for entries in [
        &[(2, 1, 1), (2, 1, 1)][..],
        &[(0, 1, 1)],
        &[(3, 1, 1), (1, 1, 1)],
    ] {
        let stbl = table(vec![
            stts(&[(1, 1)]),
            stsc(entries),
            stsz_uniform(1, 1),
            stco(&[0, 0, 0]),
        ]);
        match cursor(&stbl, Some(1 << 20)) {
            Err(Error::InvalidValue(value)) => assert_eq!(value.field, "stsc.first_chunk"),
            other => panic!("expected an stsc.first_chunk error, got {other:?}"),
        }
    }
}

#[test]
fn a_zero_sample_description_index_is_a_typed_error() {
    let stbl = table(vec![
        stts(&[(1, 1)]),
        stsc(&[(1, 1, 0)]),
        stsz_uniform(1, 1),
        stco(&[0]),
    ]);
    match cursor(&stbl, Some(1 << 20)) {
        Err(Error::InvalidValue(value)) => {
            assert_eq!(value.field, "stsc.sample_description_index");
        }
        other => panic!("expected a description-index error, got {other:?}"),
    }
}

#[test]
fn a_chunk_offset_past_the_file_length_is_a_typed_error() {
    let stbl = table(vec![
        stts(&[(1, 100)]),
        stsc(&[(1, 1, 1)]),
        stsz_uniform(10, 1),
        stco(&[1000]),
    ]);
    let mut cursor = cursor(&stbl, Some(100)).expect("the tables resolve up front");
    assert!(matches!(cursor.next(), Some(Err(Error::BufferTooShort(_)))));
}

#[test]
fn overlapping_chunks_past_the_byte_budget_are_a_typed_error() {
    let stbl = table(vec![
        stts(&[(4, 1)]),
        stsc(&[(1, 1, 1)]),
        stsz_uniform(64, 4),
        stco(&[0, 0, 0, 0]),
    ]);
    let mut cursor = cursor(&stbl, Some(100)).expect("the tables resolve up front");
    let mut last = None;
    for item in &mut cursor {
        last = Some(item);
    }
    assert!(matches!(
        last,
        Some(Err(Error::SampleDataBudgetExceeded { limit: 100 }))
    ));
}

#[test]
fn a_short_stts_run_errors_at_the_sample_where_it_ends() {
    let stbl = table(vec![
        stts(&[(1, 100)]),
        stsc(&[(1, 2, 1)]),
        stsz_uniform(4, 2),
        stco(&[0]),
    ]);
    let mut cursor = cursor(&stbl, Some(1 << 20)).expect("the tables resolve up front");
    assert!(matches!(cursor.next(), Some(Ok(_))));
    assert!(matches!(cursor.next(), Some(Err(Error::InvalidInput(_)))));
    assert!(cursor.next().is_none());
}

#[test]
fn a_long_stts_run_errors_at_the_end() {
    let stbl = table(vec![
        stts(&[(3, 100)]),
        stsc(&[(1, 2, 1)]),
        stsz_uniform(4, 2),
        stco(&[0]),
    ]);
    let mut cursor = cursor(&stbl, Some(1 << 20)).expect("the tables resolve up front");
    assert!(matches!(cursor.next(), Some(Ok(_))));
    assert!(matches!(cursor.next(), Some(Ok(_))));
    assert!(matches!(cursor.next(), Some(Err(Error::InvalidInput(_)))));
    assert!(cursor.next().is_none());
}

#[test]
fn a_mismatched_ctts_run_is_a_typed_error() {
    let stbl = table(vec![
        stts(&[(2, 100)]),
        ctts(&[(1, 5)]),
        stsc(&[(1, 2, 1)]),
        stsz_uniform(4, 2),
        stco(&[0]),
    ]);
    let mut cursor = cursor(&stbl, Some(1 << 20)).expect("the tables resolve up front");
    assert!(matches!(cursor.next(), Some(Ok(_))));
    assert!(matches!(cursor.next(), Some(Err(Error::InvalidInput(_)))));
}

#[test]
fn both_size_tables_present_is_an_error() {
    let stbl = table(vec![
        stts(&[(1, 1)]),
        stsc(&[(1, 1, 1)]),
        stsz_uniform(1, 1),
        StblChild::Stz2(CompactSampleSizeBox {
            version: 0,
            flags: 0,
            field_size: 8,
            sample_count: 1,
            entries: vec![1],
        }),
        stco(&[0]),
    ]);
    assert!(matches!(
        cursor(&stbl, Some(1 << 20)),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn a_stz2_size_table_is_read_when_stsz_is_absent() {
    let stbl = table(vec![
        stts(&[(2, 10)]),
        stsc(&[(1, 2, 1)]),
        StblChild::Stz2(CompactSampleSizeBox {
            version: 0,
            flags: 0,
            field_size: 8,
            sample_count: 2,
            entries: vec![3, 5],
        }),
        stco(&[100]),
    ]);
    let rows: Vec<_> = cursor(&stbl, Some(200))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].offset, 100);
    assert_eq!(rows[0].size, 3);
    assert_eq!(rows[1].offset, 103);
    assert_eq!(rows[1].size, 5);
}

#[test]
fn seek_lands_on_the_last_sync_sample_at_or_before_the_time() {
    // Stts: samples at decode times 0, 100, 200, 400, 600. Sync at 0, 2, 4.
    let stbl = table(vec![
        stts(&[(2, 100), (3, 200)]),
        stsc(&[(1, 5, 1)]),
        stsz_uniform(10, 5),
        stco(&[0]),
        stss(&[1, 3, 5]),
    ]);
    let mut cursor = cursor(&stbl, Some(1000)).unwrap();
    assert_eq!(cursor.len(), 5);
    assert_eq!(cursor.peek_decode_time(), Some(0));

    cursor.seek_sync(450).unwrap();
    assert_eq!(cursor.peek_decode_time(), Some(200));
    assert_eq!(cursor.next().unwrap().unwrap().index, 2);

    cursor.seek_sync(0).unwrap();
    assert_eq!(cursor.peek_decode_time(), Some(0));
    assert_eq!(cursor.next().unwrap().unwrap().index, 0);

    cursor.seek_sync(u64::MAX).unwrap();
    assert_eq!(cursor.peek_decode_time(), Some(600));
    assert_eq!(cursor.next().unwrap().unwrap().index, 4);
    assert!(cursor.is_empty());
    assert!(cursor.next().is_none());
}

#[test]
fn seek_with_no_sync_table_lands_on_the_sample_at_or_before_the_time() {
    let stbl = table(vec![
        stts(&[(5, 100)]),
        stsc(&[(1, 5, 1)]),
        stsz_uniform(10, 5),
        stco(&[0]),
    ]);
    let mut cursor = cursor(&stbl, Some(1000)).unwrap();
    cursor.seek_sync(250).unwrap();
    assert_eq!(cursor.peek_decode_time(), Some(200));
    assert_eq!(cursor.next().unwrap().unwrap().index, 2);
}

#[test]
fn description_index_comes_from_the_stsc_run() {
    let stbl = table(vec![
        stts(&[(2, 100)]),
        stsc(&[(1, 2, 3)]),
        stsz_uniform(4, 2),
        stco(&[0]),
    ]);
    let rows: Vec<_> = cursor(&stbl, Some(1000))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows[0].description_index, 3);
    assert_eq!(rows[1].description_index, 3);
}

fn stsz_entries(sizes: &[u32]) -> StblChild {
    StblChild::Stsz(SampleSizeBox {
        version: 0,
        flags: 0,
        sample_size: 0,
        sample_count: sizes.len() as u32,
        entries: sizes.to_vec(),
    })
}

fn rows(stbl: &SampleTableBox, file_len: u64) -> Vec<Row> {
    cursor(stbl, Some(file_len))
        .expect("the tables resolve up front")
        .map(|item| {
            let sample = item.expect("the tables yield");
            Row {
                offset: sample.offset,
                size: u64::from(sample.size),
                decode_time: sample.decode_time,
                composition_offset: i64::from(sample.composition_offset),
                duration: sample.duration,
                sync: sample.sync,
            }
        })
        .collect()
}

#[test]
fn surplus_chunk_capacity_is_tolerated_for_a_per_entry_stsz() {
    let stbl = table(vec![
        stts(&[(5, 10)]),
        stsc(&[(1, 3, 1)]),
        stsz_entries(&[10, 20, 30, 40, 50]),
        stco(&[100, 1000]),
    ]);
    let got: Vec<(u64, u64)> = rows(&stbl, 1 << 20)
        .iter()
        .map(|r| (r.offset, r.size))
        .collect();
    assert_eq!(
        got,
        [(100, 10), (110, 20), (130, 30), (1000, 40), (1040, 50)]
    );
}

#[test]
fn surplus_chunk_capacity_is_tolerated_for_a_uniform_stsz() {
    let stbl = table(vec![
        stts(&[(4, 10)]),
        stsc(&[(1, 3, 1)]),
        stsz_uniform(8, 4),
        stco(&[0, 100, 200]),
    ]);
    let got: Vec<(u64, u64)> = rows(&stbl, 1 << 20)
        .iter()
        .map(|r| (r.offset, r.size))
        .collect();
    assert_eq!(got, [(0, 8), (8, 8), (16, 8), (100, 8)]);
}

#[test]
fn a_chunk_table_covering_fewer_samples_is_a_typed_error() {
    for stsz in [stsz_uniform(8, 5), stsz_entries(&[8; 5])] {
        let stbl = table(vec![
            stts(&[(5, 10)]),
            stsc(&[(1, 2, 1)]),
            stsz,
            stco(&[0, 100]),
        ]);
        assert!(matches!(
            cursor(&stbl, Some(1 << 20)),
            Err(Error::InvalidInput(_))
        ));
    }
}

#[test]
fn chunks_before_the_first_run_hold_no_samples() {
    let stbl = table(vec![
        stts(&[(1, 10)]),
        stsc(&[(2, 1, 1)]),
        stsz_entries(&[7]),
        stco(&[100, 200]),
    ]);
    let got = rows(&stbl, 1 << 20);
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].offset, got[0].size), (200, 7));
}

#[test]
fn a_sample_count_equal_to_the_file_length_is_accepted() {
    let stbl = table(vec![
        stts(&[(3, 1)]),
        stsc(&[(1, 1, 1)]),
        stsz_uniform(1, 3),
        stco(&[0, 1, 2]),
    ]);
    assert_eq!(rows(&stbl, 3).len(), 3);

    let over = table(vec![
        stts(&[(4, 1)]),
        stsc(&[(1, 1, 1)]),
        stsz_uniform(1, 4),
        stco(&[0, 1, 2]),
    ]);
    assert!(matches!(
        cursor(&over, Some(3)),
        Err(Error::BufferTooShort(_))
    ));
}

#[test]
fn one_stsc_run_per_chunk_is_one_forward_pass() {
    const CHUNKS: u32 = 400;
    let stsc_runs: Vec<(u32, u32, u32)> = (1..=CHUNKS).map(|c| (c, 1, 1)).collect();
    let stco_offsets: Vec<u32> = (1..=CHUNKS).map(|c| c * 100).collect();
    let stbl = table(vec![
        stts(&[(CHUNKS, 10)]),
        stsc(&stsc_runs),
        stsz_uniform(10, CHUNKS),
        stco(&stco_offsets),
    ]);
    let got = rows(&stbl, 1 << 20);
    assert_eq!(got.len(), CHUNKS as usize);
    assert_eq!((got[0].offset, got[0].size), (100, 10));
    assert_eq!(
        (
            got[CHUNKS as usize - 1].offset,
            got[CHUNKS as usize - 1].size
        ),
        (u64::from(CHUNKS) * 100, 10)
    );
}

#[test]
fn a_malformed_opaque_stts_surfaces_its_real_parse_error() {
    // A bare 8-byte header is below `TimeToSampleBox::parse`'s own minimum, so
    // the box fails to parse and `parse_stbl_children` stores it as `Opaque`.
    let mut raw = vec![0u8; 8];
    raw[0..4].copy_from_slice(&8u32.to_be_bytes());
    raw[4..8].copy_from_slice(b"stts");
    let stbl = table(vec![
        StblChild::Opaque(raw),
        stsc(&[(1, 1, 1)]),
        stsz_uniform(1, 1),
        stco(&[0]),
    ]);
    assert!(matches!(
        cursor(&stbl, Some(1 << 20)),
        Err(Error::BufferTooShort(_))
    ));
}
