//! Build a live/DVR client Manifest from the typed model — including the
//! `IsLive`/`DVRWindowLength`/`LookAheadFragmentCount` attributes and a
//! `Protection`/`ProtectionHeader` — and render it.
//!
//! Run with `cargo run -p broadcast-smooth --example build_manifest`.

use broadcast_smooth::{C, SmoothManifest, StreamIndex, StreamType};

/// One 2 s fragment at the 10 MHz manifest time scale.
const LIVE_FRAGMENT_TICKS: u64 = 2 * 10_000_000;
/// A 120 s DVR window.
const DVR_WINDOW_TICKS: u64 = 120 * 10_000_000;
/// Fragments the server announces ahead of the live edge.
const LOOK_AHEAD_FRAGMENTS: u32 = 2;

fn main() {
    let path = test_fixtures::path("smooth/av/Manifest");
    let vetted = SmoothManifest::parse(&std::fs::read_to_string(path).expect("read the fixture"))
        .expect("valid client manifest");
    let video = vetted.streams[0].qualities[0].clone();
    let protection_header: String = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "AAABAwBXAFcAVwAB//8A4wAA".to_string());

    let manifest = SmoothManifest::new(vec![
        StreamIndex::new(
            StreamType::Video,
            "QualityLevels({bitrate})/Fragments(video={start time})",
            vec![video],
        )
        .with_subtype("")
        .with_counts(6, 1)
        .with_chunks(
            (0..6)
                .map(|i| {
                    if i == 0 {
                        C::new(LIVE_FRAGMENT_TICKS).at(0)
                    } else {
                        C::new(LIVE_FRAGMENT_TICKS)
                    }
                })
                .collect(),
        ),
    ])
    .with_duration(6 * LIVE_FRAGMENT_TICKS)
    .live(DVR_WINDOW_TICKS, LOOK_AHEAD_FRAGMENTS)
    .with_protection(protection_header);

    let xml = manifest.to_xml();
    print!("{xml}");
    assert_eq!(
        SmoothManifest::parse(&xml).expect("re-parse"),
        manifest,
        "the model round-trips through the serializer"
    );
}
