#![no_main]

//! Whole-file read-write-read: the input is a file image, not a box.
//!
//! A strict read that reaches [`Step::Done`] with every sample inside the input
//! is rewritten through the writer and read back. The rewrite must describe the
//! same tracks, the same sample descriptors and the same sample bytes; the
//! write path is the one place an arbitrary input reaches the *output* code, so
//! this is where a reader/writer disagreement (a size the writer trusts that
//! the reader derived differently) would land.

use std::collections::BTreeMap;

use isobmff::frag_layout::{FragmentIndex, FragmentLayout};
use isobmff::plan::OwnedPlan;
use isobmff::sample_table::SampleRef;
use isobmff::writer::{
    FileType, Interleave, Layout, MovieHeader, SampleEntry, SampleMeta, Sink, TrackSetup, Write,
    Writer,
};
use isobmff::{Limits, Reader, ReaderConfig, SampleOrder, Step, TrackId, TrackInfo};
use libfuzzer_sys::fuzz_target;

/// A hostile input cannot describe more steps than this.
const MAX_STEPS: usize = 1 << 16;
/// The rewrite is materialised whole to read it back; refuse anything larger.
const MAX_FILE_BYTES: u64 = 8 << 20;

fn fuzz_limits() -> Limits {
    Limits::default()
        .with_max_depth(8)
        .with_max_moov_bytes(1 << 20)
        .with_max_samples_per_track(1 << 12)
        .with_max_sample_bytes(1 << 16)
        .with_max_fragment_bytes(1 << 20)
        .with_max_metadata_item_bytes(1 << 16)
}

struct Run {
    tracks: Vec<TrackInfo>,
    samples: Vec<SampleRef>,
    fragments: Vec<Fragment>,
    index: FragmentIndex,
    plan: OwnedPlan,
}

/// One fragment as the reader reported it: its sequence number, the layout it
/// states, and its samples.
struct Fragment {
    sequence_number: u32,
    layout: FragmentLayout,
    samples: Vec<SampleRef>,
}

fn drive(data: &[u8]) -> Option<Run> {
    // A rewrite is exact only for what was read cleanly: the tolerant reader
    // deliberately drops what a damaged table cannot address, and a plan built
    // from those tables would describe samples the read never delivered.
    let mut reader = Reader::new(
        ReaderConfig::default()
            .with_total_len(Some(data.len() as u64))
            .with_order(SampleOrder::File)
            .with_mode(isobmff::Mode::Strict)
            .with_limits(fuzz_limits()),
    );
    let mut tracks = Vec::new();
    let mut samples: Vec<SampleRef> = Vec::new();
    let mut fragments: Vec<Fragment> = Vec::new();
    for _ in 0..MAX_STEPS {
        match reader.step().ok()? {
            Step::Need(range) => {
                let start = usize::try_from(range.offset).ok()?;
                let end = usize::try_from(range.end()).ok()?.min(data.len());
                reader.supply(range.offset, data.get(start..end)?).ok()?;
            }
            Step::Tracks(step_tracks) => tracks = step_tracks.to_vec(),
            Step::Fragment(info) => fragments.push(Fragment {
                sequence_number: info.sequence_number,
                layout: info.layout.clone(),
                samples: Vec::new(),
            }),
            Step::Sample(sample) => {
                let end = sample.offset.checked_add(u64::from(sample.size))?;
                if end > data.len() as u64 {
                    return None;
                }
                if let Some(fragment) = fragments.last_mut() {
                    fragment.samples.push(sample.clone());
                }
                samples.push(sample);
            }
            Step::Done => {
                let index = reader.fragment_index().clone();
                let plan = reader.plan()?.to_owned_plan().ok()?;
                return Some(Run {
                    tracks,
                    samples,
                    fragments,
                    index,
                    plan,
                });
            }
            _ => {}
        }
    }
    None
}

fn sample_bytes<'a>(data: &'a [u8], sample: &SampleRef) -> Option<&'a [u8]> {
    let start = usize::try_from(sample.offset).ok()?;
    let end = start.checked_add(usize::try_from(sample.size).ok()?)?;
    data.get(start..end)
}

fn place(out: &mut [u8], write: &Write<'_>) -> Option<()> {
    let start = usize::try_from(write.offset).ok()?;
    let end = start.checked_add(write.bytes.len())?;
    out.get_mut(start..end)?.copy_from_slice(&write.bytes);
    Some(())
}

fn file_type(data: &[u8]) -> FileType {
    let Some(ftyp) = isobmff::box_types::find_box(data, b"ftyp") else {
        return FileType::default();
    };
    match isobmff::segments::FileTypeBox::parse_box(ftyp) {
        Ok(ftyp) => FileType {
            major_brand: ftyp.major_brand,
            minor_version: ftyp.minor_version,
            compatible_brands: ftyp.compatible_brands.clone(),
            emit_ftyp: true,
        },
        Err(_) => FileType::default(),
    }
}

fn rewrite(data: &[u8], run: &Run) -> Option<Vec<u8>> {
    if run.fragments.is_empty() {
        rewrite_progressive(data, run)
    } else {
        rewrite_fragmented(data, run)
    }
}

/// Whether a sample entry carries a QuickTime form whose serialisers this
/// target exercises.
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

/// The plan with every QuickTime `stsd` entry rebuilt from its typed parse.
fn typed_plan(run: &Run) -> Option<OwnedPlan> {
    let mut tracks = run.plan.tracks().to_vec();
    for (track, info) in tracks.iter_mut().zip(&run.tracks) {
        if track.entries.len() != info.entries.len() {
            return None;
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
    Some(plan)
}

fn rewrite_progressive(data: &[u8], run: &Run) -> Option<Vec<u8>> {
    let plan = typed_plan(run)?;
    let shell = plan.shell()?;
    let layout = Layout::new(true, Interleave::AsPlanned, Sink::Seekable);
    let header = MovieHeader::with_times(
        shell.mvhd.timescale,
        shell.mvhd.creation_time,
        shell.mvhd.modification_time,
    );
    let mut writer =
        Writer::progressive(plan.clone(), layout, file_type(data), header, fuzz_limits()).ok()?;
    let len = writer.file_len();
    if len > MAX_FILE_BYTES {
        return None;
    }
    let mut out = vec![0u8; usize::try_from(len).ok()?];
    for write in writer.header() {
        place(&mut out, &write)?;
    }
    for sample in &run.samples {
        let bytes = sample_bytes(data, sample)?;
        for write in writer.push(sample.track, sample.index, bytes).ok()? {
            place(&mut out, &write)?;
        }
    }
    for write in writer.finish().ok()? {
        place(&mut out, &write)?;
    }
    Some(out)
}

fn rewrite_fragmented(data: &[u8], run: &Run) -> Option<Vec<u8>> {
    let plan = typed_plan(run)?;
    let shell = plan.shell()?.clone();
    let setups: Vec<TrackSetup> = plan
        .tracks()
        .iter()
        .map(|track| track.setup.clone())
        .collect();
    let mut writer = Writer::fragmented(setups, shell.mvhd.timescale, fuzz_limits()).ok()?;
    writer.set_file_type(file_type(data));
    writer.set_movie_shell(shell);
    let init = writer.init().ok()?;
    if init.len() as u64 > MAX_FILE_BYTES {
        return None;
    }
    writer.set_index_layout(run.index.clone());
    writer.set_position(init.len() as u64);
    let mut out = init;
    for fragment in &run.fragments {
        writer.set_sequence_number(fragment.sequence_number);
        writer.set_fragment_layout(fragment.layout.clone());
        let samples = &fragment.samples;
        let mut based: Vec<TrackId> = Vec::new();
        for sample in samples {
            if !based.contains(&sample.track) {
                based.push(sample.track);
                writer
                    .set_base_decode_time(sample.track, sample.decode_time)
                    .ok()?;
            }
        }
        for sample in samples {
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
                    sample_bytes(data, sample)?,
                )
                .ok()?;
        }
        let fragment = writer.flush().ok()??;
        out.extend_from_slice(&fragment.bytes);
        if out.len() as u64 > MAX_FILE_BYTES {
            return None;
        }
    }
    out.extend_from_slice(&writer.trailer().ok()?);
    Some(out)
}

/// The track facts that live on the wire. `typed` and `registered` are two
/// *parses* of an entry's carried bytes (one of which a malformed `stsd` can
/// suppress), so they are derived data: the assertion compares the four-CC and
/// the bytes, which is what the rewrite has to preserve.
fn wire_tracks(mut tracks: Vec<TrackInfo>) -> Vec<TrackInfo> {
    for track in &mut tracks {
        for entry in &mut track.entries {
            entry.typed = None;
            entry.registered = None;
        }
    }
    tracks
}

fn descriptors(data: &[u8], run: &Run) -> BTreeMap<(u32, u32), (SampleRef, Vec<u8>)> {
    run.samples
        .iter()
        .filter_map(|sample| {
            let mut keyed = sample.clone();
            keyed.offset = 0;
            Some((
                (sample.track.value(), sample.index),
                (keyed, sample_bytes(data, sample)?.to_vec()),
            ))
        })
        .collect()
}

fuzz_target!(|data: &[u8]| {
    let Some(source) = drive(data) else {
        return;
    };
    let Some(rewritten) = rewrite(data, &source) else {
        return;
    };
    let Some(read_back) = drive(&rewritten) else {
        panic!("the writer's output does not read back");
    };
    assert_eq!(
        wire_tracks(read_back.tracks.clone()),
        wire_tracks(source.tracks.clone()),
        "the rewrite describes different tracks"
    );
    let want = descriptors(data, &source);
    let got = descriptors(&rewritten, &read_back);
    assert_eq!(
        got.len(),
        want.len(),
        "the rewrite describes a different sample count"
    );
    for (key, expected) in &want {
        let actual = got
            .get(key)
            .unwrap_or_else(|| panic!("the rewrite has no sample {key:?}"));
        assert_eq!(
            actual, expected,
            "the rewrite describes sample {key:?} differently"
        );
    }
});
