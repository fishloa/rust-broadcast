//! A generated 1 GiB MPEG-2 TS → progressive MP4 with peak live memory bounded
//! by the sample plan plus one interleave window (issues #4057, #953).
//!
//! The file is generated, never committed: `test_harness::big_media` repeats a
//! real capture's samples to the target size one pass at a time. Both passes
//! then stream it — [`StreamingTsDemux`] feeds [`PlanPass`], then the same demux
//! feeds [`ProgressiveMuxStage`], whose output is written to disk as it
//! arrives.
//!
//! `test_alloc::ThreadCapped` caps this thread's live bytes: an allocation past
//! the cap aborts, so the bound is enforced, not merely observed. Nothing the
//! two passes hold scales with the input's byte size: the plan is 4 bytes of
//! `stsz` per sample, and the writer builds one `moov` (a second `stsz`) and
//! hands it over as one `Bytes`. The cap is therefore a fixed number of
//! plan-sized tables, not a fraction of the 4 GiB file.

use broadcast_common::stage::{Stage, Timestamp};
use broadcast_common::timescale::Timescale;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use test_alloc::ThreadCapped;
use transmux::mux_stage::MuxInput;
use transmux::{DemuxEvent, StreamingTsDemux};

/// Sample count a 4 GiB file of the capture carries, rounded up.
const MAX_SAMPLES: usize = 1_100_000;
/// Plan bytes for [`MAX_SAMPLES`]: 4 bytes of `stsz` per sample plus the
/// run-length `stts`/`stss` and one `stsd` entry per track.
const PLAN_ALLOWANCE: usize = MAX_SAMPLES * 4 + 128 * 1024;
/// The enforced cap on this thread's live bytes.
///
/// The writer's `moov` carries its own `stsz` (one entry per sample), so
/// building it, its `stco`, and the `Bytes` message it is handed all scale
/// with the plan; the measured peak for the 4 GiB input is 54 740 609 bytes,
/// about 12 plan-sized tables. Nothing here scales with the input's byte size,
/// which is the property that lets a 4 GiB stream through.
const CAP_BYTES: usize = 64 * 1024 * 1024;

#[global_allocator]
static CAP: ThreadCapped<CAP_BYTES> = ThreadCapped::new();

/// Bytes per read from the generated file.
const CHUNK: usize = 188 * 4096;
/// A TS-sourced movie's timescale: the demuxer's 90 kHz video clock.
const MOVIE_TIMESCALE: u32 = 90_000;
/// The caller's append-only reorder window (one interleave window).
const REORDER_WINDOW: u64 = 1024 * 1024;
/// Everything in the file that is not sample payload: `ftyp`, `moov` (its
/// `stsz` is one entry per sample) and the `mdat` header.
const HEADER_ALLOWANCE: u64 = 16 * 1024 * 1024;

/// Write every output the stage has ready to `file`, at its offset.
fn write_output(
    file: &mut File,
    end: &mut u64,
    stage: &mut transmux::progressive::ProgressiveMuxStage,
) -> Result<(), transmux::Error> {
    while let Some(item) = stage.poll() {
        match item {
            transmux::mux_stage::MuxOutput::Bytes(chunk) => {
                file.seek(SeekFrom::Start(*end)).expect("seek");
                file.write_all(&chunk).expect("write");
                *end += u64::try_from(chunk.len()).expect("length fits");
            }
            transmux::mux_stage::MuxOutput::BytesAt { offset, bytes } => {
                file.seek(SeekFrom::Start(offset)).expect("seek");
                file.write_all(&bytes).expect("write");
                *end = (*end).max(offset + u64::try_from(bytes.len()).expect("length fits"));
            }
            _ => {}
        }
    }
    Ok(())
}

/// The capture the generated file repeats: one AVC video track with ~10 KiB
/// frames and one AAC audio track, so a gigabyte is a few hundred thousand
/// samples rather than millions.
const SOURCE: &str = "ts/gulli-opengop.ts";

/// Where the generated file goes (under the gitignored `target/`).
fn generated_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target/progressive-plan-big")
        .join("big.ts")
}

/// The generated file's size: 4 GiB under `RUST_BROADCAST_SOAK`, 1 GiB for a
/// quick run.
fn target_bytes() -> u64 {
    if std::env::var_os("RUST_BROADCAST_SOAK").is_some() {
        4 * 1024 * 1024 * 1024
    } else {
        1024 * 1024 * 1024
    }
}

/// Stream `path` through a TS demux into `feed`, in file order, carrying only
/// `kept` tracks (the ones the muxers can place).
fn drive(
    path: &std::path::Path,
    kept: &[transmux::TrackId],
    feed: &mut impl FnMut(MuxInput) -> Result<(), transmux::Error>,
) -> Result<(), transmux::Error> {
    let mut file = File::open(path).expect("open the generated file");
    let mut demux = StreamingTsDemux::new();
    let mut chunk = vec![0u8; CHUNK];
    loop {
        let read = file.read(&mut chunk).expect("read the generated file");
        if read == 0 {
            break;
        }
        demux.feed(&chunk[..read]).expect("feed the demux");
        drain(&mut demux, kept, feed)?;
    }
    demux.finish().expect("finish the demux");
    drain(&mut demux, kept, feed)
}

fn drain(
    demux: &mut StreamingTsDemux,
    kept: &[transmux::TrackId],
    feed: &mut impl FnMut(MuxInput) -> Result<(), transmux::Error>,
) -> Result<(), transmux::Error> {
    while let Some(event) = demux.poll() {
        match event {
            DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec)
                if kept.contains(&spec.track_id) =>
            {
                feed(MuxInput::Track(Box::new(spec)))?;
            }
            DemuxEvent::Sample {
                track_id, sample, ..
            } if kept.contains(&track_id) => {
                feed(MuxInput::Sample {
                    track: track_id,
                    sample,
                })?;
            }
            _ => {}
        }
    }
    Ok(())
}

#[test]
fn a_generated_gibibyte_streams_into_a_progressive_file() {
    if std::env::var_os("RUST_BROADCAST_SOAK").is_none() {
        eprintln!(
            "SKIPPED a_generated_gibibyte_streams_into_a_progressive_file: the 4 GiB run \
             measures 164 s; set RUST_BROADCAST_SOAK=1 for the nightly soak"
        );
        return;
    }
    let source = test_fixtures::path(SOURCE);
    let small = test_fixtures::read(SOURCE);
    let kept: Vec<transmux::TrackId> = test_harness::collect::media_from_ts(small.as_slice())
        .expect("demux the source")
        .tracks
        .iter()
        .filter(|t| t.spec.config.is_muxable_in_bmff())
        .map(|t| t.spec.track_id)
        .collect();
    drop(small);
    assert!(!kept.is_empty(), "the source has a muxable track");

    let path = generated_path();
    std::fs::create_dir_all(path.parent().expect("parent")).expect("create the target dir");
    let target = target_bytes();
    let written = test_harness::big_media::write_big_ts(&path, &source, target);
    assert!(
        written.bytes >= 1024 * 1024 * 1024,
        "the generated file is at least 1 GiB: {}",
        written.bytes
    );
    eprintln!("generated {} bytes of TS", written.bytes);

    let movie_timescale = Timescale::try_new(MOVIE_TIMESCALE).expect("a non-zero timescale");
    let plan = {
        let mut pass = transmux::plan_pass::PlanPass::new(movie_timescale);
        drive(&path, &kept, &mut |input| pass.feed(input, Timestamp::ZERO)).expect("plan pass");
        pass.finish().expect("finish the plan pass");
        let plan = pass.poll().expect("a plan");
        let samples = plan.sample_count();
        eprintln!(
            "plan: {samples} samples over {} tracks = {} bytes of stsz \
             (allowance {PLAN_ALLOWANCE} bytes)",
            plan.tracks().len(),
            samples * 4
        );
        assert!(
            samples <= MAX_SAMPLES as u64,
            "plan sample count {samples} exceeds the allowance {MAX_SAMPLES}"
        );
        plan
    };
    let payload: u64 = plan
        .tracks()
        .iter()
        .map(|track| {
            (0..track.tables.sample_count)
                .map(|index| {
                    u64::from(
                        track
                            .tables
                            .size(index)
                            .expect("every planned sample has a size"),
                    )
                })
                .sum::<u64>()
        })
        .sum();

    let layout = isobmff::writer::Layout::new(
        true,
        isobmff::writer::Interleave::ByDuration { millis: 500 },
        isobmff::writer::Sink::AppendOnly {
            reorder_bytes: REORDER_WINDOW,
        },
    );
    let mut stage = transmux::progressive::ProgressiveMuxStage::new(
        plan,
        layout,
        isobmff::writer::MovieHeader::new(movie_timescale),
    );
    let output = path.with_extension("mp4");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&output)
        .expect("create the output file");
    let mut end = 0u64;
    {
        let mut feed = |input| {
            stage.feed(input, Timestamp::ZERO)?;
            write_output(&mut file, &mut end, &mut stage)
        };
        drive(&path, &kept, &mut feed).expect("mux the planning stream");
    }
    stage.finish().expect("finish the mux");
    write_output(&mut file, &mut end, &mut stage).expect("write the trailer");
    file.flush().expect("flush");
    let size = file.metadata().expect("metadata").len();
    assert!(
        size >= payload,
        "the file holds every planned sample: {size} < {payload}"
    );
    assert!(
        size - payload <= HEADER_ALLOWANCE,
        "the boxes take {} bytes (over {HEADER_ALLOWANCE})",
        size - payload
    );
    eprintln!("cap: {CAP_BYTES} bytes (plan allowance {PLAN_ALLOWANCE}); wrote {size} bytes");
    std::fs::remove_file(&output).ok();
    std::fs::remove_file(&path).ok();
}
