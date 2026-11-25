//! No per-segment init + segment concatenation in the HLS client's demux path
//! (issue #4057, streaming-first phase 3 slice 2).
//!
//! The client used to build one `init.len() + segment.len()` buffer for every
//! fetched Part or Segment and hand it to a whole-input demuxer. It now feeds
//! one streaming stage a bounded piece at a time, so no single allocation in
//! the demux call is anywhere near the size of the body. The allocation
//! counter is armed only around that call, with the caller's own body and
//! init outside the measured window.

use broadcast_hls::MediaSequence;
use hls_runtime::client::{Action, HlsClient, HlsClientConfig, Output, ResourceId};
use test_alloc::ProcessCounting;
use test_harness::big_media::{BigFmp4Options, write_big_fmp4};
use test_harness::fmp4_split::{Split, split};
use url::Url;

#[global_allocator]
static COUNTER: ProcessCounting = ProcessCounting::new();

/// Largest single allocation the demux call may make, as a fraction of the
/// body: a per-segment concatenation is the whole body.
const MAX_ALLOCATION_DIVISOR: usize = 4;
/// Bytes in each fragment's one sample.
const SAMPLE_BYTES: u32 = 4 * 1024;
/// Fragments in the generated presentation (one sample each).
const FRAGMENTS: u32 = 1024;

const PLAYLIST_URL: &str = "http://fixture/index.m3u8";

fn temp_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("hls-runtime-alloc-{}.mp4", std::process::id()))
}

fn generated() -> (Vec<u8>, Vec<u8>) {
    let path = temp_path();
    write_big_fmp4(
        &path,
        &BigFmp4Options {
            target_bytes: 0,
            fragment_count: FRAGMENTS,
            sample_bytes: SAMPLE_BYTES,
            with_mfra: false,
            protect: false,
        },
    );
    let bytes = std::fs::read(&path).expect("read the generated fMP4");
    let _ = std::fs::remove_file(&path);
    let Split { init, segments } = split(&bytes).expect("split the generated fMP4");
    let mut segment = Vec::new();
    for fragment in &segments {
        segment.extend_from_slice(fragment);
    }
    (init, segment)
}

fn drain(client: &mut HlsClient) -> Vec<Action> {
    std::iter::from_fn(|| client.poll()).collect()
}

fn samples_of(client: &mut HlsClient) -> usize {
    let mut samples = 0usize;
    while let Some(output) = client.next_output() {
        if let Output::Samples { samples: batch, .. } = output {
            samples = samples.saturating_add(batch.len());
        }
    }
    samples
}

#[test]
fn demuxing_a_segment_allocates_nothing_near_its_length() {
    let (init, segment) = generated();
    let combined = init.len().saturating_add(segment.len());
    let playlist = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n\
                    #EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-MAP:URI=\"init.mp4\"\n\
                    #EXTINF:2.0,\nseg0.m4s\n#EXT-X-ENDLIST\n";
    let mut client = HlsClient::new(HlsClientConfig::new(Url::parse(PLAYLIST_URL).expect("URL")));
    COUNTER.set_armed(false);
    let _ = drain(&mut client);
    client
        .on_playlist(playlist.as_bytes())
        .expect("playlist parses");
    let _ = drain(&mut client);
    client
        .on_resource(ResourceId::Init, &init)
        .expect("init accepted");
    let _ = drain(&mut client);
    COUNTER.reset_largest();
    COUNTER.set_armed(true);
    let fed = client.on_resource(
        ResourceId::Segment {
            msn: MediaSequence::new(0),
        },
        &segment,
    );
    COUNTER.set_armed(false);
    fed.expect("the segment demuxes");
    let largest = COUNTER.largest();
    let samples = samples_of(&mut client);

    assert_eq!(
        samples,
        usize::try_from(FRAGMENTS).expect("fragment count"),
        "every fragment's one sample was demuxed"
    );
    assert!(
        largest < combined / MAX_ALLOCATION_DIVISOR,
        "one allocation of {largest} bytes for a {combined}-byte init+segment"
    );
}
