//! The fragmented writer: init segment, `moof`+`mdat`, and the round trip of
//! every track of a real GoPro capture (five tracks, one of them a raw sample
//! entry with the `meta` handler).
//!
//! The source is walked with the box layer (`MovieBox`, the `stsd` entry
//! bytes and `TrackCursor`), the result is written with
//! [`isobmff::writer::FragmentedWriter`], and read back through
//! `MovieFragmentBox` + `frag_offsets` so every sample's bytes, duration, sync
//! flag and composition offset is compared with the source.

use broadcast_common::Parse;
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
    }];
    let fragment = fragmented_segment(
        &tracks,
        &isobmff::writer::FragmentOptions {
            sequence_number: 4,
            styp: None,
            events: &[],
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
    }];
    let fragment = fragmented_segment(
        &tracks,
        &isobmff::writer::FragmentOptions {
            sequence_number: 1,
            styp: None,
            events: &[],
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
