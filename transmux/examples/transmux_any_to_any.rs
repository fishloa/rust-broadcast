//! `transmux` cross-container walkthrough — one IR, many outputs.
//!
//! The whole point of the hub is that a source container is demuxed *once* into
//! the neutral [`Media`] IR, then that single IR feeds any number of muxer
//! stages and manifest renderers — no re-demux, no re-encoding.
//!
//! This example drives `{WebM} → IR → {DASH MPD + CMAF}`:
//!
//! 1. Read a real VP9 + Opus WebM fixture.
//! 2. [`WebmDemux`] → [`Media`] (via [`Unpackage`](broadcast_common::Unpackage)).
//! 3. Feed that one IR to two packagers:
//!    - [`DashPackager`] → an MPEG-DASH `.mpd` manifest (printed in full), and
//!    - [`CmafMuxStage`](transmux::CmafMuxStage) → CMAF/fMP4 segment bytes (init + media sizes printed).
//!
//! Run it with:
//!
//! ```text
//! cargo run -p transmux --example transmux_any_to_any
//! ```

use std::path::PathBuf;

use broadcast_common::Unpackage;
use transmux::pipeline::CodecConfig;
use transmux::{DashPackager, Media, WebmDemux};

/// Locate a fixture under the workspace `fixtures/` directory (a sibling of the
/// crate's `CARGO_MANIFEST_DIR`). Read at runtime via `std::fs` — never
/// `include_bytes!`.
/// One CMAF init segment followed by one media segment.
/// Mux the IR to one CMAF artifact, returning its bytes and the summary log
/// the DASH manifest renders from (the muxer and the manifest share one pass).
fn cmaf(
    media: &Media,
) -> Result<(Vec<u8>, transmux::segment_log::SegmentSummaryLog), Box<dyn std::error::Error>> {
    let mut stage =
        transmux::CmafMuxStage::new(media.movie_timescale, transmux::FragmentCut::OnFlush, 1);
    let outputs =
        transmux::mux_stage::drive_media(&mut stage, media, broadcast_common::Timestamp::ZERO)?;
    let mut bytes = Vec::new();
    let mut log = transmux::segment_log::SegmentSummaryLog::new();
    for output in outputs {
        match output {
            transmux::mux_stage::MuxOutput::Init(part)
            | transmux::mux_stage::MuxOutput::Bytes(part) => bytes.extend_from_slice(&part),
            transmux::mux_stage::MuxOutput::Segment(segment) => {
                log.push(&segment)?;
                bytes.extend_from_slice(&segment.bytes);
            }
            _ => {}
        }
    }
    Ok((bytes, log))
}

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = fixture_path("mkv/vp9-opus.webm");
    let webm = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) => {
            // A fixture-less checkout still builds and runs cleanly (exit 0).
            println!(
                "fixture {} unavailable ({err}); nothing to do.",
                path.display()
            );
            return Ok(());
        }
    };
    println!("read {} bytes from {}\n", webm.len(), path.display());

    // 1 + 2. Unpackage the WebM into the neutral IR (demuxed exactly once).
    let media: Media = WebmDemux::new().unpackage(&webm[..])?;
    println!("demuxed IR: {} track(s)", media.tracks.len());
    for track in &media.tracks {
        println!(
            "  track {:>2}: {:<26} {:>4} samples",
            track.track_id(),
            describe(track.config()),
            track.samples.len(),
        );
    }

    // 3a. The SAME IR → CMAF/fMP4 segment bytes plus the segment summary log.
    let (cmaf, log) = cmaf(&media)?;
    println!("--- CMAF/fMP4 ---");
    println!(
        "segment bytes: {} (one init + one media segment)",
        cmaf.len()
    );

    // 3b. The SAME IR → an MPEG-DASH manifest, rendered from the summaries the
    //     muxer just emitted — never from the samples.
    let specs: Vec<transmux::TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let mpd: String = DashPackager::default().render(&specs, &log)?;
    println!("\n--- DASH MPD ({} bytes) ---", mpd.len());
    println!("{mpd}");

    Ok(())
}
