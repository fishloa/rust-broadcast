//! Pre-release audit #1485, `HlsClient` findings 1, 2, 4, 5, 12, 14, 15, 16,
//! driven purely through the public API.

use hls_runtime::client::{Action, Error, HlsClient, KeyId, Output, ResourceId};
use transmux::sample_aes::aes128_encrypt_segment;

const URL: &str = "http://origin/live/media.m3u8";
const TS_PACKET_LEN: usize = 188;
const LEADING_PACKETS_DROPPED: usize = 3;
const FIXTURE0: &[u8] = include_bytes!("fixtures/ts-hls/index0.ts");
const KEY: [u8; 16] = [0x42; 16];
const MAX_WAIT_MS: u64 = 60_000;

fn drain(client: &mut HlsClient) -> Vec<Action> {
    std::iter::from_fn(|| client.poll()).collect()
}

fn drain_outputs(client: &mut HlsClient) -> Vec<Output> {
    std::iter::from_fn(|| client.next_output()).collect()
}

fn resource_ids(actions: &[Action]) -> Vec<ResourceId> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::FetchResource { id, .. } => Some(*id),
            _ => None,
        })
        .collect()
}

fn key_fetches(actions: &[Action]) -> Vec<(KeyId, String)> {
    actions
        .iter()
        .filter_map(|a| match a {
            Action::FetchKey { id, url } => Some((*id, url.clone())),
            _ => None,
        })
        .collect()
}

fn segment_ids(msns: impl IntoIterator<Item = u64>) -> Vec<ResourceId> {
    msns.into_iter()
        .map(|msn| ResourceId::Segment { msn })
        .collect()
}

fn has_discontinuity(outputs: &[Output]) -> bool {
    outputs.iter().any(|o| matches!(o, Output::Discontinuity))
}

fn ts_segment() -> Vec<u8> {
    FIXTURE0[TS_PACKET_LEN * LEADING_PACKETS_DROPPED..].to_vec()
}

fn delta_playlist(first_msn: u64, skipped: Option<u64>, last_msn: u64, key: bool) -> String {
    let mut text = format!(
        "#EXTM3U\n#EXT-X-VERSION:9\n#EXT-X-TARGETDURATION:2\n\
         #EXT-X-SERVER-CONTROL:CAN-SKIP-UNTIL=12.0\n#EXT-X-MEDIA-SEQUENCE:{first_msn}\n"
    );
    if key {
        text.push_str("#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n");
    }
    let first_listed = match skipped {
        Some(n) => {
            text.push_str(&format!("#EXT-X-SKIP:SKIPPED-SEGMENTS={n}\n"));
            first_msn + n
        }
        None => first_msn,
    };
    for msn in first_listed..=last_msn {
        text.push_str(&format!("#EXTINF:2.0,\nseg{msn}.ts\n"));
    }
    text
}

fn skip_requested(actions: &[Action]) -> bool {
    actions
        .iter()
        .any(|a| matches!(a, Action::FetchPlaylist { skip: true, .. }))
}

/// The smallest init the client accepts: an empty `moov` box.
const EMPTY_MOOV: &[u8] = b"\x00\x00\x00\x08moov";

#[test]
fn consecutive_delta_reloads_keep_numbering_and_the_aes_iv_after_the_window_slides() {
    let mut client = HlsClient::new(URL);
    drain(&mut client);

    client
        .on_playlist(delta_playlist(0, None, 9, true).as_bytes())
        .unwrap();
    let first = drain(&mut client);
    assert_eq!(resource_ids(&first), segment_ids([7, 8, 9]));
    let key_id = key_fetches(&first)[0].0;

    client
        .on_playlist(delta_playlist(3, Some(6), 10, false).as_bytes())
        .unwrap();
    let second = drain(&mut client);
    assert_eq!(resource_ids(&second), segment_ids([10]), "{second:?}");
    assert!(skip_requested(&second), "{second:?}");

    client
        .on_playlist(delta_playlist(6, Some(5), 13, false).as_bytes())
        .unwrap();
    let third = drain(&mut client);
    assert_eq!(
        resource_ids(&third),
        segment_ids([11, 12, 13]),
        "window slid past the original baseline: {third:?}"
    );

    client
        .on_playlist(delta_playlist(9, Some(4), 14, false).as_bytes())
        .unwrap();
    let fourth = drain(&mut client);
    assert_eq!(resource_ids(&fourth), segment_ids([14]), "{fourth:?}");
    assert!(
        key_fetches(&fourth).is_empty() && key_fetches(&third).is_empty(),
        "the one key must not be fetched again"
    );

    client.on_key(key_id, &KEY).unwrap();
    drain_outputs(&mut client);
    let iv = {
        let mut iv = [0u8; 16];
        iv[8..].copy_from_slice(&14u64.to_be_bytes());
        iv
    };
    let cipher = aes128_encrypt_segment(&KEY, &iv, &ts_segment());
    client
        .on_resource(ResourceId::Segment { msn: 14 }, &cipher)
        .unwrap();
    let outputs = drain_outputs(&mut client);

    let mut baseline = HlsClient::new(URL);
    drain(&mut baseline);
    baseline
        .on_playlist(
            b"#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:14\n\
              #EXTINF:2.0,\nseg14.ts\n",
        )
        .unwrap();
    drain(&mut baseline);
    baseline
        .on_resource(ResourceId::Segment { msn: 14 }, &ts_segment())
        .unwrap();
    let want = drain_outputs(&mut baseline);
    assert!(
        want.iter().any(|o| matches!(o, Output::Samples { .. })),
        "baseline must produce samples"
    );
    let got: Vec<&Output> = outputs
        .iter()
        .filter(|o| !matches!(o, Output::EndOfStream))
        .collect();
    assert_eq!(
        format!("{got:?}"),
        format!("{:?}", want.iter().collect::<Vec<_>>())
    );
}

#[test]
fn an_unmergeable_delta_requests_a_full_playlist_instead_of_misnumbering() {
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    client
        .on_playlist(delta_playlist(0, None, 9, false).as_bytes())
        .unwrap();
    drain(&mut client);

    let result = client.on_playlist(delta_playlist(40, Some(20), 49, false).as_bytes());
    assert!(result.is_err(), "an unmergeable delta must be reported");
    let actions = drain(&mut client);
    assert_eq!(
        resource_ids(&actions),
        Vec::<ResourceId>::new(),
        "{actions:?}"
    );
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, Action::FetchPlaylist { skip: false, .. })),
        "a full playlist must be requested: {actions:?}"
    );
    assert!(!skip_requested(&actions), "{actions:?}");
}

fn ll_hint_playlist() -> &'static str {
    "#EXTM3U\n#EXT-X-VERSION:9\n#EXT-X-TARGETDURATION:2\n\
     #EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=1.0\n\
     #EXT-X-PART-INF:PART-TARGET=0.5\n#EXT-X-MEDIA-SEQUENCE:0\n\
     #EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:2.0,\nseg0.m4s\n\
     #EXT-X-PART:DURATION=0.5,URI=\"p1.0.m4s\"\n\
     #EXT-X-PRELOAD-HINT:TYPE=PART,URI=\"p1.1.m4s\"\n"
}

#[test]
fn a_repeated_playlist_does_not_request_the_preload_hinted_part_twice() {
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    client.on_playlist(ll_hint_playlist().as_bytes()).unwrap();
    client.on_playlist(ll_hint_playlist().as_bytes()).unwrap();
    let hinted = ResourceId::Part { msn: 1, part: 1 };
    let ids = resource_ids(&drain(&mut client));
    assert_eq!(ids.iter().filter(|id| **id == hinted).count(), 1, "{ids:?}");
}

fn keyed_live(extra_header: &str, tag: &str) -> String {
    format!(
        "#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
         {extra_header}{tag}\n#EXTINF:2.0,\nindex0.ts\n"
    )
}

#[test]
fn a_wrong_length_key_response_lets_the_next_reload_request_the_key_again() {
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    let text = keyed_live("", "#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"");
    client.on_playlist(text.as_bytes()).unwrap();
    let (id, _) = key_fetches(&drain(&mut client))[0].clone();
    assert!(matches!(
        client.on_key(id, b"<html>error</html>"),
        Err(Error::BadKeyLength { .. })
    ));
    client.on_playlist(text.as_bytes()).unwrap();
    assert_eq!(
        key_fetches(&drain(&mut client)),
        vec![(id, "http://origin/live/key.bin".to_string())]
    );
}

fn vod_with_init() -> &'static str {
    "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
     #EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:2.0,\nseg0.m4s\n#EXT-X-ENDLIST\n"
}

#[test]
fn a_failed_init_never_ends_the_stream_cleanly_and_can_be_retried() {
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    client.on_playlist(vod_with_init().as_bytes()).unwrap();
    let ids = resource_ids(&drain(&mut client));
    assert!(ids.contains(&ResourceId::Init), "{ids:?}");
    client
        .on_resource(ResourceId::Segment { msn: 0 }, b"opaque-media")
        .unwrap();
    client.on_error(Some(ResourceId::Init));
    let outputs = drain_outputs(&mut client);
    assert!(
        !outputs.iter().any(|o| matches!(o, Output::EndOfStream)),
        "EndOfStream with zero samples and an unfetched init: {outputs:?}"
    );

    assert!(client.retry_init(), "a failed init must be retryable");
    let ids = resource_ids(&drain(&mut client));
    assert_eq!(ids, vec![ResourceId::Init]);
    assert!(
        !client.retry_init(),
        "nothing left to retry while in flight"
    );
}

#[test]
fn a_re_signed_segment_url_is_not_an_origin_restart() {
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    let pl = |first: u64, token: u32, names: &[&str]| {
        let mut text = format!(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{first}\n"
        );
        for name in names {
            text.push_str(&format!("#EXTINF:2.0,\n{name}?token={token}\n"));
        }
        text
    };
    client
        .on_playlist(pl(100, 1, &["a.ts", "b.ts"]).as_bytes())
        .unwrap();
    assert_eq!(resource_ids(&drain(&mut client)), segment_ids([100, 101]));
    drain_outputs(&mut client);
    client
        .on_playlist(pl(101, 2, &["b.ts", "c.ts"]).as_bytes())
        .unwrap();
    assert_eq!(resource_ids(&drain(&mut client)), segment_ids([102]));
    assert!(!has_discontinuity(&drain_outputs(&mut client)));
}

#[test]
fn a_different_path_or_duration_at_a_shared_msn_is_still_an_origin_restart() {
    let reload = |second: &str| {
        let mut client = HlsClient::new(URL);
        drain(&mut client);
        client
            .on_playlist(
                b"#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:100\n\
                  #EXTINF:2.0,\na.ts?token=1\n#EXTINF:2.0,\nb.ts?token=1\n",
            )
            .unwrap();
        drain(&mut client);
        drain_outputs(&mut client);
        client.on_playlist(second.as_bytes()).unwrap();
        drain_outputs(&mut client)
    };
    let other_path = reload(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:101\n\
         #EXTINF:2.0,\nz.ts?token=1\n#EXTINF:2.0,\nc.ts?token=1\n",
    );
    assert!(has_discontinuity(&other_path), "{other_path:?}");
    let other_duration = reload(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:101\n\
         #EXTINF:1.5,\nb.ts?token=2\n#EXTINF:2.0,\nc.ts?token=2\n",
    );
    assert!(has_discontinuity(&other_duration), "{other_duration:?}");
}

#[test]
fn a_hostile_target_duration_cannot_park_the_reload_for_days() {
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    client
        .on_playlist(
            b"#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4000000000\n\
              #EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:2.0,\nseg0.ts\n",
        )
        .unwrap();
    let wait = drain(&mut client)
        .into_iter()
        .find_map(|a| match a {
            Action::WaitMs(ms) => Some(ms),
            _ => None,
        })
        .expect("a non-blocking live playlist queues a wait");
    assert!(wait <= MAX_WAIT_MS, "wait {wait} ms");
}

#[test]
fn a_changed_map_byte_range_on_the_same_uri_requests_the_new_init() {
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    let head = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n";
    let first_run = "#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"100@0\"\n#EXTINF:2.0,\nseg0.m4s\n";
    let second_run = "#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"100@100\"\n#EXTINF:2.0,\nseg1.m4s\n";
    client
        .on_playlist(format!("{head}{first_run}").as_bytes())
        .unwrap();
    let first = drain(&mut client);
    assert!(
        first.iter().any(|a| matches!(
            a,
            Action::FetchResource {
                id: ResourceId::Init,
                byte_range: Some((0, 100)),
                ..
            }
        )),
        "{first:?}"
    );
    client.on_resource(ResourceId::Init, EMPTY_MOOV).unwrap();
    client
        .on_playlist(format!("{head}{first_run}{second_run}").as_bytes())
        .unwrap();
    let second = drain(&mut client);
    assert!(
        second.iter().any(|a| matches!(
            a,
            Action::FetchResource {
                id: ResourceId::Init,
                byte_range: Some((100, 100)),
                ..
            }
        )),
        "{second:?}"
    );
}

#[test]
fn one_unsupported_encrypted_segment_fails_only_that_segment() {
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    let text = "#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
                #EXTINF:2.0,\nseg0.ts\n\
                #EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"k\"\n#EXTINF:2.0,\nseg1.ts\n\
                #EXT-X-KEY:METHOD=NONE\n#EXTINF:2.0,\nseg2.ts\n";
    let err = client.on_playlist(text.as_bytes()).unwrap_err();
    assert!(
        matches!(err, Error::UnsupportedEncryption { .. }),
        "{err:?}"
    );
    let actions = drain(&mut client);
    assert_eq!(resource_ids(&actions), segment_ids([0, 2]), "{actions:?}");
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, Action::FetchPlaylist { .. })),
        "the reload must still be queued: {actions:?}"
    );
}

fn iv_hex(iv: &[u8; 16]) -> String {
    iv.iter().map(|b| format!("{b:02X}")).collect()
}

#[test]
fn aes_128_segments_with_a_map_declared_before_the_key_are_accepted() {
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    let text = keyed_live(
        "#EXT-X-MAP:URI=\"init.mp4\"\n",
        "#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"",
    );
    client.on_playlist(text.as_bytes()).unwrap();
    let actions = drain(&mut client);
    assert_eq!(key_fetches(&actions).len(), 1, "{actions:?}");
    assert_eq!(
        resource_ids(&actions),
        vec![ResourceId::Segment { msn: 0 }, ResourceId::Init]
    );
}

#[test]
fn an_aes_128_encrypted_map_is_decrypted_with_the_key_that_precedes_it() {
    let iv = [0x24u8; 16];
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    let text = format!(
        "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:0\n\
         #EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x{}\n\
         #EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:2.0,\nindex0.ts\n#EXT-X-ENDLIST\n",
        iv_hex(&iv)
    );
    client.on_playlist(text.as_bytes()).unwrap();
    let actions = drain(&mut client);
    let (key_id, _) = key_fetches(&actions)[0].clone();
    assert!(
        resource_ids(&actions).contains(&ResourceId::Init),
        "{actions:?}"
    );
    let plain_init = EMPTY_MOOV.to_vec();
    let cipher = aes128_encrypt_segment(&KEY, &iv, &plain_init);
    client.on_resource(ResourceId::Init, &cipher).unwrap();
    assert!(
        !drain_outputs(&mut client)
            .iter()
            .any(|o| matches!(o, Output::Init(_))),
        "the init must not be emitted before its key"
    );
    client.on_key(key_id, &KEY).unwrap();
    assert!(
        drain_outputs(&mut client)
            .iter()
            .any(|o| matches!(o, Output::Init(bytes) if *bytes == plain_init)),
        "the decrypted init must be emitted once the key arrives"
    );
}

#[test]
fn a_map_encrypted_with_an_unsupported_method_fails_with_a_typed_error() {
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    let text = keyed_live(
        "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"key.bin\"\n#EXT-X-MAP:URI=\"init.mp4\"\n",
        "",
    );
    let err = client.on_playlist(text.as_bytes()).unwrap_err();
    assert!(
        matches!(err, Error::UnsupportedEncryption { what, .. } if what.contains("EXT-X-MAP")),
        "{err:?}"
    );
}

#[test]
fn a_key_unreferenced_by_the_window_is_fetched_again_when_its_uri_returns() {
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    let pl = |first: u64| {
        format!(
            "#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{first}\n\
             #EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n#EXTINF:2.0,\nseg{first}.ts\n"
        )
    };
    client.on_playlist(pl(0).as_bytes()).unwrap();
    let (id, _) = key_fetches(&drain(&mut client))[0].clone();
    client.on_key(id, &KEY).unwrap();
    client.on_playlist(pl(10).as_bytes()).unwrap();
    assert_eq!(
        key_fetches(&drain(&mut client)).len(),
        1,
        "the rotated key behind the same URI must be fetched again"
    );
}
