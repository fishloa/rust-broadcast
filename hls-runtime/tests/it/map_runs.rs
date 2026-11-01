//! RFC 8216 §4.4.4.5: an `EXT-X-MAP` applies to every Media Segment after it
//! until the next map, so each segment is demuxed with the init of its own
//! map-run, however the fetches interleave. Round-6 audit F3.

#[path = "../../../broadcast-client/tests/support/fmp4_split.rs"]
#[allow(dead_code)]
mod fmp4_split;

use std::collections::BTreeMap;

use broadcast_common::Unpackage;
use fmp4_split::{Split, split};
use hls_runtime::client::{Action, HlsClient, Output, ResourceId};
use transmux::sample_aes::aes128_encrypt_segment;

const URL: &str = "http://o/v/index.m3u8";
const H264_AAC: &str = "transmux/h264_aac_frag.mp4";
const HEVC: &str = "transmux/hevc_frag.mp4";
const KEY: [u8; 16] = [0x5a; 16];
const IV: [u8; 16] = [0x11; 16];

struct Source {
    bytes: Vec<u8>,
    parts: Split,
}

fn source(name: &str) -> Source {
    let bytes = test_fixtures::read(name);
    let parts = split(&bytes).unwrap();
    Source { bytes, parts }
}

fn counts(bytes: &[u8]) -> BTreeMap<u32, usize> {
    let media = transmux::Fmp4Demux::new().unpackage(bytes).unwrap();
    media
        .tracks
        .iter()
        .map(|t| (t.spec.track_id.value(), t.samples.len()))
        .collect()
}

fn two_run_playlist(first: &Source, second: &Source, key_line: &str) -> String {
    let mut p = String::from(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n#EXT-X-PLAYLIST-TYPE:VOD\n\
         #EXT-X-MAP:URI=\"i1.mp4\"\n",
    );
    for i in 0..first.parts.segments.len() {
        p.push_str(&format!("#EXTINF:1.0,\na{i}.m4s\n"));
    }
    p.push_str("#EXT-X-DISCONTINUITY\n");
    p.push_str(key_line);
    p.push_str("#EXT-X-MAP:URI=\"i2.mp4\"\n");
    for i in 0..second.parts.segments.len() {
        p.push_str(&format!("#EXTINF:1.0,\nb{i}.m4s\n"));
    }
    p.push_str("#EXT-X-ENDLIST\n");
    p
}

fn drain(client: &mut HlsClient) -> Vec<Action> {
    std::iter::from_fn(|| client.poll()).collect()
}

fn fetches(actions: &[Action]) -> Vec<(ResourceId, String)> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::FetchResource { id, url, .. } => Some((*id, url.clone())),
            _ => None,
        })
        .collect()
}

fn init_fetches(actions: &[Action]) -> Vec<String> {
    fetches(actions)
        .into_iter()
        .filter(|(id, _)| *id == ResourceId::Init)
        .map(|(_, url)| url)
        .collect()
}

#[derive(Default)]
struct Seen {
    announced: Vec<Vec<u8>>,
    per_init: Vec<BTreeMap<u32, usize>>,
}

impl Seen {
    fn absorb(&mut self, outputs: Vec<Output>) {
        for o in outputs {
            match o {
                Output::Init(bytes) => {
                    self.announced.push(bytes);
                    self.per_init.push(BTreeMap::new());
                }
                Output::Samples { track_id, samples } => {
                    let current = self.per_init.last_mut().expect("Init precedes Samples");
                    *current.entry(track_id.value()).or_default() += samples.len();
                }
                _ => {}
            }
        }
    }
}

fn collect_outputs(client: &mut HlsClient, seen: &mut Seen) {
    seen.absorb(std::iter::from_fn(|| client.next_output()).collect());
}

#[test]
fn segments_delivered_out_of_order_still_use_their_own_init() {
    let first = source(H264_AAC);
    let second = source(HEVC);
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    client
        .on_playlist(two_run_playlist(&first, &second, "").as_bytes())
        .unwrap();
    let actions = drain(&mut client);
    assert_eq!(
        init_fetches(&actions),
        vec!["http://o/v/i1.mp4".to_string()],
        "inits are fetched one at a time, in playlist order"
    );
    let mut seen = Seen::default();

    for i in 0..second.parts.segments.len() {
        client
            .on_resource(
                ResourceId::Segment {
                    msn: (first.parts.segments.len() + i) as u64,
                },
                &second.parts.segments[i],
            )
            .unwrap();
    }
    client
        .on_resource(ResourceId::Init, &first.parts.init)
        .unwrap();
    let after_first = drain(&mut client);
    assert_eq!(
        init_fetches(&after_first),
        vec!["http://o/v/i2.mp4".to_string()],
        "the next init is fetched once the previous one arrived"
    );
    for (i, seg) in first.parts.segments.iter().enumerate() {
        client
            .on_resource(ResourceId::Segment { msn: i as u64 }, seg)
            .unwrap();
    }
    client
        .on_resource(ResourceId::Init, &second.parts.init)
        .unwrap();
    collect_outputs(&mut client, &mut seen);

    assert_eq!(seen.announced.len(), 2, "one Init per map-run");
    let by_init: BTreeMap<&[u8], &BTreeMap<u32, usize>> = seen
        .announced
        .iter()
        .map(Vec::as_slice)
        .zip(seen.per_init.iter())
        .collect();
    assert_eq!(by_init[first.parts.init.as_slice()], &counts(&first.bytes));
    assert_eq!(
        by_init[second.parts.init.as_slice()],
        &counts(&second.bytes)
    );
}

#[test]
fn each_init_is_fetched_once_across_reloads() {
    let first = source(H264_AAC);
    let second = source(HEVC);
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    let text = two_run_playlist(&first, &second, "").replace("#EXT-X-ENDLIST\n", "");
    client.on_playlist(text.as_bytes()).unwrap();
    let mut urls = init_fetches(&drain(&mut client));
    client
        .on_resource(ResourceId::Init, &first.parts.init)
        .unwrap();
    client.on_playlist(text.as_bytes()).unwrap();
    urls.extend(init_fetches(&drain(&mut client)));
    client
        .on_resource(ResourceId::Init, &second.parts.init)
        .unwrap();
    client.on_playlist(text.as_bytes()).unwrap();
    urls.extend(init_fetches(&drain(&mut client)));
    assert_eq!(
        urls,
        vec![
            "http://o/v/i1.mp4".to_string(),
            "http://o/v/i2.mp4".to_string()
        ]
    );
}

#[test]
fn a_failed_init_fetch_is_asked_for_again_without_losing_the_queue() {
    let first = source(H264_AAC);
    let second = source(HEVC);
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    client
        .on_playlist(two_run_playlist(&first, &second, "").as_bytes())
        .unwrap();
    assert_eq!(init_fetches(&drain(&mut client)).len(), 1);
    client.on_error(Some(ResourceId::Init));
    let queued = init_fetches(&drain(&mut client));
    assert_eq!(
        queued,
        vec!["http://o/v/i2.mp4".to_string()],
        "the next wanted init proceeds while the failed one waits"
    );
    client
        .on_resource(ResourceId::Init, &second.parts.init)
        .unwrap();
    assert!(client.retry_init());
    assert_eq!(
        init_fetches(&drain(&mut client)),
        vec!["http://o/v/i1.mp4".to_string()]
    );
    assert!(
        !client.retry_init(),
        "nothing left to retry while it is in flight"
    );
}

#[test]
fn a_map_run_with_its_own_key_decrypts_its_own_init() {
    let first = source(H264_AAC);
    let second = source(HEVC);
    let key_line = format!(
        "#EXT-X-KEY:METHOD=AES-128,URI=\"k.bin\",IV=0x{}\n",
        IV.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    client
        .on_playlist(two_run_playlist(&first, &second, &key_line).as_bytes())
        .unwrap();
    let key_id = drain(&mut client)
        .into_iter()
        .find_map(|a| match a {
            Action::FetchKey { id, .. } => Some(id),
            _ => None,
        })
        .expect("the second run's key is requested");
    client
        .on_resource(ResourceId::Init, &first.parts.init)
        .unwrap();
    drain(&mut client);
    let cipher = aes128_encrypt_segment(&KEY, &IV, &second.parts.init);
    client.on_resource(ResourceId::Init, &cipher).unwrap();
    client.on_key(key_id, &KEY).unwrap();
    for (i, seg) in first.parts.segments.iter().enumerate() {
        client
            .on_resource(ResourceId::Segment { msn: i as u64 }, seg)
            .unwrap();
    }
    for (i, seg) in second.parts.segments.iter().enumerate() {
        let msn = (first.parts.segments.len() + i) as u64;
        let cipher = aes128_encrypt_segment(&KEY, &IV, seg);
        client
            .on_resource(ResourceId::Segment { msn }, &cipher)
            .unwrap();
    }
    let mut seen = Seen::default();
    collect_outputs(&mut client, &mut seen);
    assert_eq!(
        seen.announced,
        vec![first.parts.init.clone(), second.parts.init.clone()]
    );
    assert_eq!(
        seen.per_init,
        vec![counts(&first.bytes), counts(&second.bytes)]
    );
}
