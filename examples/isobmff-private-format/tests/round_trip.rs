//! The worked example, exercised: build a progressive file and a fragmented
//! file carrying a private sample entry and a private `udta` atom, read both
//! back with the registry (typed) and without it (opaque), and prove the bytes
//! do not change either way.

use std::path::PathBuf;

use broadcast_common::timescale::Timescale;

use isobmff::box_support::OpaqueBox;
use isobmff::plan::{MovieShell, OwnedPlan, PlanBuilder, PlanSample};
use isobmff::sample_table::SampleRef;
use isobmff::writer::{
    FileType, Interleave, Layout, MovieHeader, SampleEntry, SampleMeta, Sink, TrackSetup, Write,
    Writer,
};
use isobmff::{Limits, Reader, ReaderConfig, SampleOrder, Step, TrackId, TrackInfo, parse_box};
use isobmff_private_format::{
    AcmeCalibrationBox, AcmeTelemetryEntry, CALIBRATION_FOURCC, TELEMETRY_FOURCC,
    TELEMETRY_HANDLER, acme_registry,
};

const TIMESCALE: u32 = 1_000;
const ACTM_SAMPLES: u32 = 4;
const ACTM_SAMPLE_BYTES: u32 = 16;
const VIDEO_SAMPLES_IN_FIRST_FRAGMENT: usize = 8;
const TELEMETRY_TRACK: u32 = 9;
const SERIAL: u32 = 0x1234_5678;
const SAMPLE_FILL: u8 = 0x5a;

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

fn video_fixture() -> Vec<u8> {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "..",
        "..",
        "fixtures",
        "mp4",
        "progressive",
        "av-prog.mp4",
    ]
    .iter()
    .collect();
    std::fs::read(&path).expect("fixtures/mp4/progressive/av-prog.mp4")
}

fn calibration() -> AcmeCalibrationBox {
    AcmeCalibrationBox {
        gain: 0x0001_8000,
        shift: -4,
        label: b"acme".to_vec(),
    }
}

/// One reader run over a file.
struct Run {
    tracks: Vec<TrackInfo>,
    samples: Vec<SampleRef>,
    fragments: Vec<Vec<SampleRef>>,
    sequences: Vec<u32>,
    plan: OwnedPlan,
}

fn drive(bytes: &[u8]) -> Run {
    drive_with(bytes, None)
}

fn drive_with(bytes: &[u8], registry: Option<isobmff::registry::BoxRegistry>) -> Run {
    let mut config = ReaderConfig::default()
        .with_total_len(Some(u64::try_from(bytes.len()).expect("length")))
        .with_order(SampleOrder::File);
    if let Some(registry) = registry {
        config = config.with_registry(registry);
    }
    let mut reader = Reader::new(config);
    let mut tracks = Vec::new();
    let mut samples = Vec::new();
    let mut fragments: Vec<Vec<SampleRef>> = Vec::new();
    let mut sequences = Vec::new();
    loop {
        match reader.step().expect("reader step") {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let end = usize::try_from(range.end()).expect("end").min(bytes.len());
                reader
                    .supply(range.offset, &bytes[start..end])
                    .expect("supply");
            }
            Step::Tracks(step_tracks) => tracks = step_tracks.to_vec(),
            Step::Fragment(info) => {
                fragments.push(Vec::new());
                sequences.push(info.sequence_number);
            }
            Step::Sample(sample) => {
                if let Some(fragment) = fragments.last_mut() {
                    fragment.push(sample.clone());
                }
                samples.push(sample);
            }
            Step::Done => break,
            _ => {}
        }
    }
    let plan = reader
        .plan()
        .expect("a plan")
        .to_owned_plan()
        .expect("owned plan");
    Run {
        tracks,
        samples,
        fragments,
        sequences,
        plan,
    }
}

fn sample_bytes<'a>(bytes: &'a [u8], sample: &SampleRef) -> &'a [u8] {
    let start = usize::try_from(sample.offset).expect("offset");
    let end = start + usize::try_from(sample.size).expect("size");
    &bytes[start..end]
}

fn place(out: &mut [u8], write: &Write<'_>) {
    let start = usize::try_from(write.offset).expect("offset");
    let end = start + write.bytes.len();
    out[start..end].copy_from_slice(&write.bytes);
}

/// The `udta` payload of the file's `moov`.
fn udta_payload(bytes: &[u8]) -> Vec<u8> {
    let moov = isobmff::box_types::find_box(bytes, b"moov").expect("moov");
    let body = parse_box(moov).expect("moov box").0.body;
    let udta = isobmff::box_types::find_box(body, b"udta").expect("udta");
    parse_box(udta).expect("udta box").0.payload().to_vec()
}

/// The registered parse of one hand-built box, as the writer takes it.
fn registered<T: isobmff::registry::BoxDef>(value: &T) -> isobmff::registry::RegisteredEntry {
    let bytes = broadcast_common::Serialize::try_to_bytes(value).expect("box bytes");
    acme_registry()
        .parse(T::PARENT, &bytes)
        .expect("parse")
        .expect("registered")
}

/// The source movie shell with the calibration atom appended to its `udta`.
fn shell_with_calibration(shell: &MovieShell) -> MovieShell {
    let mut shell = shell.clone();
    let calibration = broadcast_common::Serialize::try_to_bytes(&calibration()).expect("acal");
    for boxed in &mut shell.opaque {
        if boxed.box_type == *b"udta" {
            boxed.data.extend_from_slice(&calibration);
            return shell;
        }
    }
    let mut order = shell.order.clone();
    order.push_opaque(*b"udta");
    shell.order = order;
    shell.opaque.push(OpaqueBox::new(*b"udta", calibration));
    shell
}

fn meta_of(sample: &SampleRef) -> SampleMeta {
    SampleMeta {
        duration: sample.duration,
        composition_offset: sample.composition_offset,
        sync: sample.sync,
        aux: sample.aux.clone(),
        description_index: sample.description_index,
    }
}

fn actm_entry() -> SampleEntry {
    SampleEntry::from_registered(registered(&AcmeTelemetryEntry { serial: SERIAL }).as_registered())
        .expect("actm sample entry")
}

fn actm_setup() -> TrackSetup {
    TrackSetup::new(
        TrackId::new(TELEMETRY_TRACK),
        TELEMETRY_HANDLER,
        b"acme telemetry".to_vec(),
        timescale(TIMESCALE),
        actm_entry(),
    )
}

/// The fixture's first track's samples, in file order.
fn video_samples(run: &Run, track: TrackId) -> Vec<&SampleRef> {
    run.samples
        .iter()
        .filter(|sample| sample.track == track)
        .collect()
}

/// A progressive file: the fixture's video track plus one private telemetry
/// track, with the calibration atom in `moov`'s `udta`.
fn build_progressive(source: &[u8], run: &Run) -> Vec<u8> {
    let video = run.plan.tracks().first().expect("the fixture has a track");
    let video_id = video.setup.track_id;
    let samples = video_samples(run, video_id);
    let mut builder = PlanBuilder::new(vec![video.setup.clone(), actm_setup()], Limits::default());
    for sample in &samples {
        builder
            .push(
                sample.track,
                PlanSample {
                    duration: sample.duration,
                    composition_offset: sample.composition_offset,
                    sync: sample.sync,
                    size: sample.size,
                    description_index: sample.description_index,
                },
            )
            .expect("push video sample");
    }
    for _ in 0..ACTM_SAMPLES {
        builder
            .push(
                TrackId::new(TELEMETRY_TRACK),
                PlanSample::new(TIMESCALE, ACTM_SAMPLE_BYTES),
            )
            .expect("push telemetry sample");
    }
    builder
        .set_durations(video_id, video.media_duration, video.presentation_duration)
        .expect("video durations");
    builder
        .set_edits(video_id, video.edits.clone())
        .expect("video edits");
    builder
        .set_durations(
            TrackId::new(TELEMETRY_TRACK),
            u64::from(ACTM_SAMPLES) * u64::from(TIMESCALE),
            u64::from(ACTM_SAMPLES) * u64::from(TIMESCALE),
        )
        .expect("telemetry durations");
    let mut plan = builder.finish().expect("plan");
    let shell = run.plan.shell().expect("the fixture's movie shell");
    plan.set_shell(Some(shell_with_calibration(shell)));

    let layout = Layout::new(true, Interleave::ByDuration { millis: 500 }, Sink::Seekable);
    let mut writer = Writer::progressive(
        plan,
        layout,
        FileType::default(),
        MovieHeader::new(timescale(TIMESCALE)),
        Limits::default(),
    )
    .expect("progressive writer");
    let mut out = vec![0u8; usize::try_from(writer.file_len()).expect("length")];
    for write in writer.header() {
        place(&mut out, &write);
    }
    for sample in &samples {
        for write in writer
            .push(sample.track, sample.index, sample_bytes(source, sample))
            .expect("push")
        {
            place(&mut out, &write);
        }
    }
    let telemetry = vec![SAMPLE_FILL; usize::try_from(ACTM_SAMPLE_BYTES).expect("size")];
    for index in 0..ACTM_SAMPLES {
        for write in writer
            .push(TrackId::new(TELEMETRY_TRACK), index, &telemetry)
            .expect("push telemetry")
        {
            place(&mut out, &write);
        }
    }
    for write in writer.finish().expect("finish") {
        place(&mut out, &write);
    }
    out
}

/// A fragmented file over the same two tracks, in two fragments.
fn build_fragmented(source: &[u8], run: &Run) -> Vec<u8> {
    let video = run.plan.tracks().first().expect("the fixture has a track");
    let samples = video_samples(run, video.setup.track_id);
    let mut writer = Writer::fragmented(
        vec![video.setup.clone(), actm_setup()],
        video.setup.timescale,
        Limits::default(),
    )
    .expect("fragmented writer");
    writer.set_movie_shell(shell_with_calibration(
        run.plan.shell().expect("the fixture's movie shell"),
    ));
    let mut out = writer.init().expect("init segment");
    let split = VIDEO_SAMPLES_IN_FIRST_FRAGMENT.min(samples.len());
    let telemetry = vec![SAMPLE_FILL; usize::try_from(ACTM_SAMPLE_BYTES).expect("size")];
    for (index, chunk) in [&samples[..split], &samples[split..]]
        .into_iter()
        .enumerate()
    {
        writer.set_sequence_number(u32::try_from(index + 1).expect("sequence"));
        if let Some(first) = chunk.first() {
            writer
                .set_base_decode_time(first.track, first.decode_time)
                .expect("video base");
        }
        for sample in chunk {
            writer
                .push(sample.track, meta_of(sample), sample_bytes(source, sample))
                .expect("push video");
        }
        let per_fragment = ACTM_SAMPLES / 2;
        writer
            .set_base_decode_time(
                TrackId::new(TELEMETRY_TRACK),
                u64::from(u32::try_from(index).expect("index"))
                    * u64::from(per_fragment)
                    * u64::from(TIMESCALE),
            )
            .expect("telemetry base");
        for _ in 0..per_fragment {
            writer
                .push(
                    TrackId::new(TELEMETRY_TRACK),
                    SampleMeta {
                        duration: TIMESCALE,
                        sync: true,
                        ..SampleMeta::default()
                    },
                    &telemetry,
                )
                .expect("push telemetry");
        }
        if let Some(fragment) = writer.flush().expect("flush") {
            out.extend_from_slice(&fragment.bytes);
        }
    }
    out
}

fn telemetry_track(tracks: &[TrackInfo]) -> &TrackInfo {
    tracks
        .iter()
        .find(|track| track.handler == TELEMETRY_HANDLER)
        .expect("the telemetry track")
}

fn typed_calibration(udta: &[u8]) -> Option<AcmeCalibrationBox> {
    acme_registry()
        .parse_children(*b"udta", udta)
        .filter_map(|child| child.ok())
        .find(|child| child.fourcc == CALIBRATION_FOURCC)
        .and_then(|child| {
            child
                .registered
                .as_ref()
                .and_then(|entry| entry.downcast_ref::<AcmeCalibrationBox>().cloned())
        })
}

/// Whether the calibration atom is present and unparsed (no registry).
fn opaque_calibration(udta: &[u8]) -> bool {
    isobmff::registry::BoxRegistry::new()
        .parse_children(*b"udta", udta)
        .filter_map(|child| child.ok())
        .any(|child| child.fourcc == CALIBRATION_FOURCC && child.registered.is_none())
}

#[test]
fn the_private_entry_and_atom_come_back_typed() {
    let source = video_fixture();
    let run = drive(&source);
    let progressive = build_progressive(&source, &run);

    let with = drive_with(&progressive, Some(acme_registry()));
    assert_eq!(with.tracks.len(), 2, "video + telemetry");
    let track = telemetry_track(&with.tracks);
    let entry = track.entries.first().expect("a telemetry sample entry");
    assert_eq!(entry.fourcc, TELEMETRY_FOURCC);
    assert!(entry.typed.is_none(), "not a four-CC isobmff types");
    assert_eq!(
        entry
            .registered
            .as_ref()
            .and_then(|entry| entry.downcast_ref::<AcmeTelemetryEntry>()),
        Some(&AcmeTelemetryEntry { serial: SERIAL })
    );
    assert_eq!(
        typed_calibration(&udta_payload(&progressive)),
        Some(calibration())
    );

    let fragmented = build_fragmented(&source, &run);
    let with = drive_with(&fragmented, Some(acme_registry()));
    assert_eq!(with.tracks.len(), 2, "video + telemetry");
    let entry = telemetry_track(&with.tracks)
        .entries
        .first()
        .expect("a telemetry sample entry")
        .clone();
    assert_eq!(entry.fourcc, TELEMETRY_FOURCC);
    assert_eq!(
        entry
            .registered
            .as_ref()
            .and_then(|entry| entry.downcast_ref::<AcmeTelemetryEntry>()),
        Some(&AcmeTelemetryEntry { serial: SERIAL })
    );
    assert_eq!(
        typed_calibration(&udta_payload(&fragmented)),
        Some(calibration())
    );
}

#[test]
fn without_the_registry_they_are_opaque_and_the_file_still_round_trips() {
    let source = video_fixture();
    let run = drive(&source);
    let progressive = build_progressive(&source, &run);

    let opaque = drive(&progressive);
    let entry = telemetry_track(&opaque.tracks)
        .entries
        .first()
        .expect("a telemetry sample entry")
        .clone();
    assert_eq!(entry.fourcc, TELEMETRY_FOURCC);
    assert!(entry.typed.is_none());
    assert!(entry.registered.is_none(), "no registry, no parse");
    let udta = udta_payload(&progressive);
    assert!(opaque_calibration(&udta), "the atom is carried opaque");

    let shell = opaque.plan.shell().expect("shell").clone();
    let layout = Layout::new(true, Interleave::AsPlanned, Sink::Seekable);
    let mut writer = Writer::progressive(
        opaque.plan.clone(),
        layout,
        FileType::default(),
        MovieHeader::with_times(
            shell.mvhd.timescale,
            shell.mvhd.creation_time,
            shell.mvhd.modification_time,
        ),
        Limits::default(),
    )
    .expect("writer");
    let mut out = vec![0u8; usize::try_from(writer.file_len()).expect("length")];
    for write in writer.header() {
        place(&mut out, &write);
    }
    for sample in &opaque.samples {
        for write in writer
            .push(
                sample.track,
                sample.index,
                sample_bytes(&progressive, sample),
            )
            .expect("push")
        {
            place(&mut out, &write);
        }
    }
    for write in writer.finish().expect("finish") {
        place(&mut out, &write);
    }
    assert_eq!(out, progressive, "progressive file is byte-identical");
}

#[test]
fn the_fragmented_file_round_trips_too() {
    let source = video_fixture();
    let run = drive(&source);
    let fragmented = build_fragmented(&source, &run);
    let read = drive(&fragmented);
    assert_eq!(read.fragments.len(), 2, "two fragments");
    assert!(opaque_calibration(&udta_payload(&fragmented)));

    let shell = read.plan.shell().expect("shell").clone();
    let setups: Vec<TrackSetup> = read
        .plan
        .tracks()
        .iter()
        .map(|track| track.setup.clone())
        .collect();
    let mut writer = Writer::fragmented(setups, shell.mvhd.timescale, Limits::default())
        .expect("fragmented writer");
    writer.set_movie_shell(shell);
    let mut out = writer.init().expect("init");
    for (index, fragment) in read.fragments.iter().enumerate() {
        writer.set_sequence_number(read.sequences[index]);
        let mut based: Vec<TrackId> = Vec::new();
        for sample in fragment {
            if !based.contains(&sample.track) {
                based.push(sample.track);
                writer
                    .set_base_decode_time(sample.track, sample.decode_time)
                    .expect("base decode time");
            }
        }
        for sample in fragment {
            writer
                .push(
                    sample.track,
                    meta_of(sample),
                    sample_bytes(&fragmented, sample),
                )
                .expect("push");
        }
        if let Some(flushed) = writer.flush().expect("flush") {
            out.extend_from_slice(&flushed.bytes);
        }
    }
    out.extend_from_slice(&writer.trailer().expect("trailer"));
    assert_eq!(out, fragmented, "fragmented file is byte-identical");
}
