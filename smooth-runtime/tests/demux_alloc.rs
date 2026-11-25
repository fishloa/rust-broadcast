//! A fragment is demuxed on the stream's own stage, never concatenated with
//! the synthetic init segment.
//!
//! Eight large fragments play back to back under one synthetic init. While the
//! last of them is demuxed, no single allocation may reach `init + fragment`:
//! the buffer the old per-fragment concatenation built. The stage keeps its
//! buffer from fragment to fragment, so the only allocation that size is the
//! sample cut out of the fragment.
//!
//! One `#[test]` in this binary: [`ProcessCounting`] is process-global, so a
//! sibling test would pollute the measurement.

use broadcast_common::stage::Timestamp;
use broadcast_smooth::SmoothManifest;
use smooth_runtime::{Action, SmoothClient, SmoothClientConfig};
use transmux::pipeline::build_init_segment;
use transmux::smooth_fragment::fragment_track_id;
use transmux::{Sample, SmoothPackager, Track, track_spec_from_quality_level};
use url::Url;

#[global_allocator]
static ALLOC: test_alloc::ProcessCounting = test_alloc::ProcessCounting::new();

/// Chunks the generated manifest carries. Every one is served the same large
/// fragment, so the stage is fed eight fragments under one init.
const CHUNKS: usize = 8;
/// The large fragment's sample payload, just under a mebibyte.
const PAYLOAD: usize = (1 << 20) - 4096;
/// The decode time the generated fragment is placed at, in the track's
/// 10 MHz timescale.
const CHUNK_START: u64 = 10_000_000;
/// One chunk's duration, in the same timescale.
const CHUNK_DURATION: u32 = 1_000_000;

const MANIFEST: &str = "Manifest";
const FIRST_VIDEO: &str = "QualityLevels(64528)/Fragments(video=0)";
const ORIGIN: &str = "http://origin.test/";
/// Test-clock steps, enough to play every chunk of a static manifest.
const STEP_LIMIT: usize = 500;

/// One length-prefixed NAL of `PAYLOAD` bytes, so the sample is canonical
/// four-byte-length AVC and the demuxer keeps it opaque.
fn sample_data() -> Vec<u8> {
    let mut data = Vec::with_capacity(PAYLOAD + 4);
    data.extend_from_slice(
        &u32::try_from(PAYLOAD)
            .expect("payload fits a NAL length")
            .to_be_bytes(),
    );
    data.resize(PAYLOAD + 4, 0x5a);
    data
}

/// The fixture manifest with its video chunk list replaced by `CHUNKS`
/// chunks of one stream, and its one video `QualityLevel` kept as-is.
fn manifest_text() -> String {
    let fixture = String::from_utf8(test_fixtures::read(&format!("smooth/av/{MANIFEST}")))
        .expect("the manifest is utf8");
    let quality = fixture
        .lines()
        .find(|line| line.starts_with("<QualityLevel Index=\"0\" Bitrate=\"64528\""))
        .expect("the fixture manifest describes a video quality level");
    let chunks: String = (0..CHUNKS)
        .map(|index| format!("<c n=\"{index}\" d=\"{CHUNK_DURATION}\" />\n"))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <SmoothStreamingMedia MajorVersion=\"2\" MinorVersion=\"0\" Duration=\"{duration}\">\n\
         <StreamIndex Type=\"video\" QualityLevels=\"1\" Chunks=\"{CHUNKS}\" \
         Url=\"QualityLevels({{bitrate}})/Fragments(video={{start time}})\">\n\
         {quality}\n{chunks}</StreamIndex>\n</SmoothStreamingMedia>\n",
        duration = u64::from(CHUNK_DURATION) * CHUNKS as u64,
    )
}

/// The manifest's first video quality as a synthetic init segment, and one
/// fragment carrying a single large sample of that track.
fn large_fragment(manifest: &SmoothManifest) -> (Vec<u8>, Vec<u8>) {
    let index = manifest.streams[0].clone();
    let quality = index.qualities[0].clone();
    let fixture = test_fixtures::read(&format!("smooth/av/{FIRST_VIDEO}"));
    let local = fragment_track_id(&fixture).expect("the fixture fragment names a track");
    let mut spec =
        track_spec_from_quality_level(local, manifest.timescale, index.stream_type, &quality)
            .expect("the quality level describes a track");
    spec.track_id = local;
    let init = build_init_segment(std::slice::from_ref(&spec), transmux::VIDEO_CLOCK_TIMESCALE)
        .expect("the synthetic init segment builds");
    let start = i64::try_from(CHUNK_START).expect("chunk start fits i64");
    let sample = Sample::timed(
        sample_data(),
        start,
        start,
        core::num::NonZeroU32::new(CHUNK_DURATION),
    );
    let fragment = SmoothPackager {
        target_duration: std::time::Duration::from_secs(1),
    }
    .package_track_fragment(&Track::new(spec, vec![sample]), 1, CHUNK_START)
    .expect("the large fragment packages");
    (init, fragment.data)
}

#[test]
fn a_fragment_is_never_concatenated_with_the_init_segment() {
    let text = manifest_text();
    let manifest = SmoothManifest::parse(&text).expect("the generated manifest parses");
    let (init, large) = large_fragment(&manifest);
    let mut client = SmoothClient::new(
        SmoothClientConfig::default()
            .with_manifest_url(Url::parse(&format!("{ORIGIN}{MANIFEST}")).expect("url")),
    );
    let mut now = 0u64;
    let mut served = 0usize;
    let mut largest = 0usize;
    for _ in 0..STEP_LIMIT {
        while let Some(action) = client.poll(Timestamp::from_micros(now)) {
            match action {
                Action::FetchManifest { id, .. } => {
                    client.on_manifest(Timestamp::from_micros(now), id, text.as_bytes());
                }
                Action::FetchFragment { id, .. } => {
                    served += 1;
                    ALLOC.reset_largest();
                    client.on_fragment(Timestamp::from_micros(now), id, &large);
                    largest = ALLOC.largest();
                }
                _ => {}
            }
        }
        while client.next_output().is_some() {}
        match client.next_deadline() {
            Some(t) if t > now => now = t,
            Some(_) => {}
            None => break,
        }
    }
    assert_eq!(served, CHUNKS, "every chunk reached the stage");
    assert!(
        largest < init.len() + large.len(),
        "one allocation of {largest} bytes spans the init and the last fragment ({} + {})",
        init.len(),
        large.len()
    );
}
