//! Authoring a `.mov` from typed values: a video track copied from a public
//! progressive fixture, a QuickTime version 1 `wave` audio track, and a `tmcd`
//! timecode track with a typed `gmhd`.

use std::collections::BTreeMap;
use std::process::Command;

use broadcast_common::Serialize;
use isobmff::box_support::OpaqueBox;
use isobmff::init_segment::{Mp4aSampleEntry, SampleEntryVariant};
use isobmff::plan::{PlanBuilder, PlanSample};
use isobmff::quicktime::{
    GmhdAtom, GminAtom, QuickTimeSound, SoundDescriptionV1, TcmiAtom, TimecodeEntry, WaveAtom,
    timecode_from_sample,
};
use isobmff::sample_table::SampleRef;
use isobmff::writer::{
    FileType, Interleave, Layout, MovieHeader, SampleEntry, Sink, TrackSetup, Write, Writer,
};
use isobmff::{Limits, Reader, ReaderConfig, SampleOrder, Step, TrackId, TrackInfo};

struct Run {
    tracks: Vec<TrackInfo>,
    samples: Vec<SampleRef>,
}

fn drive(data: &[u8]) -> Run {
    let mut reader = Reader::new(
        ReaderConfig::default()
            .with_total_len(Some(u64::try_from(data.len()).expect("length")))
            .with_order(SampleOrder::File),
    );
    let mut tracks = Vec::new();
    let mut samples = Vec::new();
    loop {
        match reader.step().expect("reader step") {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let end = usize::try_from(range.end()).expect("end").min(data.len());
                reader
                    .supply(range.offset, &data[start..end])
                    .expect("supply");
            }
            Step::Tracks(step_tracks) => tracks = step_tracks.to_vec(),
            Step::Sample(sample) => samples.push(sample),
            Step::Done => break,
            _ => {}
        }
    }
    Run { tracks, samples }
}

fn sample_bytes<'a>(data: &'a [u8], sample: &SampleRef) -> &'a [u8] {
    let start = usize::try_from(sample.offset).expect("offset");
    let end = start + usize::try_from(sample.size).expect("size");
    &data[start..end]
}

fn place(out: &mut [u8], write: &Write<'_>) {
    let start = usize::try_from(write.offset).expect("offset");
    let end = start + write.bytes.len();
    out[start..end].copy_from_slice(&write.bytes);
}

fn audio_entry(mp4a: &Mp4aSampleEntry) -> SampleEntry {
    let esds = mp4a.esds().expect("wave esds");
    let wave = WaveAtom {
        format: *b"mp4a",
        esds: Some(esds),
        others: Vec::new(),
        order: Vec::new(),
    };
    let wave_bytes = wave.try_to_bytes().expect("wave bytes");
    let wave_child = OpaqueBox {
        box_type: *b"wave",
        data: wave_bytes[isobmff::box_types::BOX_HEADER_MIN_SIZE..].to_vec(),
        to_end: false,
        largesize: false,
    };
    let entry = Mp4aSampleEntry {
        codec_type: *b"mp4a",
        entry_version: 1,
        reserved_1: [0; 6],
        data_reference_index: mp4a.data_reference_index,
        channelcount: mp4a.channelcount,
        samplesize: mp4a.samplesize,
        compression_id_and_packet_size: mp4a.compression_id_and_packet_size,
        samplerate: mp4a.samplerate,
        quicktime: Some(QuickTimeSound::V1(SoundDescriptionV1 {
            samples_per_packet: 1024,
            bytes_per_packet: 0,
            bytes_per_frame: 0,
            bytes_per_sample: 2,
        })),
        config_boxes: vec![wave_child],
    };
    SampleEntry::Typed(SampleEntryVariant::Mp4a(Box::new(entry)))
}

fn timecode_entry() -> TimecodeEntry {
    TimecodeEntry {
        reserved: [0; 6],
        data_reference_index: 1,
        reserved_1: 0,
        flags: isobmff::quicktime::timecode::COUNTER_FLAG,
        timescale: 12800,
        frame_duration: 512,
        number_of_frames: 25,
        reserved_tail: vec![0, 0, 0],
        name: None,
        others: Vec::new(),
    }
}

fn gmhd() -> GmhdAtom {
    GmhdAtom {
        gmin: Some(GminAtom {
            version: 0,
            flags: 0,
            graphics_mode: 0x40,
            opcolor: [0x8000, 0x8000, 0x8000],
            balance: 0,
            reserved: 0,
        }),
        text: None,
        timecode: Some(TcmiAtom {
            version: 0,
            flags: 0,
            text_font: 0,
            text_face: 0,
            text_size: 12,
            text_color: [0, 0, 0],
            background_color: [0xffff, 0xffff, 0xffff],
            reserved: 0,
            font_name: b"Helvetica".to_vec(),
        }),
        timecode_wrapped: true,
        others: Vec::new(),
        order: Vec::new(),
    }
}

#[test]
fn a_mov_is_authored_from_typed_values() {
    let video_src = test_fixtures::read("mp4/progressive/av-prog.mp4");
    let Some(mov_path) = test_fixtures::require("mp4/quicktime/lavf-aac-tmcd.mov", "quicktime")
    else {
        eprintln!("SKIP quicktime_write: private /mp4/quicktime/lavf-aac-tmcd.mov missing");
        return;
    };
    let mov_src = std::fs::read(&mov_path).expect("mov bytes");
    let video = drive(&video_src);
    let mov = drive(&mov_src);

    let video_track = video
        .tracks
        .iter()
        .find(|track| track.handler == *b"vide")
        .expect("video track");
    let audio_track = mov
        .tracks
        .iter()
        .find(|track| track.handler == *b"soun")
        .expect("audio track");
    let video_samples: Vec<SampleRef> = video
        .samples
        .iter()
        .filter(|sample| sample.track == video_track.track_id)
        .cloned()
        .collect();
    let audio_samples: Vec<SampleRef> = mov
        .samples
        .iter()
        .filter(|sample| sample.track == audio_track.track_id)
        .cloned()
        .collect();
    assert!(!video_samples.is_empty() && !audio_samples.is_empty());

    let SampleEntryVariant::Mp4a(mp4a) = audio_track.entries[0]
        .typed
        .as_ref()
        .expect("typed audio entry")
    else {
        panic!("audio entry is not mp4a");
    };
    let SampleEntryVariant::Avc1(video_entry) = video_track.entries[0]
        .typed
        .clone()
        .expect("typed video entry")
    else {
        panic!("video entry is not avc1");
    };

    let video_id = TrackId::new(1);
    let audio_id = TrackId::new(2);
    let timecode_id = TrackId::new(3);

    let mut video_setup = TrackSetup::new(
        video_id,
        *b"vide",
        b"VideoHandler\0".to_vec(),
        video_track.timescale,
        SampleEntry::Typed(SampleEntryVariant::Avc1(video_entry)),
    );
    video_setup.width = video_track.header.width;
    video_setup.height = video_track.header.height;
    video_setup.references = vec![isobmff::track_reference::TrackReferenceTypeBox {
        reference_type: *b"tmcd",
        track_ids: vec![timecode_id],
    }];

    let mut audio_setup = TrackSetup::new(
        audio_id,
        *b"soun",
        b"SoundHandler\0".to_vec(),
        audio_track.timescale,
        audio_entry(mp4a),
    );
    audio_setup.volume = 0x0100;

    let mut timecode_setup = TrackSetup::new(
        timecode_id,
        *b"tmcd",
        b"TimecodeHandler\0".to_vec(),
        broadcast_common::timescale!(12800),
        SampleEntry::Typed(SampleEntryVariant::Tmcd(Box::new(timecode_entry()))),
    );
    timecode_setup.gmhd = Some(gmhd());

    let mut builder = PlanBuilder::new(
        vec![video_setup, audio_setup, timecode_setup],
        Limits::default(),
    );
    let mut expected: BTreeMap<(u32, u32), Vec<u8>> = BTreeMap::new();
    for (index, sample) in video_samples.iter().enumerate() {
        let index = u32::try_from(index).expect("sample index");
        builder
            .push(
                video_id,
                PlanSample {
                    duration: sample.duration,
                    composition_offset: sample.composition_offset,
                    sync: sample.sync,
                    size: sample.size,
                    description_index: 1,
                },
            )
            .expect("video sample");
        expected.insert(
            (video_id.value(), index),
            sample_bytes(&video_src, sample).to_vec(),
        );
    }
    for (index, sample) in audio_samples.iter().enumerate() {
        let index = u32::try_from(index).expect("sample index");
        builder
            .push(
                audio_id,
                PlanSample {
                    duration: sample.duration,
                    composition_offset: sample.composition_offset,
                    sync: sample.sync,
                    size: sample.size,
                    description_index: 1,
                },
            )
            .expect("audio sample");
        expected.insert(
            (audio_id.value(), index),
            sample_bytes(&mov_src, sample).to_vec(),
        );
    }
    let timecode_sample = 900_000u32.to_be_bytes();
    builder
        .push(
            timecode_id,
            PlanSample {
                duration: 38400,
                composition_offset: 0,
                sync: true,
                size: 4,
                description_index: 1,
            },
        )
        .expect("timecode sample");
    expected.insert((timecode_id.value(), 0), timecode_sample.to_vec());

    let video_duration: u64 = video_samples.iter().map(|s| u64::from(s.duration)).sum();
    let audio_duration: u64 = audio_samples.iter().map(|s| u64::from(s.duration)).sum();
    builder
        .set_durations(video_id, video_duration, video_duration)
        .expect("video durations");
    builder
        .set_durations(audio_id, audio_duration, audio_duration)
        .expect("audio durations");
    builder
        .set_durations(timecode_id, 38400, 38400)
        .expect("timecode durations");
    let plan = builder.finish().expect("plan");

    let mut writer = Writer::progressive(
        plan,
        Layout::new(
            false,
            Interleave::ByDuration { millis: 500 },
            Sink::Seekable,
        ),
        FileType::quicktime(),
        MovieHeader::new(broadcast_common::timescale!(1000)),
        Limits::default(),
    )
    .expect("progressive writer");
    let mut out = vec![0u8; usize::try_from(writer.file_len()).expect("file length")];
    for write in writer.header() {
        place(&mut out, &write);
    }
    for (index, sample) in video_samples.iter().enumerate() {
        let index = u32::try_from(index).expect("sample index");
        for write in writer
            .push(video_id, index, sample_bytes(&video_src, sample))
            .expect("push video")
        {
            place(&mut out, &write);
        }
    }
    for (index, sample) in audio_samples.iter().enumerate() {
        let index = u32::try_from(index).expect("sample index");
        for write in writer
            .push(audio_id, index, sample_bytes(&mov_src, sample))
            .expect("push audio")
        {
            place(&mut out, &write);
        }
    }
    for write in writer
        .push(timecode_id, 0, &timecode_sample)
        .expect("push timecode")
    {
        place(&mut out, &write);
    }
    for write in writer.finish().expect("finish") {
        place(&mut out, &write);
    }

    let read_back = drive(&out);
    assert_eq!(read_back.tracks.len(), 3, "three tracks");
    let handlers: Vec<[u8; 4]> = read_back.tracks.iter().map(|t| t.handler).collect();
    assert!(handlers.contains(b"vide"));
    assert!(handlers.contains(b"soun"));
    assert!(handlers.contains(b"tmcd"));

    let mut reader = Reader::new(
        ReaderConfig::default()
            .with_total_len(Some(u64::try_from(out.len()).expect("length")))
            .with_order(SampleOrder::File),
    );
    loop {
        match reader.step().expect("brand step") {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let end = usize::try_from(range.end()).expect("end").min(out.len());
                reader
                    .supply(range.offset, &out[start..end])
                    .expect("supply");
            }
            Step::Done => break,
            _ => {}
        }
    }
    assert_eq!(
        reader.brand().expect("ftyp").major_brand,
        *b"qt  ",
        "QuickTime brand"
    );

    let timecode_track = read_back
        .tracks
        .iter()
        .find(|track| track.handler == *b"tmcd")
        .expect("timecode track");
    let SampleEntryVariant::Tmcd(tmcd) = timecode_track.entries[0]
        .typed
        .as_ref()
        .expect("typed tmcd entry")
    else {
        panic!("timecode entry is not tmcd");
    };
    let sample = read_back
        .samples
        .iter()
        .find(|sample| sample.track == timecode_track.track_id)
        .expect("timecode sample");
    let timecode = timecode_from_sample(tmcd, sample_bytes(&out, sample)).expect("decode");
    assert_eq!(
        (
            timecode.hours,
            timecode.minutes,
            timecode.seconds,
            timecode.frames
        ),
        (10, 0, 0, 0)
    );

    assert_eq!(read_back.samples.len(), expected.len());
    for sample in &read_back.samples {
        let key = (sample.track.value(), sample.index);
        assert_eq!(
            sample_bytes(&out, sample),
            expected.get(&key).expect("expected sample").as_slice(),
            "sample {key:?}"
        );
    }

    if !test_fixtures::process::tool_available("ffprobe", "-version") {
        eprintln!("SKIP quicktime_write ffprobe check: ffprobe is not on PATH");
        return;
    }
    let path = std::env::temp_dir().join(format!(
        "isobmff-quicktime-write-{}.mov",
        std::process::id()
    ));
    std::fs::write(&path, &out).expect("write temp mov");
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,codec_tag_string:stream_tags=timecode",
            "-of",
            "compact",
        ])
        .arg(&path)
        .output()
        .expect("run ffprobe");
    let _ = std::fs::remove_file(&path);
    assert!(output.status.success(), "ffprobe failed: {output:?}");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("codec_tag_string=tmcd"), "ffprobe: {text}");
    assert!(text.contains("timecode=10:00:00:00"), "ffprobe: {text}");
}

#[test]
fn the_fragmented_writer_accepts_a_typed_quicktime_track() {
    let mut setup = TrackSetup::new(
        TrackId::new(1),
        *b"tmcd",
        b"TimecodeHandler\0".to_vec(),
        broadcast_common::timescale!(12800),
        SampleEntry::Typed(SampleEntryVariant::Tmcd(Box::new(timecode_entry()))),
    );
    setup.gmhd = Some(gmhd());
    let writer = Writer::fragmented(
        vec![setup],
        broadcast_common::timescale!(1000),
        Limits::default(),
    )
    .expect("fragmented writer");
    let init = writer.init().expect("init segment");
    assert!(
        init.windows(4).any(|window| window == b"tmcd"),
        "the init carries the tmcd sample entry"
    );
    assert!(
        init.windows(4).any(|window| window == b"gmhd"),
        "the init carries the typed gmhd"
    );
}

#[test]
fn the_classic_quicktime_form_has_no_ftyp() {
    let timecode_setup = || {
        let mut setup = TrackSetup::new(
            TrackId::new(1),
            *b"tmcd",
            b"TimecodeHandler\0".to_vec(),
            broadcast_common::timescale!(12800),
            SampleEntry::Typed(SampleEntryVariant::Tmcd(Box::new(timecode_entry()))),
        );
        setup.gmhd = Some(gmhd());
        setup
    };

    let mut fragmented = Writer::fragmented(
        vec![timecode_setup()],
        broadcast_common::timescale!(1000),
        Limits::default(),
    )
    .expect("fragmented writer");
    fragmented.set_file_type(FileType::without_ftyp());
    let init = fragmented.init().expect("init segment");
    assert!(
        !init.windows(4).any(|window| window == b"ftyp"),
        "the classic form has no ftyp"
    );
    assert!(init.windows(4).any(|window| window == b"moov"));

    let mut builder = PlanBuilder::new(vec![timecode_setup()], Limits::default());
    builder
        .push(
            TrackId::new(1),
            PlanSample {
                duration: 38400,
                composition_offset: 0,
                sync: true,
                size: 4,
                description_index: 1,
            },
        )
        .expect("sample");
    builder
        .set_durations(TrackId::new(1), 38400, 38400)
        .expect("durations");
    let plan = builder.finish().expect("plan");
    let mut writer = Writer::progressive(
        plan,
        Layout::new(
            false,
            Interleave::ByDuration { millis: 500 },
            Sink::Seekable,
        ),
        FileType::without_ftyp(),
        MovieHeader::new(broadcast_common::timescale!(1000)),
        Limits::default(),
    )
    .expect("progressive writer");
    let mut out = vec![0u8; usize::try_from(writer.file_len()).expect("file length")];
    for write in writer.header() {
        place(&mut out, &write);
    }
    for write in writer
        .push(TrackId::new(1), 0, &[0, 0, 0, 0])
        .expect("push")
    {
        place(&mut out, &write);
    }
    for write in writer.finish().expect("finish") {
        place(&mut out, &write);
    }
    assert!(
        !out.windows(4).any(|window| window == b"ftyp"),
        "the progressive classic form has no ftyp"
    );
    let run = drive(&out);
    assert_eq!(run.tracks.len(), 1);
    assert_eq!(run.tracks[0].handler, *b"tmcd");
}
