//! `transmux` cross-container walkthrough — one stream, many outputs.
//!
//! The whole point of the hub is that a source container is demuxed *once* into
//! the neutral [`DemuxEvent`] sample stream, then that one stream feeds any
//! number of muxer stages and manifest renderers — no re-demux, no re-encoding,
//! and no whole-input buffer.
//!
//! This example drives `{WebM} → {DASH MPD + CMAF}` in bounded memory:
//!
//! 1. Open a real VP9 + Opus WebM fixture and read it in 64 KiB pieces.
//! 2. Feed each piece to [`MatroskaDemux`](transmux::MatroskaDemux), then hand
//!    each sample straight to [`CmafMuxStage`](transmux::CmafMuxStage).
//! 3. Render an MPEG-DASH `.mpd` (printed in full) from the track specs and the
//!    segment summaries the muxer emitted — never from the samples.
//!
//! Run it with:
//!
//! ```text
//! cargo run -p transmux --example transmux_any_to_any
//! ```

use std::io::Read;
use std::path::PathBuf;

use broadcast_common::Timestamp;
use broadcast_common::stage::Stage;
use transmux::matroska_demux::{IR_TIMESCALE, MatroskaDemux, MatroskaDemuxConfig, MatroskaInput};
use transmux::mux_stage::{MuxInput, MuxOutput};
use transmux::pipeline::CodecConfig;
use transmux::segment_log::SegmentSummaryLog;
use transmux::{CmafMuxStage, DashPackager, DemuxEvent, FragmentCut, TrackSpec};

/// Size of one bounded read from the input file.
const READ_BUF: usize = 64 * 1024;

fn fixture_path(rel: &str) -> PathBuf {
    test_fixtures::path(rel)
}

/// One-line human summary of a track's codec configuration.
fn describe(config: &CodecConfig) -> String {
    match config {
        CodecConfig::Vp9 { width, height, .. } => format!("VP9 video {width}x{height}"),
        CodecConfig::Vp8 { width, height, .. } => format!("VP8 video {width}x{height}"),
        CodecConfig::Av1 { width, height, .. } => format!("AV1 video {width}x{height}"),
        CodecConfig::Avc { width, height, .. } => format!("H.264/AVC video {width}x{height}"),
        CodecConfig::Opus {
            channel_count,
            sample_rate,
            ..
        } => format!("Opus audio {channel_count}ch @ {sample_rate} Hz"),
        CodecConfig::Aac {
            channel_count,
            sample_rate,
            ..
        } => format!("AAC audio {channel_count}ch @ {sample_rate} Hz"),
        other => format!("{other:?}"),
    }
}

/// A track announcement kept for the end-of-run summary.
struct TrackSummary {
    spec: TrackSpec,
    samples: u64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = fixture_path("mkv/vp9-opus.webm");
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

    let mut demux = MatroskaDemux::sequential(MatroskaDemuxConfig::default());
    let mut mux = CmafMuxStage::new(IR_TIMESCALE, FragmentCut::OnFlush, 1);
    let mut tracks: Vec<TrackSummary> = Vec::new();
    let mut log = SegmentSummaryLog::new();
    let mut cmaf_len = 0usize;
    let mut buf = [0u8; READ_BUF];

    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        demux.feed(MatroskaInput::Next(&buf[..read]), Timestamp::ZERO)?;
        pump(&mut demux, &mut mux, &mut tracks, &mut log, &mut cmaf_len)?;
    }
    demux.feed(MatroskaInput::Eof, Timestamp::ZERO)?;
    demux.finish()?;
    pump(&mut demux, &mut mux, &mut tracks, &mut log, &mut cmaf_len)?;
    mux.finish()?;
    while let Some(output) = mux.poll() {
        match output {
            MuxOutput::Init(bytes) | MuxOutput::Bytes(bytes) => cmaf_len += bytes.len(),
            MuxOutput::Segment(segment) => {
                log.push(&segment)?;
                cmaf_len += segment.bytes.len();
            }
            _ => {}
        }
    }

    println!("IR: {} track(s)", tracks.len());
    for entry in &tracks {
        println!(
            "  track {:>2}: {:<26} {:>4} samples",
            entry.spec.track_id,
            describe(&entry.spec.config),
            entry.samples,
        );
    }

    println!("--- CMAF/fMP4 ---");
    println!("segment bytes: {cmaf_len} (one init + one media segment)");

    let specs: Vec<TrackSpec> = tracks.iter().map(|t| t.spec.clone()).collect();
    let mpd: String = DashPackager::default().render(&specs, &log)?;
    println!("\n--- DASH MPD ({} bytes) ---", mpd.len());
    println!("{mpd}");

    Ok(())
}

/// Drain the demux's events into the summary and the muxer stage.
fn pump(
    demux: &mut MatroskaDemux,
    mux: &mut CmafMuxStage,
    tracks: &mut Vec<TrackSummary>,
    log: &mut SegmentSummaryLog,
    cmaf_len: &mut usize,
) -> Result<(), Box<dyn std::error::Error>> {
    while let Some(event) = demux.poll() {
        match event {
            DemuxEvent::TrackAdded(spec) => {
                tracks.push(TrackSummary {
                    spec: spec.clone(),
                    samples: 0,
                });
                mux.feed(MuxInput::Track(Box::new(spec)), Timestamp::ZERO)?;
            }
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
                MuxOutput::Init(bytes) | MuxOutput::Bytes(bytes) => *cmaf_len += bytes.len(),
                MuxOutput::Segment(segment) => {
                    log.push(&segment)?;
                    *cmaf_len += segment.bytes.len();
                }
                _ => {}
            }
        }
    }
    Ok(())
}
