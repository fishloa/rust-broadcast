//! A key that arrives after the encrypted init it decrypts failed must still
//! decrypt the segments queued behind it (issue #3951): the failing init is
//! reported, the segments wait for the refetched init, and the session ends.

use test_harness::fmp4_split;

use fmp4_split::split;
use hls_runtime::client::{Action, Error, HlsClient, Output, ResourceId};
use transmux::sample_aes::aes128_encrypt_segment;
use url::Url;

const URL: &str = "http://o/v/index.m3u8";
const FIXTURE: &str = "mp4/h264-aac-frag.mp4";
const KEY: [u8; 16] = [0x5a; 16];
const IV: [u8; 16] = [0x11; 16];
const NOT_A_CIPHER_BLOCK_MULTIPLE: usize = 17;
const GARBAGE: &[u8] = b"not an iso bmff init, no moov box in here";

fn drain(client: &mut HlsClient) -> Vec<Action> {
    std::iter::from_fn(|| client.poll()).collect()
}

fn outputs(client: &mut HlsClient) -> Vec<Output> {
    std::iter::from_fn(|| client.next_output()).collect()
}

fn segment(i: u64) -> ResourceId {
    ResourceId::Segment {
        msn: broadcast_hls::MediaSequence::new(i),
    }
}

fn run(bad_init: Vec<u8>, expected: impl Fn(&Error) -> bool) {
    let bytes = test_fixtures::read(FIXTURE);
    let parts = split(&bytes).unwrap();
    let iv: String = IV.iter().map(|b| format!("{b:02X}")).collect();
    let mut playlist = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n#EXT-X-PLAYLIST-TYPE:VOD\n\
         #EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x{iv}\n#EXT-X-MAP:URI=\"init.mp4\"\n"
    );
    for i in 0..parts.segments.len() {
        playlist.push_str(&format!("#EXTINF:1.0,\ns{i}.m4s\n"));
    }
    playlist.push_str("#EXT-X-ENDLIST\n");

    let mut client = HlsClient::new(hls_runtime::client::HlsClientConfig::new(
        Url::parse(URL).unwrap(),
    ));
    drain(&mut client);
    client.on_playlist(playlist.as_bytes()).unwrap();
    let key_id = drain(&mut client)
        .into_iter()
        .find_map(|a| match a {
            Action::FetchKey { id, .. } => Some(id),
            _ => None,
        })
        .expect("a key fetch is issued");
    client.on_resource(ResourceId::Init, &bad_init).unwrap();
    for (i, seg) in parts.segments.iter().enumerate() {
        let cipher = aes128_encrypt_segment(&KEY, &IV, seg);
        client.on_resource(segment(i as u64), &cipher).unwrap();
    }
    assert!(outputs(&mut client).is_empty());

    let err = client.on_key(key_id, &KEY).unwrap_err();
    assert!(expected(&err), "{err:?}");

    assert!(client.retry_init(), "the failed init can be retried");
    let refetch = drain(&mut client).into_iter().any(|a| {
        matches!(
            a,
            Action::FetchResource {
                id: ResourceId::Init,
                ..
            }
        )
    });
    assert!(refetch, "the failed init is requested again");
    let init = aes128_encrypt_segment(&KEY, &IV, &parts.init);
    client.on_resource(ResourceId::Init, &init).unwrap();
    drain(&mut client);
    let out = outputs(&mut client);
    assert!(
        out.iter().any(|o| matches!(o, Output::Samples { .. })),
        "queued segments play after the init refetch: {out:?}"
    );
    assert!(
        out.iter().any(|o| matches!(o, Output::EndOfStream)),
        "the session ends: {out:?}"
    );
}

#[test]
fn segments_behind_a_key_play_when_the_init_it_decrypts_fails_to_decrypt() {
    run(vec![0; NOT_A_CIPHER_BLOCK_MULTIPLE], |e| {
        matches!(e, Error::DecryptFailed { .. })
    });
}

#[test]
fn segments_behind_a_key_play_when_the_init_it_decrypts_is_not_an_init() {
    run(aes128_encrypt_segment(&KEY, &IV, GARBAGE), |e| {
        matches!(e, Error::InitUndecodable)
    });
}
