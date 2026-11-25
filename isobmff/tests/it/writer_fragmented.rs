//! The fragmented writer: init segment, `moof`+`mdat`, and the round trip of
//! every track of a real GoPro capture (five tracks, one of them a raw sample
//! entry with the `meta` handler).
//!
//! The source is walked with the box layer (`MovieBox`, the `stsd` entry
//! bytes and `TrackCursor`), the result is written with
//! [`isobmff::writer::FragmentedWriter`], and read back through
//! `MovieFragmentBox` + `frag_offsets` so every sample's bytes, duration, sync
//! flag and composition offset is compared with the source.

use broadcast_common::{Parse, Serialize};
use isobmff::box_types::{BoxIter, find_box};
use isobmff::init_segment::MovieBox;
use isobmff::limits::Limits;
use isobmff::movie_fragment::MovieFragmentBox;
use isobmff::sample_table::{SampleRef, TrackCursor};
use isobmff::writer::{
    FragmentedWriter, SampleEntry, SampleMeta, TrackSetup, Writer, fragmented_segment,
};
use isobmff::{Reader, ReaderConfig, SampleOrder, Step, TrackId, parse_box};

struct SourceTrack {
    setup: TrackSetup,
    timescale: u32,
    samples: Vec<SourceSample>,
}

struct SourceSample {
    reference: SampleRef,
    data: Vec<u8>,
}

/// The child box of `container` with the given four-CC.
fn child<'a>(container: &'a [u8], fourcc: &[u8; 4]) -> &'a [u8] {
    let body = parse_box(container).expect("a box").0.body;
    find_box(body, fourcc).expect("the child box")
}

/// The first `stsd` entry's four-CC and complete box bytes.
fn first_sample_entry(stbl: &[u8]) -> ([u8; 4], Vec<u8>) {
    let stsd = parse_box(child(stbl, b"stsd")).expect("an stsd box").0;
    // version/flags (4) then entry_count (4).
    let entry = BoxIter::new(&stsd.body[8..])
        .next()
        .expect("an stsd entry")
        .expect("a valid stsd entry");
    (entry.four_cc(), entry.bytes.to_vec())
}

fn parse_source(file: &[u8], limits: &Limits) -> Vec<SourceTrack> {
    let moov_bytes = find_box(file, b"moov").expect("a moov");
    let moov = MovieBox::parse(moov_bytes).expect("parse moov");
    let moov_item = parse_box(moov_bytes).expect("moov box").0;
    let trak_bytes: Vec<&[u8]> = BoxIter::new(moov_item.body)
        .filter_map(|b| b.ok())
        .filter(|b| b.four_cc() == *b"trak")
        .map(|b| b.bytes)
        .collect();
    assert_eq!(trak_bytes.len(), moov.tracks.len());

    let mut tracks = Vec::new();
    for (trak, bytes) in moov.tracks.iter().zip(&trak_bytes) {
        let mdia = trak.mdia.as_ref().expect("mdia");
        let mdhd = mdia.mdhd.as_ref().expect("mdhd");
        let hdlr = mdia.hdlr.as_ref().expect("hdlr");
        let stbl = mdia
            .minf
            .as_ref()
            .and_then(|m| m.stbl.as_ref())
            .expect("stbl");
        let stbl_bytes = child(child(child(bytes, b"mdia"), b"minf"), b"stbl");
        let (fourcc, body) = first_sample_entry(stbl_bytes);
        let track_id = TrackId::new(trak.tkhd.track_id.value());
        let mut setup = TrackSetup::new(
            track_id,
            hdlr.handler_type,
            hdlr.name.clone(),
            mdhd.timescale,
            SampleEntry::Raw { fourcc, body },
        );
        setup.width = trak.tkhd.width;
        setup.height = trak.tkhd.height;
        setup.volume = trak.tkhd.volume;
        if hdlr.handler_type != *b"vide" && hdlr.handler_type != *b"soun" {
            // ISO/IEC 14496-12:2026 §8.4.5.2: a track whose handler is not
            // audio or video carries the null media header.
            setup.minf_header = Some(*b"nmhd");
        }

        let mut cursor = TrackCursor::new(stbl, track_id, limits, Some(file.len() as u64))
            .expect("sample cursor");
        let samples: Vec<SourceSample> = cursor
            .by_ref()
            .map(|sample| {
                let sample = sample.expect("sample reference");
                let start = usize::try_from(sample.offset).expect("offset fits usize");
                let end = start + sample.size as usize;
                SourceSample {
                    reference: sample,
                    data: file[start..end].to_vec(),
                }
            })
            .collect();
        tracks.push(SourceTrack {
            setup,
            timescale: mdhd.timescale.get(),
            samples,
        });
    }
    tracks
}

fn write_fragmented(tracks: &[SourceTrack]) -> Vec<u8> {
    let movie_timescale = broadcast_common::timescale::Timescale::try_new(1000).expect("timescale");
    let setups: Vec<TrackSetup> = tracks.iter().map(|t| t.setup.clone()).collect();
    let mut writer: FragmentedWriter =
        Writer::fragmented(setups, movie_timescale, Limits::default()).expect("writer");
    let mut out = writer.init().expect("init segment");

    let max_second = tracks
        .iter()
        .flat_map(|t| {
            t.samples
                .iter()
                .map(move |s| s.reference.decode_time / u64::from(t.timescale))
        })
        .max()
        .unwrap_or(0);
    for second in 0..=max_second {
        let mut any = false;
        for track in tracks {
            let members: Vec<&SourceSample> = track
                .samples
                .iter()
                .filter(|s| s.reference.decode_time / u64::from(track.timescale) == second)
                .collect();
            if members.is_empty() {
                continue;
            }
            any = true;
            writer
                .set_base_decode_time(track.setup.track_id, members[0].reference.decode_time)
                .expect("base decode time");
            for sample in members {
                writer
                    .push(
                        track.setup.track_id,
                        SampleMeta {
                            duration: sample.reference.duration,
                            composition_offset: sample.reference.composition_offset,
                            sync: sample.reference.sync,
                            aux: None,
                            description_index: 1,
                        },
                        &sample.data,
                    )
                    .expect("push sample");
            }
        }
        if !any {
            continue;
        }
        let fragment = writer.flush().expect("flush").expect("a fragment");
        out.extend_from_slice(&fragment.bytes);
    }
    out
}

struct ReadSample {
    decode_time: u64,
    duration: u32,
    composition_offset: i64,
    sync: bool,
    data: Vec<u8>,
}

fn read_back(bytes: &[u8]) -> std::collections::BTreeMap<u32, Vec<ReadSample>> {
    let mdats = isobmff::frag_offsets::mdat_ranges(bytes);
    let budget = isobmff::frag_offsets::sample_bytes_budget(bytes.len() as u64);
    let mut copied = 0u64;
    let mut tracks: std::collections::BTreeMap<u32, Vec<ReadSample>> =
        std::collections::BTreeMap::new();
    for item in BoxIter::new(bytes).filter_map(|b| b.ok()) {
        if item.four_cc() != *b"moof" {
            continue;
        }
        let moof = MovieFragmentBox::parse_body(item.body).expect("parse moof");
        for traf in &moof.traf {
            let base = traf
                .tfdt
                .as_ref()
                .map(|t| t.base_media_decode_time())
                .unwrap_or(0);
            let ranges = isobmff::frag_offsets::sample_ranges_in_run(
                item.start as u64,
                &moof,
                traf.tfhd.track_id,
                &mdats,
                budget,
                &mut copied,
            )
            .expect("resolve sample ranges");
            let mut decode_time = base;
            let out = tracks.entry(traf.tfhd.track_id.value()).or_default();
            for range in ranges {
                out.push(ReadSample {
                    decode_time,
                    duration: range.duration,
                    composition_offset: range.composition_offset,
                    sync: range.flags & isobmff::movie_fragment::SAMPLE_FLAG_IS_NON_SYNC == 0,
                    data: bytes[usize::try_from(range.start).expect("start")
                        ..usize::try_from(range.end).expect("end")]
                        .to_vec(),
                });
                decode_time = decode_time
                    .checked_add(u64::from(range.duration))
                    .expect("decode time");
            }
        }
    }
    tracks
}

fn private_gopro() -> Option<std::path::PathBuf> {
    let Some(path) = test_fixtures::private("mp4/camera/gopro-karma.mp4") else {
        eprintln!(
            "SKIPPED: private/camera/gopro-karma.mp4 is unavailable (initialize the private submodule)"
        );
        return None;
    };
    Some(path)
}

#[test]
fn gopro_five_tracks_write_and_read_back() {
    let Some(path) = private_gopro() else {
        return;
    };
    let file = std::fs::read(&path).expect("read the GoPro capture");
    let tracks = parse_source(&file, &Limits::default());
    assert_eq!(tracks.len(), 5, "the capture carries five tracks");

    let fourccs: Vec<[u8; 4]> = tracks
        .iter()
        .map(|t| match &t.setup.entry {
            SampleEntry::Raw { fourcc, .. } => *fourcc,
            _ => panic!("every GoPro sample entry is kept raw"),
        })
        .collect();
    for expected in [*b"avc1", *b"mp4a", *b"tmcd", *b"gpmd", *b"fdsc"] {
        assert!(
            fourccs.contains(&expected),
            "the source carries {expected:?}"
        );
    }

    let out = write_fragmented(&tracks);
    let read = read_back(&out);
    assert_eq!(read.len(), 5, "every track is present in the output");

    for track in &tracks {
        let got = read
            .get(&track.setup.track_id.value())
            .expect("the track reads back");
        assert_eq!(got.len(), track.samples.len(), "sample count");
        for (got, source) in got.iter().zip(&track.samples) {
            assert_eq!(got.decode_time, source.reference.decode_time, "decode time");
            assert_eq!(got.duration, source.reference.duration, "duration");
            assert_eq!(
                got.composition_offset,
                i64::from(source.reference.composition_offset),
                "composition offset"
            );
            assert_eq!(got.sync, source.reference.sync, "sync flag");
            assert_eq!(got.data, source.data, "sample bytes");
        }
    }

    if !test_fixtures::process::tool_available("ffprobe", "-version") {
        eprintln!("ffprobe not on PATH: SKIPPED the five-stream check");
        return;
    }
    let temp =
        std::env::temp_dir().join(format!("isobmff-writer-gopro-{}.mp4", std::process::id()));
    std::fs::write(&temp, &out).expect("write the temporary output");
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_tag_string",
            "-of",
            "csv=p=0",
        ])
        .arg(&temp)
        .output()
        .expect("run ffprobe");
    let _ = std::fs::remove_file(&temp);
    assert!(output.status.success(), "ffprobe failed: {output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let tags: Vec<&str> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(tags.len(), 5, "ffprobe lists five streams: {tags:?}");
    for expected in ["avc1", "mp4a", "tmcd", "gpmd", "fdsc"] {
        assert!(
            tags.contains(&expected),
            "ffprobe must report {expected}: {tags:?}"
        );
    }
}

fn raw_video_setup() -> TrackSetup {
    TrackSetup::new(
        TrackId::new(1),
        *b"vide",
        b"VideoHandler\0".to_vec(),
        broadcast_common::timescale::Timescale::try_new(1000).expect("timescale"),
        SampleEntry::Raw {
            fourcc: *b"avc1",
            body: vec![0, 0, 0, 8, b'a', b'v', b'c', b'1'],
        },
    )
}

#[test]
fn a_fragment_over_the_cap_is_rejected_before_it_grows() {
    let limits = Limits::default().with_max_fragment_bytes(64);
    let mut writer = Writer::fragmented(
        vec![raw_video_setup()],
        broadcast_common::timescale::Timescale::try_new(1000).expect("timescale"),
        limits,
    )
    .expect("writer");
    let err = writer
        .push(
            TrackId::new(1),
            SampleMeta {
                duration: 1,
                composition_offset: 0,
                sync: true,
                aux: None,
                description_index: 1,
            },
            &[0u8; 128],
        )
        .expect_err("a 128-byte sample exceeds the 64-byte cap");
    assert_eq!(
        err,
        isobmff::Error::FragmentTooLarge { limit: 64 },
        "the push names the cap it would have exceeded"
    );
    assert!(
        writer.flush().expect("flush").is_none(),
        "nothing was buffered by the rejected push"
    );
}

#[test]
fn init_and_one_fragment_parse_back() {
    let mut writer = Writer::fragmented(
        vec![raw_video_setup()],
        broadcast_common::timescale::Timescale::try_new(1000).expect("timescale"),
        Limits::default(),
    )
    .expect("writer");
    let init = writer.init().expect("init segment");
    let moov = MovieBox::parse(find_box(&init, b"moov").expect("moov")).expect("parse moov");
    assert_eq!(moov.tracks.len(), 1);
    assert_eq!(moov.tracks[0].tkhd.track_id, TrackId::new(1));

    writer
        .set_base_decode_time(TrackId::new(1), 7)
        .expect("base decode time");
    writer
        .push(
            TrackId::new(1),
            SampleMeta {
                duration: 13,
                composition_offset: -2,
                sync: false,
                aux: None,
                description_index: 1,
            },
            &[1, 2, 3, 4],
        )
        .expect("push");
    let fragment = writer.flush().expect("flush").expect("a fragment");
    assert!(fragment.moof_len > 0);
    let body = find_box(&fragment.bytes, b"moof").expect("moof");
    let moof = MovieFragmentBox::parse_body(parse_box(body).expect("moof box").0.body)
        .expect("parse moof");
    assert_eq!(moof.traf.len(), 1);
    assert_eq!(moof.traf[0].tfhd.track_id, TrackId::new(1));
    assert_eq!(
        moof.traf[0]
            .tfdt
            .as_ref()
            .expect("tfdt")
            .base_media_decode_time(),
        7
    );
    let sample = &moof.traf[0].trun[0].samples[0];
    assert_eq!(sample.sample_duration, Some(13));
    assert_eq!(sample.sample_size, Some(4));
    assert_eq!(sample.sample_composition_time_offset, Some(-2));
}

/// A run of samples that use the second `stsd` entry states it once in
/// `tfhd.sample_description_index`, and a reader sees it on every sample
/// (fuzz finding on `isobmff_whole_file`: the writer dropped the field).
#[test]
fn a_second_description_entry_is_stated_in_the_tfhd() {
    let entry = SampleEntry::Raw {
        fourcc: *b"avc1",
        body: [0u8, 0, 0, 8, b'a', b'v', b'c', b'1'].to_vec(),
    };
    let mut setup = raw_video_setup();
    setup.entry = entry;
    let mut writer = Writer::fragmented(
        vec![setup],
        broadcast_common::timescale::Timescale::try_new(1000).expect("timescale"),
        Limits::default(),
    )
    .expect("writer");
    let mut out = writer.init().expect("init");
    writer
        .set_base_decode_time(TrackId::new(1), 0)
        .expect("base");
    let meta = SampleMeta {
        duration: 40,
        sync: true,
        description_index: 2,
        ..SampleMeta::default()
    };
    writer
        .push(TrackId::new(1), meta, &[0u8; 16])
        .expect("push");
    let fragment = writer.flush().expect("flush").expect("a fragment");
    out.extend_from_slice(&fragment.bytes);

    let mut reader = Reader::new(
        ReaderConfig::default()
            .with_total_len(Some(out.len() as u64))
            .with_order(SampleOrder::File),
    );
    let mut samples = Vec::new();
    loop {
        match reader.step().expect("step") {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let end = usize::try_from(range.end()).expect("end");
                reader
                    .supply(range.offset, &out[start..end])
                    .expect("supply");
            }
            Step::Sample(sample) => samples.push(sample),
            Step::Done => break,
            _ => {}
        }
    }
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].description_index, 2);
}

#[test]
fn a_caller_can_build_a_fragment_without_the_buffering_writer() {
    let meta = SampleMeta {
        duration: 10,
        composition_offset: 0,
        sync: true,
        aux: None,
        description_index: 1,
    };
    let samples = [
        isobmff::writer::SampleData {
            meta: &meta,
            data: &[9, 8, 7],
        },
        isobmff::writer::SampleData {
            meta: &meta,
            data: &[6, 5],
        },
    ];
    let tracks = [isobmff::writer::FragmentTrack {
        track_id: TrackId::new(3),
        base_decode_time: 0,
        samples: &samples,
        trex: Default::default(),
    }];
    let fragment = fragmented_segment(
        &tracks,
        &isobmff::writer::FragmentOptions {
            sequence_number: 4,
            styp: None,
            events: &[],
            layout: None,
            position: 0,
        },
    )
    .expect("fragment");
    let moof = MovieFragmentBox::parse_body(
        parse_box(find_box(&fragment.bytes, b"moof").expect("moof"))
            .expect("moof box")
            .0
            .body,
    )
    .expect("parse moof");
    assert_eq!(moof.mfhd.sequence_number, 4);
    assert_eq!(moof.traf[0].trun[0].samples.len(), 2);
    assert_eq!(fragment.bytes.len(), fragment.moof_len + 8 + 5);
}

#[test]
fn cenc_aux_emits_senc_saiz_saio_in_the_traf() {
    let aux = isobmff::writer::SampleAux {
        initialization_vector: vec![0xAA; 8],
        subsamples: vec![isobmff::cenc::SubSampleEntry {
            bytes_of_clear_data: 16,
            bytes_of_protected_data: 32,
        }],
    };
    let meta = SampleMeta {
        duration: 1,
        composition_offset: 0,
        sync: true,
        aux: Some(aux),
        description_index: 1,
    };
    let samples = [isobmff::writer::SampleData {
        meta: &meta,
        data: &[1, 2, 3, 4],
    }];
    let tracks = [isobmff::writer::FragmentTrack {
        track_id: TrackId::new(1),
        base_decode_time: 0,
        samples: &samples,
        trex: Default::default(),
    }];
    let fragment = fragmented_segment(
        &tracks,
        &isobmff::writer::FragmentOptions {
            sequence_number: 1,
            styp: None,
            events: &[],
            layout: None,
            position: 0,
        },
    )
    .expect("fragment");
    let moof = MovieFragmentBox::parse_body(
        parse_box(find_box(&fragment.bytes, b"moof").expect("moof"))
            .expect("moof box")
            .0
            .body,
    )
    .expect("parse moof");
    let opaque: Vec<[u8; 4]> = moof.traf[0]
        .order
        .iter()
        .filter_map(|child| match child {
            isobmff::movie_fragment::TrafChild::Opaque(o) => Some(o.box_type),
            _ => None,
        })
        .collect();
    assert_eq!(opaque, vec![*b"senc", *b"saiz", *b"saio"]);
}

/// A three-sample, one-track fragment as this crate writes it, plus the layout
/// the reader records for it.
fn canonical_fragment() -> (Vec<u8>, isobmff::frag_layout::FragmentLayout) {
    let metas = [
        SampleMeta {
            duration: 1_000,
            composition_offset: 0,
            sync: true,
            aux: None,
            description_index: 1,
        },
        SampleMeta {
            duration: 1_000,
            composition_offset: 40,
            sync: false,
            aux: None,
            description_index: 1,
        },
        SampleMeta {
            duration: 1_000,
            composition_offset: 0,
            sync: false,
            aux: None,
            description_index: 1,
        },
    ];
    let samples: Vec<isobmff::writer::SampleData<'_>> = metas
        .iter()
        .map(|meta| isobmff::writer::SampleData {
            meta,
            data: &[0u8; 4],
        })
        .collect();
    let tracks = [isobmff::writer::FragmentTrack {
        track_id: TrackId::new(1),
        base_decode_time: 0,
        samples: &samples,
        trex: Default::default(),
    }];
    let fragment = fragmented_segment(
        &tracks,
        &isobmff::writer::FragmentOptions {
            sequence_number: 2,
            styp: None,
            events: &[],
            layout: None,
            position: 0,
        },
    )
    .expect("canonical fragment");
    let moof = parse_moof(&fragment.bytes);
    let layout =
        isobmff::frag_layout::FragmentLayout::from_moof(&moof, 0).expect("recorded layout");
    (fragment.bytes[..fragment.moof_len].to_vec(), layout)
}

fn parse_moof(bytes: &[u8]) -> MovieFragmentBox {
    let found = find_box(bytes, b"moof").expect("a moof");
    MovieFragmentBox::parse_body(parse_box(found).expect("moof box").0.body).expect("parse moof")
}

/// The fragment a written layout produces at `position`, as `moof` bytes.
fn write_with_layout(
    layout: &isobmff::frag_layout::FragmentLayout,
    position: u64,
) -> (
    Vec<u8>,
    isobmff::writer::Fragment,
    isobmff::frag_layout::FragmentLayout,
) {
    let metas = [
        SampleMeta {
            duration: 1_000,
            composition_offset: 0,
            sync: true,
            aux: None,
            description_index: 1,
        },
        SampleMeta {
            duration: 1_000,
            composition_offset: 40,
            sync: false,
            aux: None,
            description_index: 1,
        },
        SampleMeta {
            duration: 1_000,
            composition_offset: 0,
            sync: false,
            aux: None,
            description_index: 1,
        },
    ];
    let samples: Vec<isobmff::writer::SampleData<'_>> = metas
        .iter()
        .map(|meta| isobmff::writer::SampleData {
            meta,
            data: &[0u8; 4],
        })
        .collect();
    let tracks = [isobmff::writer::FragmentTrack {
        track_id: TrackId::new(1),
        base_decode_time: 0,
        samples: &samples,
        trex: Default::default(),
    }];
    let fragment = fragmented_segment(
        &tracks,
        &isobmff::writer::FragmentOptions {
            sequence_number: 2,
            styp: None,
            events: &[],
            layout: Some(layout),
            position,
        },
    )
    .expect("layout fragment");
    let moof = parse_moof(&fragment.bytes);
    let recorded = isobmff::frag_layout::FragmentLayout::from_moof(&moof, fragment.moof_offset)
        .expect("recorded layout");
    (
        fragment.bytes[..fragment.moof_len].to_vec(),
        fragment,
        recorded,
    )
}

/// A recorded layout states exactly what it was recorded from, so the writer
/// reproduces the fragment byte for byte.
#[test]
fn a_recorded_layout_reproduces_the_fragment_it_was_read_from() {
    let (canonical, layout) = canonical_fragment();
    let (rewritten, fragment, _) = write_with_layout(&layout, 0);
    assert_eq!(fragment.moof_offset, 0);
    assert_eq!(
        rewritten, canonical,
        "the moof round-trips through its layout"
    );
}

/// A layout that leaves a per-sample field to a `tfhd` default the samples
/// contradict is refused, naming the field and the sample.
#[test]
fn a_layout_default_the_samples_contradict_is_refused() {
    let (_, mut layout) = canonical_fragment();
    let track = &mut layout.tracks[0];
    track.tfhd.default_sample_duration = Some(999);
    track.runs[0].fields.duration = false;
    let metas = [SampleMeta {
        duration: 1_000,
        composition_offset: 0,
        sync: true,
        aux: None,
        description_index: 1,
    }];
    let samples = [isobmff::writer::SampleData {
        meta: &metas[0],
        data: &[0u8; 4],
    }];
    let tracks = [isobmff::writer::FragmentTrack {
        track_id: TrackId::new(1),
        base_decode_time: 0,
        samples: &samples,
        trex: Default::default(),
    }];
    let err = fragmented_segment(
        &tracks,
        &isobmff::writer::FragmentOptions {
            sequence_number: 1,
            styp: None,
            events: &[],
            layout: Some(&layout),
            position: 0,
        },
    )
    .expect_err("a contradicted default is refused");
    match err {
        isobmff::Error::LayoutMismatch {
            field,
            sample,
            expected,
            actual,
            ..
        } => {
            assert_eq!(field, "sample_count");
            assert_eq!(sample, 3);
            assert_eq!(expected, 3);
            assert_eq!(actual, 1);
        }
        other => panic!("expected LayoutMismatch, got {other:?}"),
    }
}

/// The same three samples, one at a time against a `tfhd` default they
/// contradict, are refused on the sample that disagrees.
#[test]
fn a_layout_default_is_checked_against_every_sample() {
    let (_, mut layout) = canonical_fragment();
    let track = &mut layout.tracks[0];
    track.runs[0].fields.duration = false;
    track.tfhd.default_sample_duration = Some(1_000);
    let metas = [
        SampleMeta {
            duration: 1_000,
            composition_offset: 0,
            sync: true,
            aux: None,
            description_index: 1,
        },
        SampleMeta {
            duration: 1_001,
            composition_offset: 0,
            sync: true,
            aux: None,
            description_index: 1,
        },
        SampleMeta {
            duration: 1_000,
            composition_offset: 0,
            sync: true,
            aux: None,
            description_index: 1,
        },
    ];
    let samples: Vec<isobmff::writer::SampleData<'_>> = metas
        .iter()
        .map(|meta| isobmff::writer::SampleData {
            meta,
            data: &[0u8; 4],
        })
        .collect();
    let tracks = [isobmff::writer::FragmentTrack {
        track_id: TrackId::new(1),
        base_decode_time: 0,
        samples: &samples,
        trex: Default::default(),
    }];
    let err = fragmented_segment(
        &tracks,
        &isobmff::writer::FragmentOptions {
            sequence_number: 1,
            styp: None,
            events: &[],
            layout: Some(&layout),
            position: 0,
        },
    )
    .expect_err("the second sample disagrees with the default");
    match err {
        isobmff::Error::LayoutMismatch {
            field,
            sample,
            expected,
            actual,
            ..
        } => {
            assert_eq!(field, "sample_duration");
            assert_eq!(sample, 1);
            assert_eq!(expected, 1_000);
            assert_eq!(actual, 1_001);
        }
        other => panic!("expected LayoutMismatch, got {other:?}"),
    }
}

/// A layout that states an absolute `base_data_offset` gets it recomputed for
/// where the writer places the fragment, and each run follows from it.
#[test]
fn an_absolute_base_data_offset_is_recomputed_for_the_new_position() {
    let (_, mut layout) = canonical_fragment();
    let track = &mut layout.tracks[0];
    track.tfhd.base_data_offset = Some(0);
    track.tfhd.flags &= !isobmff::movie_fragment::TFHD_DEFAULT_BASE_IS_MOOF;
    let position = 5_000;
    let (_, fragment, recorded) = write_with_layout(&layout, position);
    assert_eq!(
        recorded.tracks[0].tfhd.base_data_offset,
        Some(0),
        "the recorded base is the moof's own first byte"
    );
    let moof = parse_moof(&fragment.bytes);
    assert_eq!(
        moof.traf[0].tfhd.base_data_offset,
        Some(position),
        "the written base is the absolute position"
    );
    assert_eq!(
        moof.traf[0].trun[0].data_offset,
        Some(i32::try_from(fragment.moof_len + 8).expect("fits")),
        "the run still starts at the mdat payload"
    );
}

/// A layout that splits a track's samples into two runs writes two runs, with
/// each run's `data_offset` at its own first sample.
#[test]
fn a_layout_run_split_is_written_as_it_states_it() {
    let (_, mut layout) = canonical_fragment();
    let track = &mut layout.tracks[0];
    let whole = track.runs[0].clone();
    let first = isobmff::frag_layout::RunLayout {
        sample_count: 1,
        ..whole.clone()
    };
    let rest = isobmff::frag_layout::RunLayout {
        sample_count: 2,
        first_sample_flags: None,
        ..whole
    };
    track.runs = vec![first, rest];
    track.children = vec![
        isobmff::movie_fragment::TrafChild::Tfhd,
        isobmff::movie_fragment::TrafChild::Tfdt,
        isobmff::movie_fragment::TrafChild::Trun(0),
        isobmff::movie_fragment::TrafChild::Trun(1),
    ];
    let (_, fragment, _) = write_with_layout(&layout, 0);
    let moof = parse_moof(&fragment.bytes);
    assert_eq!(moof.traf[0].trun.len(), 2);
    assert_eq!(moof.traf[0].trun[0].samples.len(), 1);
    assert_eq!(moof.traf[0].trun[1].samples.len(), 2);
    let first_offset = moof.traf[0].trun[0].data_offset.expect("stated");
    let second_offset = moof.traf[0].trun[1].data_offset.expect("stated");
    assert_eq!(
        second_offset,
        first_offset + 4,
        "the second run starts after the first run's one 4-byte sample"
    );
}

/// An omitted per-sample field must be backed by a `tfhd` default: the flags
/// word carries no presence bit for it.
#[test]
fn a_layout_omits_the_fields_it_leaves_to_a_tfhd_default() {
    let (_, mut layout) = canonical_fragment();
    let track = &mut layout.tracks[0];
    track.tfhd.default_sample_duration = Some(1_000);
    track.tfhd.default_sample_size = Some(4);
    track.runs[0].fields.duration = false;
    track.runs[0].fields.size = false;
    let (_, _, recorded) = write_with_layout(&layout, 0);
    assert!(!recorded.tracks[0].runs[0].fields.duration);
    assert!(!recorded.tracks[0].runs[0].fields.size);
    assert!(recorded.tracks[0].runs[0].fields.composition_offset);
    assert!(recorded.tracks[0].runs[0].fields.flags);

    // Moving the duration from every record into the `tfhd` shrinks the moof
    // by three 4-byte records and grows the header by one 4-byte default.
    let (_, mut stripped) = canonical_fragment();
    stripped.tracks[0].tfhd.default_sample_duration = Some(1_000);
    stripped.tracks[0].runs[0].fields.duration = false;
    let (_, small, _) = write_with_layout(&stripped, 0);
    let (canonical, _) = canonical_fragment();
    let word = core::mem::size_of::<u32>();
    assert_eq!(
        small.moof_len + 2 * word,
        canonical.len(),
        "three per-sample durations out, one `tfhd` default in, eight bytes net"
    );
    assert_eq!(
        stripped.tracks[0].runs[0].fields.flags()
            & isobmff::movie_fragment::TRUN_SAMPLE_DURATION_PRESENT,
        0,
        "the `trun` no longer states a per-sample duration"
    );
}

/// The absolute offsets of every top-level `moof` in `bytes`.
fn moof_offsets(bytes: &[u8]) -> Vec<u64> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    while offset + 8 <= bytes.len() {
        let size = u32::from_be_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        if &bytes[offset + 4..offset + 8] == b"moof" {
            out.push(u64::try_from(offset).expect("offset"));
        }
        if size < 8 {
            break;
        }
        offset += size;
    }
    out
}

/// A one-track fragmented file of `fragments` flushes, each with `per`
/// samples: every sample the same size and duration, the first of each
/// fragment the only sync sample.
///
/// A `sidx` is placed immediately before the first fragment, which is where a
/// caller that streams the fragments out and assembles the file afterwards
/// puts it.
fn sample_file(fragments: u32, per: usize, compact: bool, mfra: bool, sidx: bool) -> Vec<u8> {
    let mut writer = Writer::fragmented(
        vec![raw_video_setup()],
        broadcast_common::timescale::Timescale::try_new(1_000).expect("timescale"),
        Limits::default(),
    )
    .expect("fragmented writer");
    writer.set_compact_layout(compact);
    writer.with_mfra(mfra);
    writer.with_sidx(sidx);
    let mut out = writer.init().expect("init segment");
    writer.set_position(u64::try_from(out.len()).expect("init length"));
    let mut body = Vec::new();
    for fragment in 0..fragments {
        let base = u64::from(fragment) * u64::try_from(per).expect("per") * 40;
        writer
            .set_base_decode_time(TrackId::new(1), base)
            .expect("base decode time");
        for sample in 0..per {
            let sync = sample == 0;
            writer
                .push(
                    TrackId::new(1),
                    SampleMeta {
                        duration: 40,
                        composition_offset: if sample == 1 { 400 } else { 0 },
                        sync,
                        aux: None,
                        description_index: 1,
                    },
                    &[0u8; 8],
                )
                .expect("push");
        }
        let flushed = writer.flush().expect("flush").expect("a fragment");
        body.extend_from_slice(&flushed.bytes);
    }
    if sidx {
        let index = writer
            .segment_index()
            .expect("index")
            .expect("the writer was asked for one");
        out.extend_from_slice(&index);
    }
    out.extend_from_slice(&body);
    if mfra {
        writer.set_position(u64::try_from(out.len()).expect("body length"));
        out.extend_from_slice(&writer.trailer().expect("trailer"));
    }
    out
}

/// The compact layout describes the same samples with fewer bytes: the moof is
/// smaller and the reader reads it back to identical samples.
#[test]
fn a_compact_layout_is_smaller_and_reads_back_to_the_same_samples() {
    let canonical = sample_file(2, 4, false, false, false);
    let compact = sample_file(2, 4, true, false, false);
    assert!(
        compact.len() < canonical.len(),
        "the compact file is {} bytes, the canonical one {}",
        compact.len(),
        canonical.len()
    );
    let canonical_samples = read_with_reader(&canonical);
    let compact_samples = read_with_reader(&compact);
    assert_eq!(compact_samples, canonical_samples);
}

/// Every sample a [`Reader`] run yields, in file order, as the descriptor
/// facts the reader reports plus the coded bytes they address.
fn read_with_reader(bytes: &[u8]) -> Vec<(u32, u64, u32, i32, bool, Vec<u8>)> {
    let mut reader = Reader::new(
        ReaderConfig::default()
            .with_total_len(Some(u64::try_from(bytes.len()).expect("length")))
            .with_order(SampleOrder::File),
    );
    let mut out = Vec::new();
    loop {
        match reader.step().expect("step") {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let end = usize::try_from(range.end()).expect("end").min(bytes.len());
                reader
                    .supply(range.offset, &bytes[start..end])
                    .expect("supply");
            }
            Step::Sample(sample) => {
                let start = usize::try_from(sample.offset).expect("offset");
                let end = start + usize::try_from(sample.size).expect("size");
                out.push((
                    sample.track.value(),
                    sample.decode_time,
                    sample.duration,
                    sample.composition_offset,
                    sample.sync,
                    bytes[start..end].to_vec(),
                ));
            }
            Step::Done => break,
            _ => {}
        }
    }
    out
}

/// The regenerated `mfra` addresses the fragments the writer placed, and a
/// seek through it lands on each of their `moof`s.
#[test]
fn the_regenerated_mfra_addresses_the_written_moofs() {
    let bytes = sample_file(3, 4, false, true, false);
    let offsets = moof_offsets(&bytes);
    assert_eq!(offsets.len(), 3);
    let mfra = isobmff::BoxIter::clamped(&bytes)
        .filter_map(|step| step.ok())
        .find(|step| step.header.box_type.is(b"mfra"))
        .expect("the writer emitted an mfra");
    let parsed =
        isobmff::timing::MovieFragmentRandomAccessBox::parse(mfra.bytes).expect("mfra parses");
    let entries = &parsed.tfra[0].entries;
    assert_eq!(entries.len(), offsets.len(), "one entry per fragment");
    for (entry, offset) in entries.iter().zip(&offsets) {
        assert_eq!(entry.moof_offset, *offset, "the entry addresses its moof");
        assert_eq!(entry.traf_number, 1);
        assert_eq!(entry.trun_number, 1);
        assert_eq!(entry.sample_number, 1);
    }
    assert_eq!(
        entries[0].time, 0,
        "the first fragment's sync sample is at its base decode time"
    );
    assert_eq!(entries[1].time, 160, "the second fragment starts 160 in");
    assert_eq!(
        parsed.mfro.parent_size,
        u32::try_from(mfra.end - mfra.start).expect("mfra length"),
        "the mfro states the mfra's own size"
    );

    for (index, entry) in entries.iter().enumerate() {
        let landed = seek_to_moof(&bytes, TrackId::new(1), entry.time);
        assert_eq!(
            landed, offsets[index],
            "a seek to {} lands on fragment {index}",
            entry.time
        );
    }
}

/// The regenerated `sidx` indexes the written fragments: one reference each,
/// sized from the offsets the writer placed them at.
#[test]
fn the_regenerated_sidx_indexes_the_written_fragments() {
    let bytes = sample_file(3, 4, false, false, true);
    let offsets = moof_offsets(&bytes);
    let sidx = isobmff::BoxIter::clamped(&bytes)
        .filter_map(|step| step.ok())
        .find(|step| step.header.box_type.is(b"sidx"))
        .expect("the writer emitted a sidx");
    let index = <isobmff::timing::SegmentIndexBox as broadcast_common::Parse>::parse(sidx.bytes)
        .expect("sidx parses");
    assert_eq!(index.references.len(), offsets.len());
    assert_eq!(index.reference_id, 1, "the first track's id");
    assert_eq!(index.timescale.get(), 1_000, "the first track's timescale");
    for (position, reference) in index.references.iter().enumerate() {
        let start = offsets[position];
        let end = offsets
            .get(position + 1)
            .copied()
            .unwrap_or(u64::try_from(bytes.len()).expect("length"));
        assert_eq!(
            u64::from(reference.referenced_size),
            end - start,
            "reference {position} spans its fragment"
        );
        assert_eq!(reference.subsegment_duration, 4 * 40);
        assert_eq!(reference.starts_with_sap, 1);
        assert_eq!(reference.sap_type, 1);
    }
}

/// Drive a reader to the fragment a seek names.
fn seek_to_moof(bytes: &[u8], track: TrackId, time: u64) -> u64 {
    let mut reader = Reader::new(
        ReaderConfig::default()
            .with_total_len(Some(u64::try_from(bytes.len()).expect("length")))
            .with_order(SampleOrder::File),
    );
    let mut seeked = false;
    loop {
        match reader.step().expect("step") {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let end = usize::try_from(range.end()).expect("end").min(bytes.len());
                reader
                    .supply(range.offset, &bytes[start..end])
                    .expect("supply");
            }
            Step::Fragment(info) => return info.moof.offset,
            Step::Done => panic!("the seek found no fragment"),
            _ => {}
        }
        if !seeked && reader.movie_timescale().is_some() {
            reader.seek_to(track, time).expect("seek");
            seeked = true;
        }
    }
}

/// The boxes between the last fragment's media and this one are carried in the
/// layout and written ahead of the `styp`/`moof`.
#[test]
fn a_layout_carries_the_boxes_between_fragments() {
    let filler = isobmff::box_support::OpaqueBox::new(*b"free", vec![0u8; 8]);
    let (_, mut layout) = canonical_fragment();
    layout.before = vec![filler.clone()];
    let (_, fragment, _) = write_with_layout(&layout, 0);
    let expected = filler.try_to_bytes().expect("free box bytes");
    assert_eq!(
        &fragment.bytes[..expected.len()],
        expected.as_slice(),
        "the carried box precedes the moof"
    );
    assert_eq!(
        &fragment.bytes[expected.len() + 4..expected.len() + 8],
        b"moof",
        "the moof follows the carried box"
    );
    assert_eq!(
        fragment.moof_offset,
        u64::try_from(expected.len()).expect("length"),
        "the moof starts after the carried box"
    );
}

/// The reader records a box that sits between two fragments in the layout of
/// the fragment it precedes, so the rewrite can put it back.
#[test]
fn the_reader_records_a_box_between_two_fragments() {
    let plain = sample_file(2, 2, false, false, false);
    let offsets = moof_offsets(&plain);
    let second = usize::try_from(offsets[1]).expect("offset");
    let filler = isobmff::box_support::OpaqueBox::new(*b"free", vec![0u8; 8]);
    let bytes = filler.try_to_bytes().expect("free box bytes");
    let mut with_free = plain[..second].to_vec();
    with_free.extend_from_slice(&bytes);
    with_free.extend_from_slice(&plain[second..]);

    let mut reader = Reader::new(
        ReaderConfig::default()
            .with_total_len(Some(u64::try_from(with_free.len()).expect("length")))
            .with_order(SampleOrder::File),
    );
    let mut carried: Vec<usize> = Vec::new();
    loop {
        match reader.step().expect("step") {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let end = usize::try_from(range.end())
                    .expect("end")
                    .min(with_free.len());
                reader
                    .supply(range.offset, &with_free[start..end])
                    .expect("supply");
            }
            Step::Fragment(info) => {
                carried.extend(info.layout.before.iter().map(|box_| box_.data.len()));
                if info.layout.tracks.is_empty() {
                    panic!("a fragment recorded no track layout");
                }
            }
            Step::Done => break,
            _ => {}
        }
    }
    assert_eq!(
        carried,
        vec![8],
        "only the second fragment carries the box before it"
    );
}
