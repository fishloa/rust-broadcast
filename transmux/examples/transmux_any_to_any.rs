//! `transmux` cross-container walkthrough — one IR, many outputs.
//!
//! The whole point of the hub is that a source container is demuxed *once* into
//! the neutral [`Media`] IR, then that single IR feeds any number of
//! [`Package`](broadcast_common::Package) outputs — no re-demux, no
//! re-encoding.
//!
//! This example drives `{WebM} → IR → {DASH MPD + CMAF}`:
//!
//! 1. Read a real VP9 + Opus WebM fixture.
//! 2. [`WebmDemux`] → [`Media`] (via [`Unpackage`](broadcast_common::Unpackage)).
//! 3. Feed that one IR to two packagers:
//!    - [`DashPackager`] → an MPEG-DASH `.mpd` manifest (printed in full), and
//!    - [`CmafMux`] → CMAF/fMP4 segment bytes (init + media sizes printed).
//!
//! Run it with:
//!
//! ```text
//! cargo run -p transmux --example transmux_any_to_any
//! ```

use std::path::PathBuf;

use broadcast_common::{Package, Unpackage};
use transmux::pipeline::CodecConfig;
use transmux::{DashPackager, Media, WebmDemux};

/// Locate a fixture under the workspace `fixtures/` directory (a sibling of the
/// crate's `CARGO_MANIFEST_DIR`). Read at runtime via `std::fs` — never
/// `include_bytes!`.
fn fixture_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join(rel)
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
    let path = fixture_path("webm/vp9_opus.webm");
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

    // 3a. The SAME IR → an MPEG-DASH manifest.
    let mpd: String = DashPackager::default().package(&media)?;
    println!("\n--- DASH MPD ({} bytes) ---", mpd.len());
    println!("{mpd}");

    // 3b. The SAME IR → CMAF/fMP4 segment bytes.
    let cmaf: Vec<u8> = transmux::CmafMux::default().package(&media)?;
    println!("--- CMAF/fMP4 ---");
    println!(
        "segment bytes: {} (one init + one media segment)",
        cmaf.len()
    );

    Ok(())
}
