//! The progressive writer driven from a plan: whole-file round trips on real
//! fixtures, both sinks, a 64-bit layout, and the plan-mismatch failures.
//!
//! The round trip is the strong form of the writer's contract: read a file's
//! index through [`Reader`], take [`SamplePlan::to_owned_plan`], write it back
//! with the source layout, push every sample in file order, and compare the
//! whole file. A transmux-written fixture must come back byte for byte; a
//! third-party one must at least keep every track and every sample's
//! descriptor and bytes, and the test names what else differs.

use std::collections::BTreeMap;
use std::io::{Seek, SeekFrom, Write as IoWrite};

use broadcast_common::Parse;
use broadcast_common::timescale::Timescale;
use isobmff::init_segment::MovieBox;
use isobmff::plan::{OwnedPlan, PlanBuilder, PlanSample};
use isobmff::sample_table::SampleRef;
use isobmff::writer::{
    FileType, Interleave, Layout, MovieHeader, ProgressiveWriter, Sink, Write, Writer,
};
use isobmff::{Limits, Reader, ReaderConfig, SampleOrder, Step, TrackId, TrackInfo};

// ---------------------------------------------------------------------------
// Driving the reader
// ---------------------------------------------------------------------------

fn config(bytes: &[u8], order: SampleOrder) -> ReaderConfig {
    ReaderConfig::default()
        .with_total_len(Some(bytes.len() as u64))
        .with_order(order)
}

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

fn mib(value: u64) -> usize {
    usize::try_from(value * 1024 * 1024).expect("this host indexes the buffer")
}

/// One full reader run: its tracks, its samples in file order, and its plan.
fn read_all(bytes: &[u8]) -> (Vec<TrackInfo>, Vec<SampleRef>, Option<OwnedPlan>) {
    let mut reader = Reader::new(config(bytes, SampleOrder::File));
    let mut tracks = Vec::new();
    let mut samples = Vec::new();
    loop {
        match reader.step().expect("reader step") {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let end = usize::try_from(range.end()).expect("end");
                reader
                    .supply(range.offset, &bytes[start..end])
                    .expect("supply");
            }
            Step::Tracks(step_tracks) => tracks = step_tracks.to_vec(),
            Step::Metadata(_) => {}
            Step::Sample(sample) => samples.push(sample),
            Step::Done => break,
            _ => break,
        }
    }
    let plan = reader.plan().and_then(|plan| plan.to_owned_plan().ok());
    (tracks, samples, plan)
}

fn sample_bytes<'a>(bytes: &'a [u8], sample: &SampleRef) -> &'a [u8] {
    let start = usize::try_from(sample.offset).expect("offset");
    let end = start + usize::try_from(sample.size).expect("size");
    &bytes[start..end]
}

// ---------------------------------------------------------------------------
// The source file's `ftyp` brands, `mvhd` fields and box order
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct TopBox {
    start: usize,
    header: usize,
    size: usize,
    fourcc: [u8; 4],
}

fn top_boxes(bytes: &[u8]) -> Vec<TopBox> {
    children(bytes, 0, bytes.len())
}

fn children(bytes: &[u8], start: usize, end: usize) -> Vec<TopBox> {
    let mut out = Vec::new();
    let mut offset = start;
    while offset + 8 <= end {
        let mut size = u32::from_be_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        let mut header = 8usize;
        if size == 1 {
            let mut large = [0u8; 8];
            large.copy_from_slice(&bytes[offset + 8..offset + 16]);
            size = usize::try_from(u64::from_be_bytes(large)).unwrap_or(usize::MAX);
            header = 16;
        } else if size == 0 {
            size = end - offset;
        }
        let fourcc = [
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ];
        out.push(TopBox {
            start: offset,
            header,
            size,
            fourcc,
        });
        if size < header {
            break;
        }
        offset += size;
    }
    out
}

fn find_top(bytes: &[u8], fourcc: &[u8; 4]) -> Option<TopBox> {
    top_boxes(bytes).into_iter().find(|b| &b.fourcc == fourcc)
}

#[derive(Debug, Clone)]
struct Header {
    file_type: FileType,
    movie: MovieHeader,
    faststart: bool,
}

fn file_type(bytes: &[u8], ftyp: &TopBox) -> FileType {
    let body = &bytes[ftyp.start + ftyp.header..ftyp.start + ftyp.size];
    let le = |slice: &[u8]| -> u32 { u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]) };
    let mut brands = Vec::new();
    let mut offset = 8;
    while offset + 4 <= body.len() {
        brands.push([
            body[offset],
            body[offset + 1],
            body[offset + 2],
            body[offset + 3],
        ]);
        offset += 4;
    }
    FileType {
        major_brand: [body[0], body[1], body[2], body[3]],
        minor_version: le(&body[4..8]),
        compatible_brands: brands,
        emit_ftyp: true,
    }
}

fn header_of(bytes: &[u8]) -> Header {
    let ftyp = find_top(bytes, b"ftyp").expect("ftyp");
    let moov = find_top(bytes, b"moov").expect("moov");
    let mdat = find_top(bytes, b"mdat").expect("mdat");
    let movie_box =
        MovieBox::parse(&bytes[moov.start..moov.start + moov.size]).expect("moov parse");
    Header {
        file_type: file_type(bytes, &ftyp),
        movie: MovieHeader::with_times(
            movie_box.mvhd.timescale,
            movie_box.mvhd.creation_time,
            movie_box.mvhd.modification_time,
        ),
        faststart: moov.start < mdat.start,
    }
}

/// The first differing byte, named by the top-level box that encloses it (and
/// the enclosing `trak` when it is inside `moov`).
fn first_difference(written: &[u8], source: &[u8]) -> Option<String> {
    let common = written.len().min(source.len());
    let offset = (0..common).find(|&index| written[index] != source[index])?;
    let boxes = top_boxes(written);
    let name = boxes
        .iter()
        .rev()
        .find(|b| b.start <= offset && offset < b.start + b.size)
        .map_or_else(
            || "outside any box".to_string(),
            |b| String::from_utf8_lossy(&b.fourcc).to_string(),
        );
    Some(format!(
        "first differing byte {offset} inside `{name}` (written {} bytes, source {} bytes)",
        written.len(),
        source.len()
    ))
}

// ---------------------------------------------------------------------------
// Placing writer output
// ---------------------------------------------------------------------------

fn place(out: &mut Vec<u8>, write: Write<'_>) {
    let start = usize::try_from(write.offset).expect("write offset");
    let end = start + write.bytes.len();
    if out.len() < end {
        out.resize(end, 0);
    }
    out[start..end].copy_from_slice(&write.bytes);
}

fn new_writer(plan: OwnedPlan, header: &Header, layout: Layout) -> ProgressiveWriter {
    Writer::progressive(
        plan,
        layout,
        header.file_type.clone(),
        header.movie,
        Limits::default(),
    )
    .expect("writer")
}

/// Write the whole file with `push` called for every sample, in `pushed` order.
fn write_all(
    header: &Header,
    plan: OwnedPlan,
    layout: Layout,
    source: &[u8],
    order: &[SampleRef],
) -> Vec<u8> {
    let mut writer = new_writer(plan, header, layout);
    let len = usize::try_from(writer.file_len()).expect("file length");
    let mut out = vec![0u8; len];
    for write in writer.header() {
        place(&mut out, write);
    }
    for sample in order {
        for write in writer
            .push(sample.track, sample.index, sample_bytes(source, sample))
            .expect("push")
        {
            place(&mut out, write);
        }
    }
    for write in writer.finish().expect("finish") {
        place(&mut out, write);
    }
    out
}

// ---------------------------------------------------------------------------
// Round trips
// ---------------------------------------------------------------------------

/// A file this workspace's progressive muxer wrote, read through the reader
/// and rewritten from its index alone, must come back byte for byte — both
/// box orders.
#[test]
fn a_transmux_written_file_round_trips_byte_identically() {
    use broadcast_common::{Package, Unpackage};
    let ts = test_fixtures::read("ts/h264-aac.ts");
    let media = transmux::TsDemux::new()
        .unpackage(ts.as_slice())
        .expect("demux h264-aac.ts");
    for faststart in [true, false] {
        let source = transmux::progressive::ProgressiveMux::new(faststart)
            .package(&media)
            .expect("package progressive");
        let header = header_of(&source);
        assert_eq!(header.faststart, faststart, "the packaged box order");
        let (tracks, samples, plan) = read_all(&source);
        assert!(!tracks.is_empty(), "faststart={faststart}: no tracks");
        assert!(!samples.is_empty(), "faststart={faststart}: no samples");
        let plan = plan.expect("plan");
        let layout = Layout::new(faststart, Interleave::AsPlanned, Sink::Seekable);
        let written = write_all(&header, plan, layout, &source, &samples);
        if let Some(difference) = first_difference(&written, &source) {
            panic!("faststart={faststart}: {difference}");
        }
        assert_eq!(written.len(), source.len(), "faststart={faststart}: length");
    }
}

/// The sample tables of every track, keyed by four-CC (the `stco`/`co64`
/// table is left out: its offsets move when the `moov` size changes).
fn table_boxes(bytes: &[u8]) -> Vec<BTreeMap<[u8; 4], Vec<u8>>> {
    let moov = find_top(bytes, b"moov").expect("moov");
    let mut tables = Vec::new();
    for trak in children(bytes, moov.start + moov.header, moov.start + moov.size)
        .into_iter()
        .filter(|b| &b.fourcc == b"trak")
    {
        let mdia = child_of(bytes, &trak, b"mdia").expect("mdia");
        let minf = child_of(bytes, &mdia, b"minf").expect("minf");
        let stbl = child_of(bytes, &minf, b"stbl").expect("stbl");
        let mut table = BTreeMap::new();
        for boxed in children(bytes, stbl.start + stbl.header, stbl.start + stbl.size) {
            if matches!(
                &boxed.fourcc,
                b"stsd" | b"stts" | b"ctts" | b"stsc" | b"stsz" | b"stz2" | b"stss"
            ) {
                table.insert(
                    boxed.fourcc,
                    bytes[boxed.start..boxed.start + boxed.size].to_vec(),
                );
            }
        }
        tables.push(table);
    }
    tables
}

fn child_of(bytes: &[u8], parent: &TopBox, fourcc: &[u8; 4]) -> Option<TopBox> {
    children(
        bytes,
        parent.start + parent.header,
        parent.start + parent.size,
    )
    .into_iter()
    .find(|b| &b.fourcc == fourcc)
}

/// The committed progressive fixtures carry boxes an [`OwnedPlan`] does not
/// model, so the whole file is not byte-identical. Named differences:
///
/// - each track's `stbl` also holds `sgpd`/`sbgp` sample-group boxes,
/// - `moov` holds a `udta`,
/// - a top-level `free` box follows the `moov`,
/// - `stss` is ordered before `ctts` rather than last.
///
/// Everything the plan does carry — `stsd`/`stts`/`ctts`/`stsc`/`stsz`/`stss`
/// box for box, every sample's descriptor and every sample byte — is equal.
#[test]
fn committed_progressive_fixtures_keep_their_sample_tables() {
    let fixtures = [
        "mp4/progressive/av-faststart.mp4",
        "mp4/progressive/av-prog.mp4",
    ];
    for name in fixtures {
        let path = test_fixtures::path(name);
        let source = std::fs::read(&path).expect("fixture");
        let header = header_of(&source);
        let (_, source_samples, plan) = read_all(&source);
        let plan = plan.expect("plan");
        let layout = Layout::new(header.faststart, Interleave::AsPlanned, Sink::Seekable);
        let written = write_all(&header, plan, layout, &source, &source_samples);
        assert_eq!(
            table_boxes(&written),
            table_boxes(&source),
            "{name}: sample tables"
        );
        let (_, written_samples, _) = read_all(&written);
        let want = by_sample(&source_samples);
        let got = by_sample(&written_samples);
        assert_eq!(got.len(), want.len(), "{name}: sample count");
        for (key, expected) in &want {
            let actual = got
                .get(key)
                .unwrap_or_else(|| panic!("{name}: {key:?} missing"));
            assert_eq!(descriptor(actual), descriptor(expected), "{name}: {key:?}");
            assert_eq!(
                sample_bytes(&written, actual),
                sample_bytes(&source, expected),
                "{name}: {key:?} bytes"
            );
        }
    }
}

/// One sample's descriptor with the file offset blanked: a third-party `moov`
/// this crate rewrites may be a different size, so the offsets legitimately
/// move.
fn descriptor(sample: &SampleRef) -> SampleRef {
    let mut keyed = sample.clone();
    keyed.offset = 0;
    keyed
}

fn by_sample(samples: &[SampleRef]) -> BTreeMap<(u32, u32), SampleRef> {
    samples
        .iter()
        .map(|sample| ((sample.track.value(), sample.index), sample.clone()))
        .collect()
}

fn third_party_round_trip(rel: &str, expected_tracks: usize) {
    let Some(path) = test_fixtures::private(rel) else {
        eprintln!("SKIP writer_progressive: private fixture {rel} is not checked out");
        return;
    };
    let source = std::fs::read(&path).expect("fixture");
    let header = header_of(&source);
    let (source_tracks, source_samples, plan) = read_all(&source);
    assert_eq!(source_tracks.len(), expected_tracks, "{rel}: track count");
    let plan = plan.expect("plan");
    let faststart = Layout::new(true, Interleave::AsPlanned, Sink::Seekable);
    let written = write_all(&header, plan, faststart, &source, &source_samples);

    let (written_tracks, written_samples, _) = read_all(&written);
    assert_eq!(
        written_tracks.len(),
        expected_tracks,
        "{rel}: written tracks"
    );
    let source_entries: Vec<(u32, [u8; 4])> = source_tracks
        .iter()
        .map(|track| (track.track_id.value(), track.handler))
        .collect();
    let written_entries: Vec<(u32, [u8; 4])> = written_tracks
        .iter()
        .map(|track| (track.track_id.value(), track.handler))
        .collect();
    assert_eq!(written_entries, source_entries, "{rel}: track identity");

    let want = by_sample(&source_samples);
    let got = by_sample(&written_samples);
    assert_eq!(got.len(), want.len(), "{rel}: sample count");
    for (key, expected) in &want {
        let actual = got
            .get(key)
            .unwrap_or_else(|| panic!("{rel}: {key:?} missing"));
        assert_eq!(
            descriptor(actual),
            descriptor(expected),
            "{rel}: {key:?} descriptor"
        );
        assert_eq!(
            sample_bytes(&written, actual),
            sample_bytes(&source, expected),
            "{rel}: {key:?} bytes"
        );
    }
}

#[test]
fn a_camera_fixture_round_trips_every_track() {
    third_party_round_trip("mp4/camera/gopro-karma.mp4", 5);
}

#[test]
fn a_tagged_m4a_round_trips() {
    third_party_round_trip("mp4/tags/lavf-tagged.m4a", 1);
}

#[test]
fn a_quicktime_file_round_trips() {
    third_party_round_trip("mp4/quicktime/lavf-aac-tmcd.mov", 3);
}

/// A seekable sink does not care what order the samples arrive in: reversing
/// every push yields the same file.
#[test]
fn a_reversed_push_order_writes_the_same_file() {
    let path = test_fixtures::path("mp4/progressive/av-prog.mp4");
    let source = std::fs::read(&path).expect("fixture");
    let header = header_of(&source);
    let (_, samples, plan) = read_all(&source);
    let plan = plan.expect("plan");
    let layout = Layout::new(header.faststart, Interleave::AsPlanned, Sink::Seekable);
    let forward = write_all(&header, plan.clone(), layout, &source, &samples);
    let reversed: Vec<SampleRef> = samples.iter().rev().cloned().collect();
    let backward = write_all(&header, plan, layout, &source, &reversed);
    assert_eq!(forward, backward, "the sink is seekable");
}

// ---------------------------------------------------------------------------
// Synthetic plans: caller-supplied entries and durations, no fixture
// ---------------------------------------------------------------------------

/// An empty `avc1` sample entry (box header only).
const RAW_AVC1: [u8; 8] = [0, 0, 0, 8, b'a', b'v', b'c', b'1'];
/// An empty `mp4a` sample entry (box header only).
const RAW_MP4A: [u8; 8] = [0, 0, 0, 8, b'm', b'p', b'4', b'a'];
/// Movie timescale of the synthetic plans.
const MOVIE_TIMESCALE: u32 = 1_000;
/// Video timescale of the synthetic plans.
const VIDEO_TIMESCALE: u32 = 90_000;
/// Audio timescale of the synthetic plans.
const AUDIO_TIMESCALE: u32 = 48_000;
/// Video track id of the synthetic plans.
const VIDEO_TRACK: u32 = 1;
/// Audio track id of the synthetic plans.
const AUDIO_TRACK: u32 = 2;

struct TrackShape {
    track_id: u32,
    handler: [u8; 4],
    handler_name: &'static [u8],
    entry: [u8; 8],
    fourcc: [u8; 4],
    timescale: u32,
    duration: u32,
    samples: u32,
    size: u32,
}

fn video_shape(duration: u32, samples: u32, size: u32) -> TrackShape {
    TrackShape {
        track_id: VIDEO_TRACK,
        handler: *b"vide",
        handler_name: b"VideoHandler\0",
        entry: RAW_AVC1,
        fourcc: *b"avc1",
        timescale: VIDEO_TIMESCALE,
        duration,
        samples,
        size,
    }
}

fn audio_shape(duration: u32, samples: u32, size: u32) -> TrackShape {
    TrackShape {
        track_id: AUDIO_TRACK,
        handler: *b"soun",
        handler_name: b"SoundHandler\0",
        entry: RAW_MP4A,
        fourcc: *b"mp4a",
        timescale: AUDIO_TIMESCALE,
        duration,
        samples,
        size,
    }
}

fn build_plan(shapes: &[TrackShape]) -> OwnedPlan {
    let setups: Vec<_> = shapes
        .iter()
        .map(|shape| {
            isobmff::writer::TrackSetup::new(
                TrackId::new(shape.track_id),
                shape.handler,
                shape.handler_name.to_vec(),
                timescale(shape.timescale),
                isobmff::writer::SampleEntry::Raw {
                    fourcc: shape.fourcc,
                    body: shape.entry.to_vec(),
                },
            )
        })
        .collect();
    let mut builder = PlanBuilder::new(setups, Limits::default());
    for shape in shapes {
        let id = TrackId::new(shape.track_id);
        let media = u64::from(shape.duration) * u64::from(shape.samples);
        builder.set_durations(id, media, media).expect("durations");
        for _ in 0..shape.samples {
            builder
                .push(id, PlanSample::new(shape.duration, shape.size))
                .expect("push");
        }
    }
    builder.finish().expect("plan")
}

fn synthetic_header() -> Header {
    Header {
        file_type: FileType::default(),
        movie: MovieHeader::new(timescale(MOVIE_TIMESCALE)),
        faststart: true,
    }
}

fn append_only_layout(reorder_bytes: u64) -> Layout {
    Layout::new(
        true,
        Interleave::ByDuration {
            millis: MOVIE_TIMESCALE,
        },
        Sink::AppendOnly { reorder_bytes },
    )
}

/// Push every sample of `shapes`, video track first, then the audio track.
fn push_by_track(
    plan: &OwnedPlan,
    shapes: &[TrackShape],
    reorder_bytes: u64,
) -> isobmff::Result<()> {
    let header = synthetic_header();
    let mut writer = new_writer(plan.clone(), &header, append_only_layout(reorder_bytes));
    let _ = writer.header();
    for shape in shapes {
        let data = vec![0u8; usize::try_from(shape.size).expect("size")];
        for index in 0..shape.samples {
            let _ = writer.push(TrackId::new(shape.track_id), index, &data)?;
        }
    }
    let _ = writer.finish()?;
    Ok(())
}

fn total_bytes(shapes: &[TrackShape]) -> u64 {
    shapes
        .iter()
        .map(|shape| u64::from(shape.samples) * u64::from(shape.size))
        .sum()
}

/// The smallest reorder cap that lets an all-video-then-all-audio push order
/// complete: success at that cap, `ReorderBufferFull` one byte below it.
#[test]
fn the_append_only_buffer_boundary_is_exact() {
    // Video chunks hold one 1 s sample; audio chunks two 0.5 s samples, so the
    // file order alternates and pushing all video first holds audio-turn gaps.
    let shapes = [video_shape(90_000, 10, 100), audio_shape(24_000, 20, 100)];
    let plan = build_plan(&shapes);
    let total = total_bytes(&shapes);
    let mut low = 0u64;
    let mut high = total;
    while low < high {
        let middle = low + (high - low) / 2;
        if push_by_track(&plan, &shapes, middle).is_ok() {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    let skew = low;
    assert!(skew > 0, "the push order must actually need a buffer");
    assert!(skew < total, "the skew must be smaller than the whole file");
    push_by_track(&plan, &shapes, skew).expect("the cap that covers the skew succeeds");
    let err = push_by_track(&plan, &shapes, skew - 1).expect_err("one byte less must fail");
    assert!(
        matches!(err, isobmff::Error::ReorderBufferFull { cap } if cap == skew - 1),
        "expected ReorderBufferFull at {}, got {err:?}",
        skew - 1
    );
}

/// The buffer never grows past the caller's cap: this binary's
/// `test_alloc::ThreadCapped` (256 MiB per thread) would abort the process if
/// the writer buffered the whole all-video push order instead of refusing it.
#[test]
fn the_append_only_buffer_never_grows_past_the_cap() {
    const VIDEO_BYTES: u32 = 48 * 1024 * 1024;
    const CAP: u64 = 1024 * 1024;
    let shapes = [
        video_shape(90_000, 10, VIDEO_BYTES),
        audio_shape(24_000, 20, 100),
    ];
    let plan = build_plan(&shapes);
    let err = push_by_track(&plan, &shapes, CAP).expect_err("the cap cannot hold the skew");
    assert!(
        matches!(err, isobmff::Error::ReorderBufferFull { cap } if cap == CAP),
        "expected ReorderBufferFull, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Failures
// ---------------------------------------------------------------------------

#[test]
fn a_sample_one_byte_short_is_a_plan_mismatch() {
    let shapes = [video_shape(3000, 4, 10)];
    let plan = build_plan(&shapes);
    let header = synthetic_header();
    let mut writer = new_writer(
        plan,
        &header,
        Layout::new(
            true,
            Interleave::ByDuration {
                millis: MOVIE_TIMESCALE,
            },
            Sink::Seekable,
        ),
    );
    let _ = writer.header();
    let short = [0u8; 9];
    let err = writer
        .push(TrackId::new(VIDEO_TRACK), 0, &short)
        .unwrap_err();
    match err {
        isobmff::Error::PlanMismatch {
            track,
            index,
            planned,
            got,
        } => {
            assert_eq!(track, TrackId::new(VIDEO_TRACK));
            assert_eq!((index, planned, got), (0, 10, 9));
        }
        other => panic!("expected PlanMismatch, got {other:?}"),
    }
}

#[test]
fn a_missing_sample_is_plan_incomplete() {
    let shapes = [video_shape(3000, 4, 10)];
    let plan = build_plan(&shapes);
    let header = synthetic_header();
    let mut writer = new_writer(
        plan,
        &header,
        Layout::new(
            true,
            Interleave::ByDuration {
                millis: MOVIE_TIMESCALE,
            },
            Sink::Seekable,
        ),
    );
    let _ = writer.header();
    let data = [0u8; 10];
    for index in 0..3 {
        let _ = writer
            .push(TrackId::new(VIDEO_TRACK), index, &data)
            .expect("push");
    }
    match writer.finish().unwrap_err() {
        isobmff::Error::PlanIncomplete { track, missing } => {
            assert_eq!(track, TrackId::new(VIDEO_TRACK));
            assert_eq!(missing, 3);
        }
        other => panic!("expected PlanIncomplete, got {other:?}"),
    }
}

#[test]
fn as_planned_without_a_source_chunk_table_is_a_typed_error() {
    let shapes = [video_shape(3000, 4, 10)];
    let plan = build_plan(&shapes);
    let header = synthetic_header();
    let result = Writer::progressive(
        plan,
        Layout::new(true, Interleave::AsPlanned, Sink::Seekable),
        header.file_type,
        header.movie,
        Limits::default(),
    );
    match result {
        Ok(_) => panic!("AsPlanned without a source chunk table must be a typed error"),
        Err(err) => assert!(matches!(err, isobmff::Error::NoPlan { .. }), "got {err:?}"),
    }
}

// ---------------------------------------------------------------------------
// A 64-bit layout: co64 chunk offsets and an mdat with largesize
// ---------------------------------------------------------------------------

/// Read exactly one byte range of `file` without loading the whole file.
fn read_range(file: &mut std::fs::File, offset: u64, len: usize) -> Vec<u8> {
    use std::io::Read as IoRead;
    file.seek(SeekFrom::Start(offset)).expect("seek");
    let mut buffer = vec![0u8; len];
    file.read_exact(&mut buffer).expect("read range");
    buffer
}

/// A plan whose samples total more than 4 GiB: the `mdat` needs `largesize`
/// (§4.2) and a chunk that starts past `u32::MAX` needs the 64-bit `co64`
/// (§8.7.5). The file is written sample by sample from one reusable buffer and
/// read back through the reader, which never loads the media.
///
/// One sample is one chunk (`ByDuration` of one second, samples one second
/// long), so the last chunk's offset is past the 32-bit field; writing takes a
/// few seconds on a local disk and the file is removed afterwards.
#[test]
fn a_co64_layout_past_four_gigabytes_round_trips_through_the_reader() {
    const SAMPLE_BYTES: u32 = 64 * 1024 * 1024;
    const SAMPLES: u32 = 70;
    assert!(
        u64::from(SAMPLE_BYTES) * u64::from(SAMPLES) > u64::from(u32::MAX),
        "the layout must pass the 32-bit field"
    );
    let shapes = [video_shape(VIDEO_TIMESCALE, SAMPLES, SAMPLE_BYTES)];
    let plan = build_plan(&shapes);
    let header = synthetic_header();
    let layout = Layout::new(
        true,
        Interleave::ByDuration {
            millis: MOVIE_TIMESCALE,
        },
        Sink::Seekable,
    );
    let mut writer = new_writer(plan, &header, layout);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/p2s7-co64.mp4");
    let mut file = std::fs::File::create(&path).expect("create sparse target");
    for write in writer.header() {
        file.seek(SeekFrom::Start(write.offset)).expect("seek");
        file.write_all(&write.bytes).expect("write header");
    }
    let data = vec![0u8; mib(u64::from(SAMPLE_BYTES) / (1024 * 1024))];
    for index in 0..SAMPLES {
        for write in writer
            .push(TrackId::new(VIDEO_TRACK), index, &data)
            .expect("push")
        {
            file.seek(SeekFrom::Start(write.offset)).expect("seek");
            file.write_all(&write.bytes).expect("write sample");
        }
    }
    for write in writer.finish().expect("finish") {
        file.seek(SeekFrom::Start(write.offset)).expect("seek");
        file.write_all(&write.bytes).expect("write trailer");
    }
    drop(file);

    let total = std::fs::metadata(&path).expect("metadata").len();
    let mut file = std::fs::File::open(&path).expect("reopen");
    let prefix = read_range(&mut file, 0, 128 * 1024);
    assert!(
        prefix.windows(4).any(|window| window == b"co64"),
        "the moov must carry a co64 chunk-offset table"
    );
    let mdat = top_boxes(&prefix)
        .into_iter()
        .find(|b| &b.fourcc == b"mdat")
        .expect("mdat header");
    assert_eq!(
        mdat.header, 16,
        "a payload past 4 GiB takes the 64-bit largesize header"
    );

    let mut reader = Reader::new(
        ReaderConfig::default()
            .with_total_len(Some(total))
            .with_order(SampleOrder::File),
    );
    let mut samples = Vec::new();
    loop {
        match reader.step().expect("reader step") {
            Step::Need(range) => {
                let len = usize::try_from(range.len).expect("range length");
                let buffer = read_range(&mut file, range.offset, len);
                reader.supply(range.offset, &buffer).expect("supply");
            }
            Step::Sample(sample) => samples.push(sample),
            Step::Done => break,
            _ => {}
        }
    }
    assert_eq!(samples.len() as u32, SAMPLES, "every sample is indexed");
    assert!(
        samples[samples.len() - 1].offset > u64::from(u32::MAX),
        "the last chunk offset is past the 32-bit field"
    );
    for sample in &samples {
        assert_eq!(sample.size, SAMPLE_BYTES, "sample size");
    }
    let _ = std::fs::remove_file(&path);
}
