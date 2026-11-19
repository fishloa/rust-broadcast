//! Audit 22 findings 58 and 59 (issues #3026, #3027): the playback client's
//! part-gap and key-cache decisions, its init-slot identity across reloads,
//! its abandoned-key bookkeeping, damaged-fragment reporting and the debug /
//! label surface.
//!
//! Spec: RFC 8216bis §4.4.4.7 (`EXT-X-GAP`), §4.4.4.9 (`EXT-X-PART`),
//! §4.3.2.4/§5.2 (`EXT-X-KEY`), §4.4.4.5 (`EXT-X-MAP`).

use broadcast_hls::MediaSequence;
use hls_runtime::client::{
    Action, Error, HlsClient, HlsClientConfig, KeyId, Output, PlaylistRequest, ResourceId,
};
use url::Url;

const URL: &str = "http://example.com/p.m3u8";
const KEY: [u8; 16] = [0x5a; 16];

fn client() -> HlsClient {
    HlsClient::new(HlsClientConfig::new(Url::parse(URL).expect("URL")))
}

fn drain(client: &mut HlsClient) -> Vec<Action> {
    std::iter::from_fn(|| client.poll()).collect()
}

fn fetch_urls(actions: &[Action]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::FetchResource { url, .. } => Some(url.as_str().to_string()),
            _ => None,
        })
        .collect()
}

fn key_fetches(actions: &[Action]) -> Vec<(KeyId, String)> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::FetchKey { id, url } => Some((*id, url.as_str().to_string())),
            _ => None,
        })
        .collect()
}

fn outputs(client: &mut HlsClient) -> Vec<Output> {
    std::iter::from_fn(|| client.next_output()).collect()
}

#[test]
fn a_gap_part_is_never_requested_in_a_closed_or_open_segment() {
    let text = "#EXTM3U\n#EXT-X-VERSION:9\n#EXT-X-TARGETDURATION:4\n\
                #EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PART-INF:PART-TARGET=0.5\n\
                #EXT-X-PART:DURATION=0.5,URI=\"c0.0.ts\"\n\
                #EXT-X-PART:DURATION=0.5,URI=\"c0.1.ts\",GAP=YES\n\
                #EXTINF:4,\nc0.ts\n\
                #EXT-X-PART:DURATION=0.5,URI=\"o1.0.ts\"\n\
                #EXT-X-PART:DURATION=0.5,URI=\"o1.1.ts\",GAP=YES\n";
    let mut client = client();
    let _ = drain(&mut client);
    client
        .on_playlist(text.as_bytes())
        .expect("playlist parses");
    let urls = fetch_urls(&drain(&mut client));
    assert!(
        urls.iter().any(|u| u.ends_with("c0.0.ts")),
        "a non-gap closed part is fetched: {urls:?}"
    );
    assert!(
        urls.iter().any(|u| u.ends_with("o1.0.ts")),
        "a non-gap open part is fetched: {urls:?}"
    );
    assert!(
        !urls.iter().any(|u| u.ends_with(".1.ts")),
        "a GAP part is never fetched: {urls:?}"
    );
}

fn keyed_playlist(first_msn: u64, key_uri: &str, segments: &[&str]) -> String {
    let mut text = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n\
         #EXT-X-MEDIA-SEQUENCE:{first_msn}\n#EXT-X-KEY:METHOD=AES-128,URI=\"{key_uri}\"\n"
    );
    for uri in segments {
        text.push_str(&format!("#EXTINF:2.0,\n{uri}\n"));
    }
    text.push_str("#EXT-X-ENDLIST\n");
    text
}

#[test]
fn a_second_key_delivery_for_the_same_id_is_refused() {
    let mut client = client();
    let _ = drain(&mut client);
    client
        .on_playlist(keyed_playlist(0, "k.bin", &["s0.ts"]).as_bytes())
        .expect("playlist parses");
    let (id, _url) = key_fetches(&drain(&mut client))
        .into_iter()
        .next()
        .expect("a key fetch is issued");
    client.on_key(id, &KEY).expect("first delivery");
    assert!(matches!(
        client.on_key(id, &KEY),
        Err(Error::UnrequestedKey { .. })
    ));
}

#[test]
fn abandoning_a_key_keeps_the_other_pending_actions() {
    let mut client = client();
    let _ = drain(&mut client);
    client
        .on_playlist(keyed_playlist(0, "k1.bin", &["s0.ts"]).as_bytes())
        .expect("first playlist parses");
    // Take only the key fetch, then queue a reload; the reload request is a
    // pending action like the key fetch.
    let first = client.poll().expect("an action");
    assert!(matches!(first, Action::FetchKey { .. }), "{first:?}");
    assert!(client.ensure_playlist_reload(), "queues one reload");

    // A reload jumps the window past k1's segment; k1 is abandoned.
    client
        .on_playlist(keyed_playlist(5, "k2.bin", &["s5.ts", "s6.ts"]).as_bytes())
        .expect("second playlist parses");
    let actions = drain(&mut client);
    let urls = fetch_urls(&actions);
    assert!(
        urls.iter().any(|u| u.ends_with("s6.ts")),
        "the new segment is fetched: {urls:?}"
    );
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, Action::FetchPlaylist(_))),
        "the queued reload survives the key pruning: {actions:?}"
    );
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

#[test]
fn a_reload_updates_a_failed_inits_re_signed_url() {
    let mut client = client();
    let _ = drain(&mut client);
    client
        .on_playlist(map_playlist("i.mp4?t=1", &["s0.ts"]).as_bytes())
        .expect("first playlist");
    let actions = drain(&mut client);
    assert!(
        fetch_urls(&actions).iter().any(|u| u.contains("i.mp4?t=1")),
        "the first init URL is fetched: {:?}",
        fetch_urls(&actions)
    );
    client.on_error(Some(ResourceId::Init));

    client
        .on_playlist(map_playlist("i.mp4?t=2", &["s0.ts", "s1.ts"]).as_bytes())
        .expect("second playlist");
    let urls = fetch_urls(&drain(&mut client));
    assert!(
        urls.iter().any(|u| u.contains("i.mp4?t=2")),
        "the re-signed map URL is used: {urls:?}"
    );
    assert!(
        !urls.iter().any(|u| u.contains("i.mp4?t=1")),
        "the stale map URL is not fetched again: {urls:?}"
    );
}

#[test]
fn a_new_segment_on_the_same_map_does_not_refetch_the_ready_init() {
    let mut client = client();
    let _ = drain(&mut client);
    client
        .on_playlist(map_playlist("i.mp4", &["s0.ts"]).as_bytes())
        .expect("first playlist");
    let _ = drain(&mut client);
    let init = test_fixtures::read("hls/cmaf-fmp4/init.mp4");
    client
        .on_resource(ResourceId::Init, &init)
        .expect("init accepted");
    let _ = drain(&mut client);

    client
        .on_playlist(map_playlist("i.mp4", &["s0.ts", "s1.ts"]).as_bytes())
        .expect("second playlist");
    let actions = drain(&mut client);
    let inits = actions.iter().filter(|a| {
        matches!(
            a,
            Action::FetchResource {
                id: ResourceId::Init,
                ..
            }
        )
    });
    assert_eq!(
        inits.count(),
        0,
        "a ready init is not fetched again for a new segment: {:?}",
        fetch_urls(&actions)
    );
}

#[test]
fn a_damaged_fragment_is_reported_not_silently_dropped() {
    use test_harness::fmp4_split::{Split, split};

    let bytes = test_fixtures::read("mp4/cmaf/av-frag.mp4");
    let Split { init, segments } = split(&bytes).expect("split");
    let mut damaged = segments[0].clone();
    let moof = damaged
        .windows(4)
        .position(|w| w == b"moof")
        .expect("a moof in the first segment");
    // Destroy the moof's first child framing (mfhd) the way the transmux
    // tests do: a 2-byte size cannot hold a header.
    damaged[moof + 8..moof + 12].copy_from_slice(&2u32.to_be_bytes());

    let mut client = client();
    let _ = drain(&mut client);
    client
        .on_playlist(map_playlist("init.mp4", &["s0.m4s"]).as_bytes())
        .expect("playlist");
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
        .expect("a damaged fragment is not a hard error");
    let outs = outputs(&mut client);
    assert!(
        outs.iter().any(|o| matches!(
            o,
            Output::FragmentDamaged { skipped, .. } if *skipped == 1
        )),
        "{outs:?}"
    );
}

#[test]
fn a_playlist_request_debug_names_the_type_and_url() {
    let request = PlaylistRequest {
        url: Url::parse("http://example.com/live.m3u8").expect("URL"),
        blocking: None,
        skip: true,
    };
    let text = format!("{request:?}");
    assert!(text.contains("PlaylistRequest"), "{text}");
    assert!(text.contains("http://example.com/live.m3u8"), "{text}");
}

#[test]
fn the_client_debug_names_its_ts_demux_state() {
    let mut client = client();
    let _ = drain(&mut client);
    client
        .on_playlist(
            b"#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n\
              #EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\nindex0.ts\n#EXT-X-ENDLIST\n",
        )
        .expect("TS playlist");
    let _ = drain(&mut client);
    let segment = test_fixtures::read("hls/ts-hls/index0.ts");
    client
        .on_resource(
            ResourceId::Segment {
                msn: MediaSequence::new(0),
            },
            &segment,
        )
        .expect("TS segment");
    let text = format!("{client:?}");
    assert!(text.contains("TsDemuxState(StreamingTsDemux)"), "{text}");
}
