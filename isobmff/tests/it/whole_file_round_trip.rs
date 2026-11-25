//! Whole-file round trip: read every box and every track, write the file back
//! through [`Writer`], and compare the bytes.
//!
//! A file this crate's own writer can reproduce must come back byte for byte.
//! A file it cannot is listed in [`EXCEPTIONS`] with the exact reason, the
//! test proves the reason holds for that file, proves the file really is not
//! byte-identical, and then compares it structurally: every track, every
//! sample's descriptor and bytes, and the boxes the writer does emit.

use std::collections::BTreeMap;

use broadcast_common::Parse;

use isobmff::frag_layout::{FragmentIndex, FragmentLayout};
use isobmff::init_segment::MovieBox;
use isobmff::plan::OwnedPlan;
use isobmff::sample_table::SampleRef;
use isobmff::writer::{
    FileType, Interleave, Layout, MovieHeader, SampleEntry, SampleMeta, Sink, TrackSetup, Write,
    Writer,
};
use isobmff::{Limits, Reader, ReaderConfig, SampleOrder, Step, TrackId, TrackInfo};

/// At least this many files must round-trip byte-identically.
const MIN_BYTE_IDENTICAL: usize = 33;

/// The fixture files that must come back byte for byte: every progressive
/// fixture this workspace's muxer wrote, every one ffmpeg wrote, the tag files,
/// the camera captures and the QuickTime `.mov` set (its atoms round-trip
/// because the plan carries the sample entries and `trak` shell verbatim).
const BYTE_IDENTICAL: &[&str] = &[
    "mp4/progressive/av-prog.mp4",
    "mp4/progressive/av-faststart.mp4",
    "mp4/h264-aac-prog.mp4",
    "mp4/aac-sgpd.mp4",
    "mp4/hevc-main.mp4",
    "mp4/opus.mp4",
    "mp4/flac.mp4",
    "mp4/vp9.mp4",
    "mp4/av1.mp4",
    "mp4/colr-hdr.mp4",
    "mp4/clap-vs-crop.mp4",
    "mp4/tref-avcp.mp4",
    "mp4/stpp.mp4",
    "mp4/h264-high.mp4",
    "mp4/tags/afconvert-aac.m4a",
    "mp4/tags/lavf-tagged.m4a",
    "mp4/tags/mp4box-chapters-nero.m4a",
    "mp4/camera/gopro-karma.mp4",
    "mp4/camera/gopro-hero6-ble.mp4",
    "mp4/camera/gopro-max-heromode.mp4",
    "mp4/quicktime/lavf-aac-tmcd.mov",
    "mp4/quicktime/lavf-pcm24.mov",
    "mp4/quicktime/iphone-x-live-photo.mov",
    // The fragmented set: each `moof` is rewritten in the layout the reader
    // recorded for it, and the trailing `mfra` is regenerated from the
    // positions the writer placed its own fragments at.
    "mp4/frag/av1.frag.mp4",
    "mp4/frag/flac.frag.mp4",
    "mp4/frag/h264-high.frag.mp4",
    "mp4/frag/hevc-main.frag.mp4",
    "mp4/frag/mp3.frag.mp4",
    "mp4/frag/mpeg2video.frag.mp4",
    "mp4/frag/opus.frag.mp4",
    "mp4/frag/vp9.frag.mp4",
    "mp4/frag/vvc.frag.mp4",
    "mp4/cmaf/av-frag.mp4",
];

/// One file that cannot be byte-identical, the exact reason, and a check that
/// the reason really holds for that file.
struct Exception {
    name: &'static str,
    reason: &'static str,
    verify: fn(&[u8]) -> bool,
}

/// `tfhd` flags that state something about the run instead of each sample:
/// `base_data_offset` (0x01), the three sample defaults (0x08/0x10/0x20).
const TFHD_RUN_LAYOUT_BITS: u32 = 0x01 | 0x08 | 0x10 | 0x20;
const FULL_BOX_FLAGS_MASK: u32 = 0x00ff_ffff;

#[derive(Debug, Clone, Copy)]
struct Span {
    start: usize,
    header: usize,
    size: usize,
    fourcc: [u8; 4],
}

fn children(bytes: &[u8], start: usize, end: usize) -> Vec<Span> {
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
        out.push(Span {
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

fn top_boxes(bytes: &[u8]) -> Vec<Span> {
    children(bytes, 0, bytes.len())
}

fn find_child(bytes: &[u8], parent: &Span, fourcc: &[u8; 4]) -> Option<Span> {
    children(
        bytes,
        parent.start + parent.header,
        parent.start + parent.size,
    )
    .into_iter()
    .find(|span| &span.fourcc == fourcc)
}

fn has_top_level(bytes: &[u8], fourcc: &[u8; 4]) -> bool {
    top_boxes(bytes).iter().any(|span| &span.fourcc == fourcc)
}

/// The `flags` of the first fragment's first `tfhd`.
fn first_tfhd_flags(bytes: &[u8]) -> Option<u32> {
    let moof = top_boxes(bytes)
        .into_iter()
        .find(|span| &span.fourcc == b"moof")?;
    let traf = find_child(bytes, &moof, b"traf")?;
    let tfhd = find_child(bytes, &traf, b"tfhd")?;
    let at = tfhd.start + tfhd.header;
    let raw = bytes.get(at..at + 4)?;
    let array: [u8; 4] = raw.try_into().ok()?;
    Some(u32::from_be_bytes(array) & FULL_BOX_FLAGS_MASK)
}

/// The first fragment really does state a run layout in its `tfhd`, which is
/// what the recorded layout has to reproduce.
fn has_tfhd_run_layout(bytes: &[u8]) -> bool {
    first_tfhd_flags(bytes).is_some_and(|flags| flags & TFHD_RUN_LAYOUT_BITS != 0)
}

/// The one fragmented file the writer cannot reproduce: its `sidx` indexes
/// fragments the excerpt does not carry, and its init segment's empty `stsz`
/// declares a per-sample size the `moov` rebuild does not.
const EXCEPTIONS: &[Exception] = &[Exception {
    name: "mp4/frag/mpegh-stereo.frag.mp4",
    reason: "its `sidx` states 183 references while the excerpt carries 6 \
             fragments, so the 177 references to absent segments cannot be \
             regenerated from the written positions; its init segment's empty \
             `stsz` also declares a sample_size of 1, which the `moov` rebuild \
             writes as 0",
    verify: sidx_indexes_absent_segments_and_stsz_states_a_size,
}];

/// The first `stsz` in `moov` that declares a per-sample size while carrying
/// no entry at all.
fn empty_stsz_states_a_size(bytes: &[u8]) -> bool {
    let Some(moov) = top_boxes(bytes)
        .into_iter()
        .find(|span| &span.fourcc == b"moov")
    else {
        return false;
    };
    let head = core::mem::size_of::<u32>();
    for trak in children(bytes, moov.start + moov.header, moov.start + moov.size)
        .into_iter()
        .filter(|span| &span.fourcc == b"trak")
    {
        let Some(stbl) = find_child(bytes, &trak, b"mdia")
            .and_then(|mdia| find_child(bytes, &mdia, b"minf"))
            .and_then(|minf| find_child(bytes, &minf, b"stbl"))
        else {
            continue;
        };
        let Some(stsz) = find_child(bytes, &stbl, b"stsz") else {
            continue;
        };
        let at = stsz.start + stsz.header;
        let Some(fields) = bytes.get(at..at + 3 * head) else {
            continue;
        };
        let sample_size = u32::from_be_bytes([fields[4], fields[5], fields[6], fields[7]]);
        let count = u32::from_be_bytes([fields[8], fields[9], fields[10], fields[11]]);
        if sample_size != 0 && count == 0 && stsz.size == stsz.header + 3 * head {
            return true;
        }
    }
    false
}

/// The source `sidx` names more fragments than the excerpt carries.
fn sidx_indexes_absent_segments(bytes: &[u8]) -> bool {
    let boxes = top_boxes(bytes);
    let Some(sidx) = boxes.iter().find(|span| &span.fourcc == b"sidx") else {
        return false;
    };
    let Ok(index) =
        isobmff::timing::SegmentIndexBox::parse(&bytes[sidx.start..sidx.start + sidx.size])
    else {
        return false;
    };
    let fragments = boxes.iter().filter(|span| &span.fourcc == b"moof").count();
    index.references.len() > fragments
}

fn sidx_indexes_absent_segments_and_stsz_states_a_size(bytes: &[u8]) -> bool {
    sidx_indexes_absent_segments(bytes) && empty_stsz_states_a_size(bytes)
}

/// A fixture's bytes, or `None` (with a loud skip) when a private one is not
/// checked out.
fn load(name: &str) -> Option<Vec<u8>> {
    if let Some(path) = test_fixtures::private(name) {
        return Some(std::fs::read(&path).expect("private fixture"));
    }
    if name.starts_with("mp4/tags/")
        || name.starts_with("mp4/camera/")
        || name.starts_with("mp4/quicktime/")
    {
        eprintln!("SKIP whole_file_round_trip: private fixture {name} is not checked out");
        return None;
    }
    Some(test_fixtures::read(name))
}

/// The `ftyp` brand set of a source file, or this crate's default when it has
/// none.
fn file_type(bytes: &[u8]) -> FileType {
    let Some(ftyp) = top_boxes(bytes)
        .into_iter()
        .find(|span| &span.fourcc == b"ftyp")
    else {
        return FileType::default();
    };
    let box_bytes = &bytes[ftyp.start..ftyp.start + ftyp.size];
    let ftyp = isobmff::segments::FileTypeBox::parse_box(box_bytes).expect("ftyp");
    FileType {
        major_brand: ftyp.major_brand,
        minor_version: ftyp.minor_version,
        compatible_brands: ftyp.compatible_brands.clone(),
        emit_ftyp: true,
    }
}

/// One fragment's `mfhd.sequence_number`, the layout the reader recorded for
/// it, and its samples in decode order.
struct FragmentSamples {
    sequence_number: u32,
    layout: FragmentLayout,
    samples: Vec<SampleRef>,
}

/// Everything one reader run surfaced.
struct Run {
    tracks: Vec<TrackInfo>,
    samples: Vec<SampleRef>,
    fragments: Vec<FragmentSamples>,
    index: FragmentIndex,
    plan: OwnedPlan,
}

/// Drive the reader once, in file order, collecting every track and sample.
fn drive(bytes: &[u8]) -> Run {
    let mut reader = Reader::new(
        ReaderConfig::default()
            .with_total_len(Some(u64::try_from(bytes.len()).expect("length")))
            .with_order(SampleOrder::File),
    );
    let mut tracks = Vec::new();
    let mut samples = Vec::new();
    let mut fragments: Vec<FragmentSamples> = Vec::new();
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
            Step::Fragment(info) => fragments.push(FragmentSamples {
                sequence_number: info.sequence_number,
                layout: info.layout.clone(),
                samples: Vec::new(),
            }),
            Step::Sample(sample) => {
                if let Some(fragment) = fragments.last_mut() {
                    fragment.samples.push(sample.clone());
                }
                samples.push(sample);
            }
            Step::Done => break,
            _ => {}
        }
    }
    let index = reader.fragment_index().clone();
    let plan = reader
        .plan()
        .expect("a progressive or fragmented movie has a plan")
        .to_owned_plan()
        .expect("owned plan");
    Run {
        tracks,
        samples,
        fragments,
        index,
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

/// Rewrite a whole file from the reader's plan and its samples.
fn rebuild(bytes: &[u8], run: &Run) -> Vec<u8> {
    if run.fragments.is_empty() {
        rebuild_progressive(bytes, run)
    } else {
        rebuild_fragmented(bytes, run)
    }
}

fn rebuild_progressive(bytes: &[u8], run: &Run) -> Vec<u8> {
    rebuild_progressive_with_plan(bytes, run, run.plan.clone())
}

/// Rewrite a progressive file from a caller-supplied plan (the typed-entry
/// variant replaces each track's raw `stsd` bytes with its typed serialisation).
fn rebuild_progressive_with_plan(bytes: &[u8], run: &Run, plan: OwnedPlan) -> Vec<u8> {
    let shell = plan.shell().expect("a plan read from a file");
    let boxes = top_boxes(bytes);
    let position = |fourcc: &[u8; 4]| boxes.iter().position(|span| &span.fourcc == fourcc);
    let faststart = match (position(b"moov"), position(b"mdat")) {
        (Some(moov), Some(mdat)) => moov < mdat,
        (Some(_), None) => true,
        _ => false,
    };
    let layout = Layout::new(faststart, Interleave::AsPlanned, Sink::Seekable);
    let header = MovieHeader::with_times(
        shell.mvhd.timescale,
        shell.mvhd.creation_time,
        shell.mvhd.modification_time,
    );
    let mut writer = Writer::progressive(plan, layout, file_type(bytes), header, Limits::default())
        .expect("progressive writer");
    let mut out = vec![0u8; usize::try_from(writer.file_len()).expect("file length")];
    for write in writer.header() {
        place(&mut out, &write);
    }
    for sample in &run.samples {
        for write in writer
            .push(sample.track, sample.index, sample_bytes(bytes, sample))
            .expect("push")
        {
            place(&mut out, &write);
        }
    }
    for write in writer.finish().expect("finish") {
        place(&mut out, &write);
    }
    out
}

/// Whether a sample entry carries a QuickTime-only form this slice serialises.
fn is_quicktime_entry(entry: &isobmff::init_segment::SampleEntryVariant) -> bool {
    use isobmff::init_segment::SampleEntryVariant as E;
    match entry {
        E::Tmcd(_) | E::QuickTimeSound(_) => true,
        E::Mp4a(e) => e.quicktime.is_some(),
        E::Ac3(e) => e.quicktime.is_some(),
        E::Ec3(e) => e.quicktime.is_some(),
        E::Opus(e) => e.quicktime.is_some(),
        E::Flac(e) => e.quicktime.is_some(),
        E::Ac4(e) => e.quicktime.is_some(),
        E::Mha(e) => e.quicktime.is_some(),
        E::Dts(e) => e.quicktime.is_some(),
        _ => false,
    }
}

/// The same plan with every `stsd` entry rebuilt from its typed parse rather
/// than the file's raw bytes. The typed serialisers must reproduce the entry
/// byte for byte for the rewrite to stay identical.
fn typed_plan(run: &Run) -> OwnedPlan {
    let mut tracks = run.plan.tracks().to_vec();
    for (track, info) in tracks.iter_mut().zip(&run.tracks) {
        if track.entries.len() != info.entries.len() {
            continue;
        }
        for (entry, sample_entry) in track.entries.iter_mut().zip(&info.entries) {
            if let Some(typed) = &sample_entry.typed
                && is_quicktime_entry(typed)
            {
                *entry = SampleEntry::Typed(typed.clone());
            }
        }
        if let Some(first) = track.entries.first() {
            track.setup.entry = first.clone();
        }
    }
    let mut plan = OwnedPlan::new(tracks);
    plan.set_shell(run.plan.shell().cloned());
    plan
}

fn rebuild_fragmented(bytes: &[u8], run: &Run) -> Vec<u8> {
    let shell = run.plan.shell().expect("a plan read from a file").clone();
    let setups: Vec<TrackSetup> = run
        .plan
        .tracks()
        .iter()
        .map(|track| track.setup.clone())
        .collect();
    let mut writer = Writer::fragmented(setups, shell.mvhd.timescale, Limits::default())
        .expect("fragmented writer");
    writer.set_file_type(file_type(bytes));
    writer.set_movie_shell(shell);
    writer.set_index_layout(run.index.clone());
    let mut out = writer.init().expect("init segment");
    writer.set_position(u64::try_from(out.len()).expect("init length"));
    for fragment in &run.fragments {
        writer.set_sequence_number(fragment.sequence_number);
        writer.set_fragment_layout(fragment.layout.clone());
        let mut based: Vec<TrackId> = Vec::new();
        for sample in &fragment.samples {
            if !based.contains(&sample.track) {
                based.push(sample.track);
                writer
                    .set_base_decode_time(sample.track, sample.decode_time)
                    .expect("base decode time");
            }
        }
        for sample in &fragment.samples {
            writer
                .push(
                    sample.track,
                    SampleMeta {
                        duration: sample.duration,
                        composition_offset: sample.composition_offset,
                        sync: sample.sync,
                        aux: sample.aux.clone(),
                        description_index: sample.description_index,
                    },
                    sample_bytes(bytes, sample),
                )
                .expect("push");
        }
        if let Some(fragment) = writer.flush().expect("flush") {
            out.extend_from_slice(&fragment.bytes);
        }
    }
    out.extend_from_slice(&writer.trailer().expect("trailer"));
    out
}

/// A track table with the registry parse cleared, so a file read with and
/// without a registry compares equal.
fn without_registry(tracks: &[TrackInfo]) -> Vec<TrackInfo> {
    let mut tracks = tracks.to_vec();
    for track in &mut tracks {
        for entry in &mut track.entries {
            entry.registered = None;
        }
    }
    tracks
}

/// The per-sample facts, keyed by track and index. The file offset is blanked:
/// a rewritten file places its chunks from its own layout.
fn sample_map(bytes: &[u8], run: &Run) -> BTreeMap<(u32, u32), (SampleRef, Vec<u8>)> {
    run.samples
        .iter()
        .map(|sample| {
            let mut keyed = sample.clone();
            keyed.offset = 0;
            (
                (sample.track.value(), sample.index),
                (keyed, sample_bytes(bytes, sample).to_vec()),
            )
        })
        .collect()
}

/// One track's `stsd` entry bytes, in `moov` order.
fn entry_boxes(bytes: &[u8]) -> Vec<Vec<u8>> {
    let Some(moov) = top_boxes(bytes)
        .into_iter()
        .find(|span| &span.fourcc == b"moov")
    else {
        return Vec::new();
    };
    children(bytes, moov.start + moov.header, moov.start + moov.size)
        .into_iter()
        .filter(|span| &span.fourcc == b"trak")
        .filter_map(|trak| {
            let mdia = find_child(bytes, &trak, b"mdia")?;
            let minf = find_child(bytes, &mdia, b"minf")?;
            let stbl = find_child(bytes, &minf, b"stbl")?;
            let stsd = find_child(bytes, &stbl, b"stsd")?;
            Some(bytes[stsd.start..stsd.start + stsd.size].to_vec())
        })
        .collect()
}

/// One track's sample-table four-CCs, in wire order.
fn table_kinds(bytes: &[u8]) -> Vec<Vec<[u8; 4]>> {
    let Some(moov) = top_boxes(bytes)
        .into_iter()
        .find(|span| &span.fourcc == b"moov")
    else {
        return Vec::new();
    };
    children(bytes, moov.start + moov.header, moov.start + moov.size)
        .into_iter()
        .filter(|span| &span.fourcc == b"trak")
        .filter_map(|trak| {
            let mdia = find_child(bytes, &trak, b"mdia")?;
            let minf = find_child(bytes, &mdia, b"minf")?;
            let stbl = find_child(bytes, &minf, b"stbl")?;
            Some(
                children(bytes, stbl.start + stbl.header, stbl.start + stbl.size)
                    .iter()
                    .map(|span| span.fourcc)
                    .collect(),
            )
        })
        .collect()
}

/// One track's sample-table boxes, keyed by four-CC, with `stco`/`co64` left
/// out (their offsets legitimately move).
fn table_boxes(bytes: &[u8]) -> Vec<BTreeMap<[u8; 4], Vec<u8>>> {
    let Some(moov) = top_boxes(bytes)
        .into_iter()
        .find(|span| &span.fourcc == b"moov")
    else {
        return Vec::new();
    };
    let mut tables = Vec::new();
    for trak in children(bytes, moov.start + moov.header, moov.start + moov.size)
        .into_iter()
        .filter(|span| &span.fourcc == b"trak")
    {
        let mdia = find_child(bytes, &trak, b"mdia").expect("mdia");
        let minf = find_child(bytes, &mdia, b"minf").expect("minf");
        let stbl = find_child(bytes, &minf, b"stbl").expect("stbl");
        let mut table = BTreeMap::new();
        for boxed in children(bytes, stbl.start + stbl.header, stbl.start + stbl.size) {
            if &boxed.fourcc == b"stco" || &boxed.fourcc == b"co64" {
                continue;
            }
            table.insert(
                boxed.fourcc,
                bytes[boxed.start..boxed.start + boxed.size].to_vec(),
            );
        }
        tables.push(table);
    }
    tables
}

/// Compare a file the writer cannot reproduce byte for byte: every track,
/// every sample's descriptor and bytes, and every box the writer emitted.
fn assert_structural(name: &str, source: &[u8], written: &[u8]) {
    let source_run = drive(source);
    let written_run = drive(written);
    assert_eq!(
        without_registry(&written_run.tracks),
        without_registry(&source_run.tracks),
        "{name}: tracks"
    );
    assert_eq!(
        entry_boxes(written),
        entry_boxes(source),
        "{name}: sample entries"
    );
    assert_eq!(
        table_kinds(written),
        table_kinds(source),
        "{name}: sample-table box set"
    );
    if !has_top_level(source, b"moof") {
        assert_eq!(
            table_boxes(written),
            table_boxes(source),
            "{name}: sample tables"
        );
    }
    let want = sample_map(source, &source_run);
    let got = sample_map(written, &written_run);
    assert_eq!(got.len(), want.len(), "{name}: sample count");
    for (key, (expected, bytes)) in &want {
        let (actual, actual_bytes) = got
            .get(key)
            .unwrap_or_else(|| panic!("{name}: {key:?} missing"));
        assert_eq!(actual, expected, "{name}: {key:?} descriptor");
        assert_eq!(actual_bytes, bytes, "{name}: {key:?} bytes");
    }
    // Every top-level box the writer emitted appears in the source in the same
    // relative order; the source's extra boxes (an index, `free` padding) are
    // what it did not emit.
    let source_names: Vec<[u8; 4]> = top_boxes(source).iter().map(|span| span.fourcc).collect();
    let written_names: Vec<[u8; 4]> = top_boxes(written).iter().map(|span| span.fourcc).collect();
    let mut cursor = 0usize;
    for name_fourcc in &written_names {
        match source_names[cursor..].iter().position(|c| c == name_fourcc) {
            Some(offset) => cursor += offset + 1,
            None => panic!("{name}: emitted {name_fourcc:?} is not in the source order"),
        }
    }
}

#[test]
fn files_the_writer_can_reproduce_round_trip_byte_identically() {
    let mut identical = 0usize;
    for name in BYTE_IDENTICAL {
        let Some(source) = load(name) else {
            continue;
        };
        let run = drive(&source);
        assert!(!run.tracks.is_empty(), "{name}: no tracks");
        let written = rebuild(&source, &run);
        if written != source {
            let at = written
                .iter()
                .zip(&source)
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| written.len().min(source.len()));
            panic!(
                "{name}: not byte-identical at {at} (written {} bytes, source {} bytes)",
                written.len(),
                source.len()
            );
        }
        identical += 1;
    }
    assert!(
        identical >= MIN_BYTE_IDENTICAL,
        "{identical} files round-tripped byte-identically, expected at least {MIN_BYTE_IDENTICAL}"
    );
}

/// Every fragmented file that now round-trips byte-identically really does
/// state its sample layout the way this crate's canonical form does not: a
/// `tfhd` carrying run defaults or an explicit base, and a trailing `mfra`. A
/// green round trip on a canonical-layout file would prove nothing about the
/// recorded-layout path.
#[test]
fn fragmented_round_trips_exercise_a_recorded_layout() {
    for name in [
        "mp4/frag/av1.frag.mp4",
        "mp4/frag/flac.frag.mp4",
        "mp4/frag/h264-high.frag.mp4",
        "mp4/frag/hevc-main.frag.mp4",
        "mp4/frag/mp3.frag.mp4",
        "mp4/frag/mpeg2video.frag.mp4",
        "mp4/frag/opus.frag.mp4",
        "mp4/frag/vp9.frag.mp4",
        "mp4/frag/vvc.frag.mp4",
        "mp4/cmaf/av-frag.mp4",
    ] {
        let Some(source) = load(name) else {
            continue;
        };
        assert!(
            has_tfhd_run_layout(&source),
            "{name}: the first tfhd states no run layout"
        );
        assert!(
            has_top_level(&source, b"mfra"),
            "{name}: the file does not end with an mfra"
        );
        let run = drive(&source);
        assert!(
            run.fragments
                .iter()
                .all(|fragment| !fragment.layout.tracks.is_empty()),
            "{name}: a fragment recorded no track layout"
        );
        assert!(
            run.index.mfra.is_some(),
            "{name}: the walk recorded no mfra form"
        );
    }
}

/// Rebuild every `.mov` track's `stsd` from the typed parse (not its carried
/// bytes) and prove the typed serialisers still reproduce the file exactly.
#[test]
fn quicktime_files_round_trip_from_their_typed_entries() {
    for name in [
        "mp4/quicktime/lavf-aac-tmcd.mov",
        "mp4/quicktime/lavf-pcm24.mov",
        "mp4/quicktime/iphone-x-live-photo.mov",
    ] {
        let Some(source) = load(name) else {
            continue;
        };
        let run = drive(&source);
        let plan = typed_plan(&run);
        let written = rebuild_progressive_with_plan(&source, &run, plan);
        if written != source {
            let at = written
                .iter()
                .zip(&source)
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| written.len().min(source.len()));
            panic!(
                "{name}: typed rewrite differs at {at} (written {} bytes, source {} bytes)",
                written.len(),
                source.len()
            );
        }
    }
}

#[test]
fn a_file_that_cannot_be_reproduced_is_listed_and_still_matches_structurally() {
    for exception in EXCEPTIONS {
        let Some(source) = load(exception.name) else {
            continue;
        };
        assert!(
            (exception.verify)(&source),
            "{}: the recorded reason no longer holds: {}",
            exception.name,
            exception.reason
        );
        let run = drive(&source);
        let written = rebuild(&source, &run);
        assert_ne!(
            written, source,
            "{}: now round-trips byte-identically, so it must leave the exception table",
            exception.name
        );
        assert_structural(exception.name, &source, &written);
    }
}

#[test]
fn every_exception_names_an_exact_reason() {
    for exception in EXCEPTIONS {
        assert!(
            !exception.reason.is_empty(),
            "{}: no reason recorded",
            exception.name
        );
        assert!(
            !BYTE_IDENTICAL.contains(&exception.name),
            "{}: listed twice",
            exception.name
        );
    }
}

/// The reader must never ask for a byte inside an `mdat` payload.
#[test]
fn no_sample_byte_is_requested_while_reading_structure() {
    let source = test_fixtures::read("mp4/progressive/av-prog.mp4");
    let mut reader = Reader::new(
        ReaderConfig::default()
            .with_total_len(Some(u64::try_from(source.len()).expect("length")))
            .with_order(SampleOrder::File),
    );
    let mut samples: Vec<SampleRef> = Vec::new();
    loop {
        match reader.step().expect("reader step") {
            Step::Need(range) => {
                for sample in &samples {
                    let start = sample.offset;
                    let end = start + u64::from(sample.size);
                    assert!(
                        range.end() <= start || range.offset >= end,
                        "structure request {range:?} overlaps sample {} of track {}",
                        sample.index,
                        sample.track.value()
                    );
                }
                let start = usize::try_from(range.offset).expect("offset");
                let end = usize::try_from(range.end()).expect("end").min(source.len());
                reader
                    .supply(range.offset, &source[start..end])
                    .expect("supply");
            }
            Step::Sample(sample) => samples.push(sample),
            Step::Done => break,
            _ => {}
        }
    }
    assert!(!samples.is_empty(), "the fixture has samples");
}

/// The plan an untouched read exposes carries the movie shell, which is what
/// makes a byte-identical rewrite possible.
#[test]
fn a_plan_read_from_a_file_carries_the_movie_shell() {
    let source = test_fixtures::read("mp4/progressive/av-faststart.mp4");
    let run = drive(&source);
    let shell = run.plan.shell().expect("shell");
    let movie = MovieBox::parse(isobmff::box_types::find_box(&source, b"moov").expect("moov"))
        .expect("parse");
    assert_eq!(shell.mvhd, movie.mvhd, "the movie header is carried");
    assert!(!shell.opaque.is_empty(), "the moov udta is carried");
    assert!(
        run.plan
            .tracks()
            .iter()
            .all(|track| track.setup.shell.is_some()),
        "every track keeps its parsed `trak`"
    );
}
