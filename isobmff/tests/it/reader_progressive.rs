//! The progressive [`Reader`]: every fixture's steps against slice 1's cursor,
//! decode-time ordering, seeking, and the `moov`-after-`mdat` walk.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use isobmff::sample_table::{SampleRef, TrackCursor};
use isobmff::segments::FileTypeBox;
use isobmff::{ByteRange, Limits, Reader, ReaderConfig, SampleOrder, Source, Step, TrackInfo};

/// Progressive fixtures the walk must exercise: enough that a checkout with
/// no usable files fails rather than passing vacuously.
const MIN_PROGRESSIVE_FIXTURES: usize = 10;
/// Fixtures with a `moov` after an `mdat` the walk must find.
const MIN_MOOV_AFTER_MDAT_FIXTURES: usize = 1;
/// Fixtures with a `moov` before an `mdat` the walk must find.
const MIN_MOOV_FIRST_FIXTURES: usize = 1;

#[derive(Debug, Default)]
struct Outcome {
    tracks: Vec<TrackInfo>,
    samples: Vec<SampleRef>,
    requests: Vec<ByteRange>,
}

fn drive(bytes: &[u8], config: ReaderConfig) -> isobmff::Result<Outcome> {
    let mut reader = Reader::new(config);
    let mut outcome = Outcome::default();
    loop {
        match reader.step()? {
            Step::Need(range) => {
                outcome.requests.push(range);
                let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                let slice = bytes.get(start..end).unwrap_or(&[]);
                reader.supply(range.offset, slice)?;
            }
            Step::Tracks(tracks) => outcome.tracks = tracks.to_vec(),
            Step::Metadata(_) => {}
            Step::Sample(sample) => outcome.samples.push(sample),
            Step::Done => break,
            _ => break,
        }
    }
    Ok(outcome)
}

fn base_config(bytes: &[u8], order: SampleOrder) -> ReaderConfig {
    ReaderConfig::default()
        .with_total_len(Some(bytes.len() as u64))
        .with_order(order)
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

fn cursor_rows(cursor: &mut TrackCursor<'_>) -> Vec<SampleRef> {
    cursor.map(|sample| sample.expect("cursor row")).collect()
}

#[test]
fn file_order_matches_the_lazy_cursor_per_track() {
    let mut exercised = 0usize;
    for (key, path) in fixtures() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(outcome) = drive(&bytes, base_config(&bytes, SampleOrder::File)) else {
            continue;
        };
        if outcome.tracks.is_empty() || outcome.samples.is_empty() {
            continue;
        }
        let mut reader = Reader::new(base_config(&bytes, SampleOrder::File));
        let mut plan = None;
        loop {
            match reader.step().expect("reader step") {
                Step::Need(range) => {
                    let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                    let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                    reader
                        .supply(range.offset, bytes.get(start..end).unwrap_or(&[]))
                        .expect("supply");
                }
                Step::Tracks(_) => {
                    plan = reader.plan();
                    break;
                }
                Step::Sample(_) | Step::Done => break,
                _ => break,
            }
        }
        let plan = plan.unwrap_or_else(|| panic!("{key}: no plan"));
        let mut grouped: BTreeMap<u32, Vec<SampleRef>> = BTreeMap::new();
        for sample in &outcome.samples {
            grouped
                .entry(sample.track.value())
                .or_default()
                .push(sample.clone());
        }
        for info in plan.tracks() {
            let mut cursor = plan
                .cursor(info.track_id, &Limits::default())
                .expect("plan cursor");
            let expected = cursor_rows(&mut cursor);
            let got = grouped
                .get(&info.track_id.value())
                .cloned()
                .unwrap_or_default();
            assert_eq!(
                got, expected,
                "{key}: track {} differs from the lazy cursor",
                info.track_id
            );
        }
        exercised += 1;
    }
    assert!(
        exercised >= MIN_PROGRESSIVE_FIXTURES,
        "only {exercised} progressive fixtures exercised"
    );
}

#[test]
fn decode_time_order_is_non_decreasing_across_tracks() {
    let mut exercised = 0usize;
    for (key, path) in fixtures() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(outcome) = drive(&bytes, base_config(&bytes, SampleOrder::DecodeTime)) else {
            continue;
        };
        if outcome.samples.is_empty() {
            continue;
        }
        exercised += 1;
        let timescales: BTreeMap<u32, u32> = outcome
            .tracks
            .iter()
            .map(|track| (track.track_id.value(), track.timescale.get()))
            .collect();
        let track_order: BTreeMap<u32, usize> = outcome
            .tracks
            .iter()
            .enumerate()
            .map(|(index, track)| (track.track_id.value(), index))
            .collect();
        let mut previous: Option<(u128, u32)> = None;
        let mut previous_track: Option<u32> = None;
        for sample in &outcome.samples {
            let timescale = *timescales
                .get(&sample.track.value())
                .unwrap_or_else(|| panic!("{key}: unknown track timescale"));
            let instant = u128::from(sample.decode_time);
            if let Some((prev_instant, prev_timescale)) = previous {
                let left = instant * u128::from(prev_timescale);
                let right = prev_instant * u128::from(timescale);
                assert!(
                    left >= right,
                    "{key}: decode time went backwards at track {}",
                    sample.track
                );
                let previous_track = *previous_track
                    .as_ref()
                    .unwrap_or_else(|| panic!("{key}: missing previous track"));
                if left == right {
                    assert!(
                        track_order[&previous_track] <= track_order[&sample.track.value()],
                        "{key}: equal decode time left track order"
                    );
                }
            }
            previous = Some((instant, timescale));
            previous_track = Some(sample.track.value());
        }
    }
    assert!(
        exercised >= MIN_PROGRESSIVE_FIXTURES,
        "only {exercised} progressive fixtures exercised"
    );
}

fn top_level_offsets(bytes: &[u8]) -> BTreeMap<[u8; 4], u64> {
    let mut out = BTreeMap::new();
    let mut offset = 0usize;
    while offset + 8 <= bytes.len() {
        let size = u32::from_be_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        let fourcc = [
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ];
        if size < 8 {
            break;
        }
        out.entry(fourcc).or_insert(offset as u64);
        offset = offset.saturating_add(size);
    }
    out
}

#[test]
fn moov_after_mdat_is_refused_over_a_forward_only_source() {
    let mut found = 0usize;
    for (key, path) in fixtures() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let boxes = top_level_offsets(&bytes);
        let (Some(&moov), Some(&mdat)) = (boxes.get(b"moov"), boxes.get(b"mdat")) else {
            continue;
        };
        if moov < mdat {
            continue;
        }
        found += 1;
        let config = base_config(&bytes, SampleOrder::File).with_source(Source::ForwardOnly);
        let error = drive(&bytes, config).expect_err(&format!("{key}: moov after mdat"));
        assert!(
            matches!(error, isobmff::Error::MoovAfterMdatForwardOnly),
            "{key}: {error:?}"
        );
    }
    assert!(
        found >= MIN_MOOV_AFTER_MDAT_FIXTURES,
        "no moov-after-mdat fixture was exercised"
    );
}

#[test]
fn moov_first_never_requests_a_backward_range() {
    let mut found = 0usize;
    for (key, path) in fixtures() {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let boxes = top_level_offsets(&bytes);
        let (Some(&moov), Some(&mdat)) = (boxes.get(b"moov"), boxes.get(b"mdat")) else {
            continue;
        };
        if moov > mdat {
            continue;
        }
        let config = base_config(&bytes, SampleOrder::File).with_source(Source::ForwardOnly);
        let Ok(outcome) = drive(&bytes, config) else {
            continue;
        };
        found += 1;
        let mut previous = 0u64;
        for range in &outcome.requests {
            assert!(
                range.offset >= previous,
                "{key}: backward request {range:?} after {previous}"
            );
            previous = range.offset;
        }
    }
    assert!(
        found >= MIN_MOOV_FIRST_FIXTURES,
        "no moov-first fixture was exercised"
    );
}

// Sample counts per camera file, from
// `ffprobe -v error -show_entries stream=index,codec_tag_string,nb_frames -of csv=p=0 <file>`
// (`nb_read_packets` reports `N/A` for the `fdsc` data track, whose `nb_frames`
// is its `stsz` sample count).
const GOPRO_COUNTS: &[(&str, &[(&str, u32)])] = &[
    (
        "camera/gopro-karma.mp4",
        &[
            ("avc1", 362),
            ("mp4a", 566),
            ("tmcd", 1),
            ("gpmd", 13),
            ("fdsc", 943),
        ],
    ),
    (
        "camera/gopro-max-heromode.mp4",
        &[
            ("avc1", 315),
            ("mp4a", 493),
            ("tmcd", 1),
            ("gpmd", 11),
            ("fdsc", 821),
        ],
    ),
    (
        "camera/gopro-hero6-ble.mp4",
        &[
            ("avc1", 279),
            ("mp4a", 546),
            ("tmcd", 1),
            ("gpmd", 11),
            ("fdsc", 838),
        ],
    ),
];

#[test]
fn the_camera_files_report_every_track() {
    for (name, expected) in GOPRO_COUNTS {
        let Some(bytes) =
            test_fixtures::require_bytes(&format!("mp4/{name}"), "reader_progressive")
        else {
            return;
        };
        let outcome = drive(&bytes, base_config(&bytes, SampleOrder::File)).expect("camera read");
        assert_eq!(outcome.tracks.len(), 5, "{name}: track count");
        let mut per_fourcc: BTreeMap<[u8; 4], u32> = BTreeMap::new();
        for track in &outcome.tracks {
            let entry = track
                .entries
                .first()
                .unwrap_or_else(|| panic!("{name}: track {} has no entry", track.track_id));
            assert!(!entry.body.is_empty(), "{name}: raw entry body");
            let count = outcome
                .samples
                .iter()
                .filter(|sample| sample.track == track.track_id)
                .count();
            per_fourcc.insert(entry.fourcc, count as u32);
        }
        for (fourcc, count) in *expected {
            let key: [u8; 4] = fourcc.as_bytes().try_into().expect("four-cc");
            assert_eq!(
                per_fourcc.get(&key).copied(),
                Some(*count),
                "{name}: {fourcc} sample count"
            );
        }
    }
}

fn identity_matrix() -> [i32; 9] {
    [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000]
}

fn tiny_mp4() -> Vec<u8> {
    use broadcast_core::Serialize;
    use broadcast_core::timescale::Timescale;
    use isobmff::TrackId;
    use isobmff::box_support::OpaqueBox;
    use isobmff::init_segment::{
        ChunkOffsetBox, HandlerBox, MediaBox, MediaHeaderBox, MediaInformationBox, MovieBox,
        MovieHeaderBox, SampleDescriptionBox, SampleEntryVariant, SampleSizeBox, SampleTableBox,
        SampleToChunkBox, StblChild, StscEntry, SyncSampleBox, TrackBox, TrackHeaderBox,
        VideoMediaHeaderBox,
    };
    use isobmff::segments::FileTypeBox;
    use isobmff::timing::{SttsEntry, TimeToSampleBox};

    let timescale = Timescale::try_new(1000).expect("timescale");
    let stsd = SampleDescriptionBox::new(
        0,
        0,
        vec![
            SampleEntryVariant::Unknown(OpaqueBox::new(*b"zzzz", vec![])),
            SampleEntryVariant::Unknown(OpaqueBox::new(*b"yyyy", vec![])),
        ],
    );
    let stbl = SampleTableBox {
        children: vec![
            StblChild::Stsd(stsd),
            StblChild::Stts(TimeToSampleBox {
                version: 0,
                flags: 0,
                entries: vec![SttsEntry {
                    sample_count: 3,
                    sample_delta: 40,
                }],
            }),
            StblChild::Stsc(SampleToChunkBox {
                version: 0,
                flags: 0,
                entries: vec![StscEntry {
                    first_chunk: 1,
                    samples_per_chunk: 3,
                    sample_description_index: 2,
                }],
            }),
            StblChild::Stsz(SampleSizeBox::new(0, 0, 4, 3, vec![])),
            StblChild::Stco(ChunkOffsetBox {
                version: 0,
                flags: 0,
                entries: vec![0],
            }),
            StblChild::Stss(SyncSampleBox {
                version: 0,
                flags: 0,
                entries: vec![1, 3],
            }),
        ],
    };
    let minf = MediaInformationBox::new(
        Some(VideoMediaHeaderBox {
            version: 0,
            flags: 1,
            graphicsmode: 0,
            opcolor: [0, 0, 0],
        }),
        None,
        None,
        Some(stbl),
        vec![],
    );
    let mdia = MediaBox::new(
        Some(MediaHeaderBox {
            version: 0,
            flags: 0,
            creation_time: 0,
            modification_time: 0,
            timescale,
            duration: 120,
            language: 0x55c4,
            quality: Default::default(),
        }),
        Some(HandlerBox {
            version: 0,
            flags: 0,
            pre_defined: 0,
            reserved: [0u8; 12],
            handler_type: *b"vide",
            name: vec![],
        }),
        Some(minf),
        vec![],
    );
    let trak = TrackBox::new(
        TrackHeaderBox {
            version: 0,
            flags: 3,
            creation_time: 0,
            modification_time: 0,
            track_id: TrackId::new(1),
            reserved: Default::default(),
            duration: 120,
            reserved_2: Default::default(),
            layer: 0,
            alternate_group: 0,
            volume: 0,
            reserved_3: Default::default(),
            matrix: identity_matrix(),
            width: 0,
            height: 0,
        },
        None,
        Some(mdia),
        vec![],
    );
    let mvhd = MovieHeaderBox {
        version: 0,
        flags: 0,
        creation_time: 0,
        modification_time: 0,
        timescale,
        duration: 120,
        rate: 0x0001_0000,
        volume: 0x0100,
        reserved: Default::default(),
        matrix: identity_matrix(),
        pre_defined: Default::default(),
        next_track_id: TrackId::new(2),
    };
    let ftyp = FileTypeBox {
        major_brand: *b"iso5",
        minor_version: 0,
        compatible_brands: vec![*b"iso5", *b"iso6", *b"mp41"],
    };
    let ftyp_len = ftyp.serialized_len();
    let mut movie = MovieBox::new(mvhd.clone(), vec![trak], None, vec![]);
    let mdat_header_len = 8usize;
    let moov_len = movie.serialized_len();
    let payload = (ftyp_len + moov_len + mdat_header_len) as u32;
    if let Some(StblChild::Stco(stco)) = movie.tracks[0]
        .mdia
        .as_mut()
        .and_then(|mdia| mdia.minf.as_mut())
        .and_then(|minf| minf.stbl.as_mut())
        .and_then(|stbl| {
            stbl.children
                .iter_mut()
                .find(|child| matches!(child, StblChild::Stco(_)))
        })
    {
        stco.entries = vec![payload];
    }
    let mut out = vec![0u8; ftyp_len + movie.serialized_len() + mdat_header_len + 12];
    let mut cursor = 0usize;
    cursor += ftyp.serialize_into(&mut out[cursor..]).expect("ftyp");
    cursor += movie.serialize_into(&mut out[cursor..]).expect("moov");
    let mdat_len = (out.len() - cursor) as u32;
    out[cursor..cursor + 4].copy_from_slice(&mdat_len.to_be_bytes());
    out[cursor + 4..cursor + 8].copy_from_slice(b"mdat");
    cursor += mdat_header_len;
    out[cursor..cursor + 12].copy_from_slice(b"ABCDEFGHIJKL");
    out.truncate(cursor + 12);
    out
}

#[test]
fn a_second_stsd_entry_is_named_by_each_sample() {
    let bytes = tiny_mp4();
    let outcome = drive(&bytes, base_config(&bytes, SampleOrder::File)).expect("tiny read");
    assert_eq!(outcome.tracks.len(), 1);
    let track = &outcome.tracks[0];
    assert_eq!(track.entries.len(), 2);
    assert_eq!(track.entries[0].fourcc, *b"zzzz");
    assert_eq!(track.entries[1].fourcc, *b"yyyy");
    assert!(track.entries[0].typed.is_none());
    assert_eq!(outcome.samples.len(), 3);
    for sample in &outcome.samples {
        assert_eq!(sample.description_index, 2);
        assert_eq!(sample.size, 4);
    }
    let sync: Vec<bool> = outcome.samples.iter().map(|sample| sample.sync).collect();
    assert_eq!(sync, vec![true, false, true]);
}

#[test]
fn seek_lands_on_the_sync_sample_at_or_before_the_time() {
    let bytes = tiny_mp4();
    let mut reader = Reader::new(base_config(&bytes, SampleOrder::File));
    let mut track = None;
    loop {
        match reader.step().expect("step") {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let end = usize::try_from(range.end()).expect("end");
                reader
                    .supply(range.offset, &bytes[start..end])
                    .expect("supply");
            }
            Step::Tracks(tracks) => track = tracks.first().map(|info| info.track_id),
            Step::Metadata(_) => {}
            Step::Done | Step::Sample(_) => break,
            _ => break,
        }
    }
    let track = track.expect("track");
    reader.seek_to(track, 80).expect("seek");
    let mut seen = Vec::new();
    loop {
        match reader.step().expect("step") {
            Step::Sample(sample) => seen.push(sample),
            Step::Done => break,
            _ => break,
        }
    }
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].index, 2);
    assert_eq!(seen[0].decode_time, 80);
}

#[test]
fn supply_must_match_the_outstanding_need() {
    let bytes = tiny_mp4();
    let mut reader = Reader::new(base_config(&bytes, SampleOrder::File));
    let Step::Need(range) = reader.step().expect("need") else {
        panic!("expected a need");
    };
    let error = reader
        .supply(range.offset + 1, &bytes[1..9])
        .expect_err("mismatch");
    assert!(matches!(error, isobmff::Error::SupplyMismatch { .. }));
    let too_long = vec![0u8; usize::try_from(range.len).expect("len") + 1];
    let error = reader
        .supply(range.offset, &too_long)
        .expect_err("too long");
    assert!(matches!(error, isobmff::Error::SupplyMismatch { .. }));
}

#[test]
fn an_unknown_total_length_still_finishes() {
    let bytes = tiny_mp4();
    let config = ReaderConfig::default().with_order(SampleOrder::File);
    let outcome = drive(&bytes, config).expect("unknown-length read");
    assert_eq!(outcome.samples.len(), 3);
}

#[test]
fn every_hostile_cap_bites_one_over() {
    let bytes = tiny_mp4();
    let moov_len = isobmff::box_types::find_box(&bytes, b"moov")
        .map(|moov| moov.len() as u64)
        .expect("moov");
    let cases: &[(&str, Limits)] = &[
        ("depth", Limits::default().with_max_depth(2)),
        (
            "moov size",
            Limits::default().with_max_moov_bytes(moov_len - 1),
        ),
        (
            "samples per track",
            Limits::default().with_max_samples_per_track(2),
        ),
        ("sample size", Limits::default().with_max_sample_bytes(3)),
    ];
    for (what, limits) in cases {
        // A cap is a caller's hard bound: a validating read stops at it.
        let config = base_config(&bytes, SampleOrder::File)
            .with_limits(*limits)
            .with_mode(isobmff::Mode::Strict);
        let error = drive(&bytes, config).expect_err(what);
        assert!(
            matches!(error, isobmff::Error::LimitExceeded { .. }),
            "{what}: {error:?}"
        );
        // And a best-effort read never hands out a sample the cap refused.
        if *what == "sample size" {
            let config = base_config(&bytes, SampleOrder::File).with_limits(*limits);
            let outcome = drive(&bytes, config).expect("best-effort read");
            assert!(
                outcome.samples.iter().all(|sample| sample.size <= 3),
                "every sample is inside the cap: {outcome:?}"
            );
        }
    }
}

#[test]
fn fragment_and_metadata_caps_do_not_constrain_a_progressive_file() {
    let bytes = tiny_mp4();
    let limits = Limits::default()
        .with_max_fragment_bytes(0)
        .with_max_metadata_item_bytes(0);
    let config = base_config(&bytes, SampleOrder::File).with_limits(limits);
    let outcome = drive(&bytes, config).expect("progressive read");
    assert_eq!(outcome.samples.len(), 3);
}

struct SequenceOutcome {
    brand: FileTypeBox,
    tracks: Vec<TrackInfo>,
    samples: Vec<SampleRef>,
    len: u64,
}

fn drive_sequence(name: &str) -> SequenceOutcome {
    let bytes = test_fixtures::read(name);
    let mut reader = Reader::new(base_config(&bytes, SampleOrder::File));
    let mut tracks = Vec::new();
    let mut samples = Vec::new();
    loop {
        match reader.step().expect("step") {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).unwrap_or(usize::MAX);
                let end = usize::try_from(range.end()).unwrap_or(usize::MAX);
                let slice = bytes.get(start..end).unwrap_or(&[]);
                reader.supply(range.offset, slice).expect("supply");
            }
            Step::Tracks(found) => tracks = found.to_vec(),
            Step::Sample(sample) => samples.push(sample),
            Step::Done => break,
            _ => {}
        }
    }
    let brand = reader.brand().expect("ftyp").clone();
    SequenceOutcome {
        brand,
        tracks,
        samples,
        len: bytes.len() as u64,
    }
}

#[test]
fn avif_image_sequence_fixture_reads_its_track() {
    let outcome = drive_sequence("avif/sequence.avif");
    assert_eq!(&outcome.brand.major_brand, b"avis");
    for brand in [
        b"avif", b"avis", b"msf1", b"iso8", b"mif1", b"miaf", b"MA1A",
    ] {
        assert!(
            outcome
                .brand
                .compatible_brands
                .iter()
                .any(|compatible| compatible == brand),
            "missing brand {:?}",
            core::str::from_utf8(brand)
        );
    }
    assert_eq!(outcome.tracks.len(), 1);
    let track = &outcome.tracks[0];
    assert_eq!(&track.handler, b"pict");
    assert!(track.is_image_sequence());
    assert_eq!(track.timescale.get(), 10);
    assert_eq!(track.duration, 4);
    let constraints = track.coding_constraints().expect("ccst");
    assert!(!constraints.all_ref_pics_intra());
    assert!(constraints.intra_pred_used());
    assert_eq!(constraints.max_ref_per_pic(), 15);
    assert_eq!(outcome.samples.len(), 4);
    for (index, sample) in outcome.samples.iter().enumerate() {
        let index = index as u64;
        assert_eq!(u64::from(sample.index), index);
        assert_eq!(sample.decode_time, index);
        assert_eq!(sample.duration, 1);
        assert!(sample.offset + u64::from(sample.size) <= outcome.len);
    }
    let total: u64 = outcome
        .samples
        .iter()
        .map(|sample| u64::from(sample.size))
        .sum();
    assert_eq!(total, 4059);
}

#[test]
fn heic_image_sequence_fixture_reads_its_track() {
    let outcome = drive_sequence("heif/sequence.heic");
    assert_eq!(&outcome.brand.major_brand, b"hevc");
    for brand in [b"mif1", b"heic", b"miaf", b"msf1", b"isom", b"hevc"] {
        assert!(
            outcome
                .brand
                .compatible_brands
                .iter()
                .any(|compatible| compatible == brand),
            "missing brand {:?}",
            core::str::from_utf8(brand)
        );
    }
    assert_eq!(outcome.tracks.len(), 1);
    let track = &outcome.tracks[0];
    assert_eq!(&track.handler, b"pict");
    assert!(track.is_image_sequence());
    assert_eq!(track.timescale.get(), 10);
    assert_eq!(track.duration, 4);
    let constraints = track.coding_constraints().expect("ccst");
    assert!(constraints.all_ref_pics_intra());
    assert!(constraints.intra_pred_used());
    assert_eq!(constraints.max_ref_per_pic(), 0);
    assert_eq!(outcome.samples.len(), 4);
    for (index, sample) in outcome.samples.iter().enumerate() {
        let index = index as u64;
        assert_eq!(u64::from(sample.index), index);
        assert_eq!(sample.decode_time, index);
        assert_eq!(sample.duration, 1);
        assert!(sample.offset + u64::from(sample.size) <= outcome.len);
    }
    let total: u64 = outcome
        .samples
        .iter()
        .map(|sample| u64::from(sample.size))
        .sum();
    assert_eq!(total, 2880);
}
