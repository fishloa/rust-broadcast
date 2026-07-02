//! `transmux` quickstart — the any-to-any container hub in one flow.
//!
//! transmux is a *samples-in* container multiplexer built around two
//! `broadcast_common` traits:
//!
//! - [`Unpackage`](broadcast_common::Unpackage) — a demuxer parses a source
//!   container into the neutral [`Media`] intermediate representation (IR): a
//!   set of elementary [`Track`]s, each an ordered run of coded access units
//!   (`Sample`s) plus its codec configuration.
//! - [`Package`](broadcast_common::Package) — a packager renders that same IR
//!   back into an output container. Coded samples pass through opaque; transmux
//!   never encodes or decodes media.
//!
//! This example drives the `{TS} → IR → {CMAF}` path:
//!
//! 1. Read a real H.264 + AAC MPEG-2 Transport Stream fixture.
//! 2. [`TsDemux`] → [`Media`]: print each track's codec, geometry, and count.
//! 3. Build a CMAF initialization segment + a media segment from the IR and
//!    print their sizes.
//!
//! Run it with:
//!
//! ```text
//! cargo run -p transmux --example transmux_hub
//! ```

use std::path::PathBuf;

use broadcast_common::Unpackage;
use transmux::pipeline::{build_init_segment, build_media_segment, CodecConfig, FragmentTrackData};
use transmux::{Media, Track, TrackSpec, TsDemux};

/// Locate a fixture under the workspace `fixtures/` directory (a sibling of the
/// crate's `CARGO_MANIFEST_DIR`). Fixtures are read at runtime via `std::fs`
/// (never `include_bytes!`, which would bloat the published crate).
fn fixture_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join(rel)
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

/// Print the codec / geometry / sample-count summary for every track in the IR.
fn print_track_summary(media: &Media) {
    println!(
        "IR: {} track(s), movie timescale {} ticks/s",
        media.tracks.len(),
        media.movie_timescale
    );
    for track in &media.tracks {
        println!(
            "  track {:>2}: {:<28} {:>5} samples @ {} Hz",
            track.track_id(),
            describe(track.config()),
            track.samples.len(),
            track.timescale(),
        );
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = fixture_path("ts/h264_aac.ts");
    let ts = match std::fs::read(&path) {
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
    println!("read {} bytes from {}\n", ts.len(), path.display());

    // 1. Unpackage: MPEG-2 TS → neutral Media IR.
    let media: Media = TsDemux::new().unpackage(&ts[..])?;
    print_track_summary(&media);

    // 2. Package: Media IR → CMAF. `CmafMux::package` concatenates an init
    //    segment and a media segment; here we build them separately (the same
    //    public pipeline the muxer uses) so we can report each size.
    let specs: Vec<TrackSpec> = media
        .tracks
        .iter()
        .map(|t: &Track| t.spec.clone())
        .collect();
    let init = build_init_segment(&specs, media.movie_timescale)?;

    let fragments: Vec<FragmentTrackData<'_>> = media
        .tracks
        .iter()
        .map(|t| FragmentTrackData {
            track_id: t.spec.track_id,
            base_media_decode_time: 0,
            samples: &t.samples,
        })
        .collect();
    let media_seg = build_media_segment(1, &fragments)?;

    println!();
    println!("CMAF init segment : {:>7} bytes", init.len());
    println!("CMAF media segment: {:>7} bytes", media_seg.len());
    println!(
        "CMAF total        : {:>7} bytes",
        init.len() + media_seg.len()
    );

    Ok(())
}
