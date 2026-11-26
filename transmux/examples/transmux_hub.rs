//! `transmux` quickstart — the any-to-any container hub in one flow.
//!
//! transmux is a *samples-in* container multiplexer: a streaming demux stage
//! parses a source container into a neutral [`DemuxEvent`] vocabulary (track
//! announcements plus coded access units and their timing), and a muxer `Stage`
//! renders that same sample stream back into an output container. Every muxer is
//! a `Stage<In = MuxInput, Out = MuxOutput>` over `mux_stage`. Coded samples pass
//! through opaque; transmux never encodes or decodes media, and no input is
//! ever held whole.
//!
//! This example drives the `{TS} → {CMAF}` path in bounded memory:
//!
//! 1. Open a real H.264 + AAC MPEG-2 Transport Stream fixture and read it in
//!    64 KiB pieces.
//! 2. Feed each piece to [`StreamingTsDemux`], printing every track's codec,
//!    geometry and a running sample count, then hand each sample straight to
//!    [`CmafMuxStage`].
//! 3. Report the initialization segment and media segment sizes the muxer
//!    emitted.
//!
//! Run it with:
//!
//! ```text
//! cargo run -p transmux --example transmux_hub
//! ```

use std::io::Read;
use std::path::PathBuf;

use broadcast_common::Timestamp;
use broadcast_common::stage::Stage;
use transmux::mux_stage::{MuxInput, MuxOutput};
use transmux::pipeline::CodecConfig;
use transmux::{
    CmafMuxStage, DemuxEvent, FragmentCut, StreamingTsDemux, TrackSpec, VIDEO_CLOCK_TIMESCALE,
};

/// Size of one bounded read from the input file.
const READ_BUF: usize = 64 * 1024;

/// Locate a fixture under the workspace `fixtures/` directory (a sibling of the
/// crate's `CARGO_MANIFEST_DIR`). Fixtures are read at runtime via `std::fs`
/// (never `include_bytes!`, which would bloat the published crate).
fn fixture_path(rel: &str) -> PathBuf {
    test_fixtures::path(rel)
}

/// One-line human summary of a track's codec configuration.
fn describe(config: &CodecConfig) -> String {
    match config {
        CodecConfig::Avc { width, height, .. } => format!("H.264/AVC video {width}x{height}"),
        CodecConfig::Hevc { width, height, .. } => format!("H.265/HEVC video {width}x{height}"),
        CodecConfig::Vvc { width, height, .. } => format!("H.266/VVC video {width}x{height}"),
        CodecConfig::Av1 { width, height, .. } => format!("AV1 video {width}x{height}"),
        CodecConfig::Vp9 { width, height, .. } => format!("VP9 video {width}x{height}"),
        CodecConfig::Mpeg2Video { width, height, .. } => {
            format!("MPEG-2 video {width}x{height}")
        }
        CodecConfig::Aac {
            channel_count,
            sample_rate,
            ..
        } => format!("AAC audio {channel_count}ch @ {sample_rate} Hz"),
        CodecConfig::Ac3 {
            channel_count,
            sample_rate,
            ..
        } => format!("AC-3 audio {channel_count}ch @ {sample_rate} Hz"),
        CodecConfig::Eac3 {
            channel_count,
            sample_rate,
            ..
        } => format!("E-AC-3 audio {channel_count}ch @ {sample_rate} Hz"),
        CodecConfig::Opus {
            channel_count,
            sample_rate,
            ..
        } => format!("Opus audio {channel_count}ch @ {sample_rate} Hz"),
        other => format!("{other:?}"),
    }
}

/// A track announcement kept for the end-of-run summary.
struct TrackSummary {
    spec: TrackSpec,
    samples: u64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = fixture_path("ts/h264-aac.ts");
    let mut file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(err) => {
            // A fixture-less checkout still builds and runs cleanly (exit 0).
            println!(
                "fixture {} unavailable ({err}); nothing to do.",
                path.display()
            );
            return Ok(());
        }
    };

    let specs = first_pass_specs(&path)?;
    let mut mux = CmafMuxStage::new(VIDEO_CLOCK_TIMESCALE, FragmentCut::OnFlush, 1);
    for spec in &specs {
        mux.feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)?;
    }
    let mut demux = StreamingTsDemux::new();
    let mut tracks: Vec<TrackSummary> = specs
        .iter()
        .map(|spec| TrackSummary {
            spec: spec.clone(),
            samples: 0,
        })
        .collect();
    let mut init_len = 0usize;
    let mut media_len = 0usize;
    let mut buf = [0u8; READ_BUF];

    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        demux.feed(&buf[..read])?;
        pump(
            &mut demux,
            &mut mux,
            &mut tracks,
            &mut init_len,
            &mut media_len,
        )?;
    }
    demux.finish()?;
    pump(
        &mut demux,
        &mut mux,
        &mut tracks,
        &mut init_len,
        &mut media_len,
    )?;
    mux.finish()?;
    while let Some(output) = mux.poll() {
        match output {
            MuxOutput::Init(bytes) => init_len += bytes.len(),
            MuxOutput::Segment(segment) => media_len += segment.bytes.len(),
            _ => {}
        }
    }

    println!(
        "IR: {} track(s), movie timescale {} ticks/s",
        tracks.len(),
        VIDEO_CLOCK_TIMESCALE.get()
    );
    for entry in &tracks {
        println!(
            "  track {:>2}: {:<28} {:>5} samples @ {} Hz",
            entry.spec.track_id,
            describe(&entry.spec.config),
            entry.samples,
            entry.spec.timescale.get(),
        );
    }
    println!();
    println!("CMAF init segment : {init_len:>7} bytes");
    println!("CMAF media segment: {media_len:>7} bytes");
    println!("CMAF total        : {:>7} bytes", init_len + media_len);
    Ok(())
}

/// Drain the demux's events into the summary and the muxer stage.
fn pump(
    demux: &mut StreamingTsDemux,
    mux: &mut CmafMuxStage,
    tracks: &mut [TrackSummary],
    init_len: &mut usize,
    media_len: &mut usize,
) -> Result<(), Box<dyn std::error::Error>> {
    while let Some(event) = demux.poll_event() {
        match event {
            DemuxEvent::TrackUpdated(spec) => {
                if let Some(entry) = tracks.iter_mut().find(|t| t.spec.track_id == spec.track_id) {
                    entry.spec = spec;
                }
            }
            DemuxEvent::Sample {
                track_id, sample, ..
            } => {
                if let Some(entry) = tracks.iter_mut().find(|t| t.spec.track_id == track_id) {
                    entry.samples += 1;
                }
                mux.feed(
                    MuxInput::Sample {
                        track: track_id,
                        sample,
                    },
                    Timestamp::ZERO,
                )?;
            }
            DemuxEvent::Discontinuity { .. } => {
                mux.feed(MuxInput::Discontinuity { track: None }, Timestamp::ZERO)?;
            }
            _ => {}
        }
        while let Some(output) = mux.poll() {
            match output {
                MuxOutput::Init(bytes) => *init_len += bytes.len(),
                MuxOutput::Bytes(bytes) => *media_len += bytes.len(),
                MuxOutput::Segment(segment) => *media_len += segment.bytes.len(),
                _ => {}
            }
        }
    }
    Ok(())
}

/// One bounded metadata pass: the track set the demux announces before its
/// first sample (and, when it emits `TracksResolved` early, just that far).
fn first_pass_specs(path: &PathBuf) -> Result<Vec<TrackSpec>, Box<dyn std::error::Error>> {
    let mut file = std::fs::File::open(path)?;
    let mut demux = StreamingTsDemux::new();
    let mut specs: Vec<TrackSpec> = Vec::new();
    let mut buf = [0u8; READ_BUF];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        demux.feed(&buf[..read])?;
        while let Some(event) = demux.poll_event() {
            match event {
                DemuxEvent::TrackAdded(spec) => specs.push(spec),
                DemuxEvent::TrackUpdated(spec) => {
                    if let Some(entry) = specs.iter_mut().find(|s| s.track_id == spec.track_id) {
                        *entry = spec;
                    }
                }
                DemuxEvent::TracksResolved { .. } => return Ok(specs),
                _ => {}
            }
        }
    }
    demux.finish()?;
    while let Some(event) = demux.poll_event() {
        if let DemuxEvent::TrackAdded(spec) = event {
            specs.push(spec);
        }
    }
    Ok(specs)
}
