//! One streaming MP4 stage per init (issue #4057, streaming-first phase 3):
//! a media segment whose `moof` is damaged costs only itself, and the next
//! segment demuxes under the same init.

use broadcast_hls::MediaSequence;
use hls_runtime::client::{Action, HlsClient, HlsClientConfig, Output, ResourceId};
use test_harness::fmp4_split::{Split, split};
use url::Url;

const PLAYLIST_URL: &str = "http://fixture/index.m3u8";

fn client() -> HlsClient {
    HlsClient::new(HlsClientConfig::new(Url::parse(PLAYLIST_URL).expect("URL")))
}

fn drain(client: &mut HlsClient) -> Vec<Action> {
    std::iter::from_fn(|| client.poll()).collect()
}

fn outputs(client: &mut HlsClient) -> Vec<Output> {
    std::iter::from_fn(|| client.next_output()).collect()
}

fn map_playlist(map_uri: &str, segments: &[&str]) -> String {
    let mut text = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n\
         #EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-MAP:URI=\"{map_uri}\"\n"
    );
    for uri in segments {
        text.push_str(&format!("#EXTINF:2.0,\n{uri}\n"));
    }
    text.push_str("#EXT-X-ENDLIST\n");
    text
}

/// The `moof` of `segment` with its first child's framing destroyed, the way
/// a real damaged fragment arrives.
fn damage_first_moof(segment: &[u8]) -> Vec<u8> {
    let mut damaged = segment.to_vec();
    let moof = damaged
        .windows(4)
        .position(|w| w == b"moof")
        .expect("a moof in the segment");
    damaged[moof + 8..moof + 12].copy_from_slice(&2u32.to_be_bytes());
    damaged
}

#[test]
fn a_damaged_segment_costs_only_itself() {
    let bytes = test_fixtures::read("mp4/cmaf/av-frag.mp4");
    let Split { init, segments } = split(&bytes).expect("split");
    assert!(
        segments.len() >= 2,
        "the fixture must hold at least two fragments: {}",
        segments.len()
    );
    let damaged = damage_first_moof(&segments[0]);
    let good = segments[1].clone();

    let mut client = client();
    let _ = drain(&mut client);
    client
        .on_playlist(map_playlist("init.mp4", &["s0.m4s", "s1.m4s"]).as_bytes())
        .expect("playlist parses");
    let _ = drain(&mut client);
    client
        .on_resource(ResourceId::Init, &init)
        .expect("init accepted");
    let _ = drain(&mut client);
    client
        .on_resource(
            ResourceId::Segment {
                msn: MediaSequence::new(0),
            },
            &damaged,
        )
        .expect("a damaged segment is not a hard error");
    let damaged_outputs = outputs(&mut client);
    assert!(
        damaged_outputs.iter().any(|o| matches!(
            o,
            Output::FragmentDamaged { id, skipped }
                if *skipped == 1 && *id == ResourceId::Segment { msn: MediaSequence::new(0) }
        )),
        "the damage is reported against its own segment: {damaged_outputs:?}"
    );

    client
        .on_resource(
            ResourceId::Segment {
                msn: MediaSequence::new(1),
            },
            &good,
        )
        .expect("the next segment still demuxes");
    let good_outputs = outputs(&mut client);
    assert!(
        !good_outputs
            .iter()
            .any(|o| matches!(o, Output::FragmentDamaged { .. })),
        "the good segment carries no damage: {good_outputs:?}"
    );
    let samples = good_outputs
        .iter()
        .filter_map(|o| match o {
            Output::Samples { samples, .. } => Some(samples.len()),
            _ => None,
        })
        .sum::<usize>();
    assert!(
        samples > 0,
        "the good segment yields samples: {good_outputs:?}"
    );
}
