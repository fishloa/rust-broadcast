//! HLS `METHOD=AES-128` (RFC 8216 §4.3.2.4, §5.1, §5.2) through the sans-IO
//! [`HlsClient`]: real ffmpeg classic-TS segments are encrypted in the TEST
//! with `transmux::sample_aes::aes128_encrypt_segment` using keys and IVs the
//! test derives itself, then the client's decrypted samples are compared with
//! the samples the same client produces from the plaintext segments.

use std::collections::BTreeMap;

use hls_runtime::client::{Action, Error, HlsClient, KeyId, Output, ResourceId};
use transmux::sample_aes::aes128_encrypt_segment;

const URL: &str = "http://fixture/index.m3u8";
const TS_PACKET_LEN: usize = 188;
const LEADING_PACKETS_DROPPED: usize = 3;
const FIXTURE0: &[u8] = include_bytes!("../fixtures/ts-hls/index0.ts");
const FIXTURE1: &[u8] = include_bytes!("../fixtures/ts-hls/index1.ts");
const KEY_A: [u8; 16] = [0x11; 16];
const KEY_B: [u8; 16] = [0x22; 16];
const KEY_WRONG: [u8; 16] = [0x33; 16];

/// A wrong IV corrupts only the first AES block. Both fixtures open with the
/// SDT, PAT and PMT packets, which the demuxer recovers from the repeats later
/// in the segment, so a bad IV there would go unnoticed. Dropping them puts a
/// video PES header (carrying the PTS) in the first block, where a bad IV
/// visibly changes the samples.
fn seg0() -> &'static [u8] {
    &FIXTURE0[TS_PACKET_LEN * LEADING_PACKETS_DROPPED..]
}

fn seg1() -> &'static [u8] {
    &FIXTURE1[TS_PACKET_LEN * LEADING_PACKETS_DROPPED..]
}

fn hex(b: &[u8; 16]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

fn iv_seq(base: u8) -> [u8; 16] {
    core::array::from_fn(|i| base.wrapping_add(i as u8))
}

fn key_tag(uri: &str, iv: Option<&[u8; 16]>) -> String {
    match iv {
        Some(iv) => format!("#EXT-X-KEY:METHOD=AES-128,URI=\"{uri}\",IV=0x{}", hex(iv)),
        None => format!("#EXT-X-KEY:METHOD=AES-128,URI=\"{uri}\""),
    }
}

fn playlist(msn: u64, header: &str, entries: &[(String, &str)]) -> String {
    let mut s = format!(
        "#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{msn}\n{header}"
    );
    for (tags, uri) in entries {
        if !tags.is_empty() {
            s.push_str(tags);
            s.push('\n');
        }
        s.push_str("#EXTINF:2.0,\n");
        s.push_str(uri);
        s.push('\n');
    }
    s.push_str("#EXT-X-ENDLIST\n");
    s
}

fn plain_entries() -> Vec<(String, &'static str)> {
    vec![(String::new(), "index0.ts"), (String::new(), "index1.ts")]
}

fn dbg_outputs(outputs: &[Output]) -> String {
    format!("{outputs:?}")
}

fn has_samples(outputs: &[Output]) -> bool {
    outputs.iter().any(|o| matches!(o, Output::Samples { .. }))
}

struct Run {
    outputs: Vec<Output>,
    key_fetches: Vec<(KeyId, String)>,
}

fn run_vod(
    text: &str,
    files: &BTreeMap<&str, Vec<u8>>,
    keys: &BTreeMap<&str, Vec<u8>>,
) -> Result<Run, Error> {
    let mut client = HlsClient::new(URL);
    let mut run = Run {
        outputs: Vec::new(),
        key_fetches: Vec::new(),
    };
    let mut playlists = 0;
    while let Some(action) = client.poll() {
        match action {
            Action::FetchPlaylist { .. } => {
                playlists += 1;
                assert_eq!(playlists, 1, "VOD playlist must be fetched exactly once");
                client.on_playlist(text.as_bytes())?;
            }
            Action::FetchResource { id, url, .. } => {
                let name = url.rsplit('/').next().unwrap();
                client.on_resource(id, &files[name])?;
            }
            Action::FetchKey { id, url } => {
                let name = url.rsplit('/').next().unwrap();
                run.key_fetches.push((id, url.clone()));
                client.on_key(id, &keys[name])?;
            }
            Action::WaitMs(_) => {}
            other => panic!("unexpected action {other:?}"),
        }
        while let Some(o) = client.next_output() {
            run.outputs.push(o);
        }
    }
    Ok(run)
}

fn plain_files() -> BTreeMap<&'static str, Vec<u8>> {
    BTreeMap::from([
        ("index0.ts", seg0().to_vec()),
        ("index1.ts", seg1().to_vec()),
    ])
}

fn plaintext_baseline(msn: u64) -> String {
    let text = playlist(msn, "", &plain_entries());
    let run = run_vod(&text, &plain_files(), &BTreeMap::new()).unwrap();
    assert!(has_samples(&run.outputs), "baseline must produce samples");
    assert!(run.key_fetches.is_empty());
    dbg_outputs(&run.outputs)
}

fn enc_files(
    k0: &[u8; 16],
    iv0: &[u8; 16],
    k1: &[u8; 16],
    iv1: &[u8; 16],
) -> BTreeMap<&'static str, Vec<u8>> {
    BTreeMap::from([
        ("index0.ts", aes128_encrypt_segment(k0, iv0, seg0())),
        ("index1.ts", aes128_encrypt_segment(k1, iv1, seg1())),
    ])
}

fn key_files() -> BTreeMap<&'static str, Vec<u8>> {
    BTreeMap::from([("key.bin", KEY_A.to_vec()), ("keyB.bin", KEY_B.to_vec())])
}

fn drain(client: &mut HlsClient) -> Vec<Action> {
    let mut v = Vec::new();
    while let Some(a) = client.poll() {
        v.push(a);
    }
    v
}

fn drain_outputs(client: &mut HlsClient) -> Vec<Output> {
    let mut v = Vec::new();
    while let Some(o) = client.next_output() {
        v.push(o);
    }
    v
}

#[test]
fn decrypts_ts_segment_with_explicit_iv_to_original_samples() {
    let iv0 = iv_seq(0xA0);
    let iv1 = iv_seq(0xC0);
    let entries = vec![
        (key_tag("key.bin", Some(&iv0)), "index0.ts"),
        (key_tag("key.bin", Some(&iv1)), "index1.ts"),
    ];
    let text = playlist(0, "", &entries);
    let run = run_vod(&text, &enc_files(&KEY_A, &iv0, &KEY_A, &iv1), &key_files()).unwrap();
    assert!(has_samples(&run.outputs));
    assert_eq!(dbg_outputs(&run.outputs), plaintext_baseline(0));
}

#[test]
fn iv_defaults_to_media_sequence_number_big_endian() {
    const MSN: u64 = 0x0102_0304_0506;
    let iv0: [u8; 16] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
    ];
    let iv1: [u8; 16] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x02, 0x03, 0x04, 0x05, 0x07,
    ];
    let entries = vec![
        (key_tag("key.bin", None), "index0.ts"),
        (String::new(), "index1.ts"),
    ];
    let text = playlist(MSN, "", &entries);
    let run = run_vod(&text, &enc_files(&KEY_A, &iv0, &KEY_A, &iv1), &key_files()).unwrap();
    assert!(has_samples(&run.outputs));
    assert_eq!(dbg_outputs(&run.outputs), plaintext_baseline(MSN));
}

#[test]
fn key_fetched_once_per_uri_across_segments() {
    let iv0 = iv_seq(0x10);
    let iv1 = iv_seq(0x20);
    let entries = vec![
        (key_tag("key.bin", Some(&iv0)), "index0.ts"),
        (key_tag("key.bin", Some(&iv1)), "index1.ts"),
    ];
    let text = playlist(0, "", &entries);
    let run = run_vod(&text, &enc_files(&KEY_A, &iv0, &KEY_A, &iv1), &key_files()).unwrap();
    assert_eq!(run.key_fetches.len(), 1, "{:?}", run.key_fetches);
    assert_eq!(run.key_fetches[0].1, "http://fixture/key.bin");
    assert!(has_samples(&run.outputs));
}

#[test]
fn segment_waits_for_key_then_decrypts_in_order() {
    let iv0 = iv_seq(0x30);
    let iv1 = iv_seq(0x40);
    let entries = vec![
        (key_tag("key.bin", Some(&iv0)), "index0.ts"),
        (key_tag("key.bin", Some(&iv1)), "index1.ts"),
    ];
    let text = playlist(0, "", &entries);
    let files = enc_files(&KEY_A, &iv0, &KEY_A, &iv1);
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    client.on_playlist(text.as_bytes()).unwrap();
    let actions = drain(&mut client);
    let key_id = actions
        .iter()
        .find_map(|a| match a {
            Action::FetchKey { id, .. } => Some(*id),
            _ => None,
        })
        .expect("a key fetch is issued");
    client
        .on_resource(ResourceId::Segment { msn: 1 }, &files["index1.ts"])
        .unwrap();
    client
        .on_resource(ResourceId::Segment { msn: 0 }, &files["index0.ts"])
        .unwrap();
    assert!(
        drain_outputs(&mut client).is_empty(),
        "nothing may be emitted before the key arrives"
    );
    client.on_key(key_id, &KEY_A).unwrap();
    let outputs = drain_outputs(&mut client);
    assert!(has_samples(&outputs));
    assert_eq!(dbg_outputs(&outputs), plaintext_baseline(0));
}

#[test]
fn key_rotation_two_keys_two_fetches() {
    let iv0 = iv_seq(0x50);
    let iv1 = iv_seq(0x60);
    let entries = vec![
        (key_tag("key.bin", Some(&iv0)), "index0.ts"),
        (key_tag("keyB.bin", Some(&iv1)), "index1.ts"),
    ];
    let text = playlist(0, "", &entries);
    let run = run_vod(&text, &enc_files(&KEY_A, &iv0, &KEY_B, &iv1), &key_files()).unwrap();
    assert_eq!(run.key_fetches.len(), 2, "{:?}", run.key_fetches);
    assert_ne!(run.key_fetches[0].0, run.key_fetches[1].0);
    let urls: Vec<&str> = run.key_fetches.iter().map(|(_, u)| u.as_str()).collect();
    assert!(urls.contains(&"http://fixture/key.bin"));
    assert!(urls.contains(&"http://fixture/keyB.bin"));
    assert_eq!(dbg_outputs(&run.outputs), plaintext_baseline(0));
}

#[test]
fn bad_key_length_is_error() {
    let iv0 = iv_seq(0x70);
    let iv1 = iv_seq(0x80);
    let entries = vec![
        (key_tag("key.bin", Some(&iv0)), "index0.ts"),
        (key_tag("key.bin", Some(&iv1)), "index1.ts"),
    ];
    let text = playlist(0, "", &entries);
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    client.on_playlist(text.as_bytes()).unwrap();
    let id = drain(&mut client)
        .into_iter()
        .find_map(|a| match a {
            Action::FetchKey { id, .. } => Some(id),
            _ => None,
        })
        .unwrap();
    assert!(matches!(
        client.on_key(id, &KEY_A[..15]),
        Err(Error::BadKeyLength { len: 15, .. })
    ));
    let mut long = KEY_A.to_vec();
    long.push(0);
    assert!(matches!(
        client.on_key(id, &long),
        Err(Error::BadKeyLength { len: 17, .. })
    ));
    assert!(matches!(
        client.on_key(id, &[]),
        Err(Error::BadKeyLength { len: 0, .. })
    ));
    let files = enc_files(&KEY_A, &iv0, &KEY_A, &iv1);
    client
        .on_resource(ResourceId::Segment { msn: 0 }, &files["index0.ts"])
        .unwrap();
    assert!(!has_samples(&drain_outputs(&mut client)));
    client.on_key(id, &KEY_A).unwrap();
    assert!(has_samples(&drain_outputs(&mut client)));
}

#[test]
fn wrong_key_gives_decrypt_failed_not_garbage_samples() {
    let iv0 = iv_seq(0x90);
    let iv1 = iv_seq(0xB0);
    let entries = vec![
        (key_tag("key.bin", Some(&iv0)), "index0.ts"),
        (key_tag("key.bin", Some(&iv1)), "index1.ts"),
    ];
    let text = playlist(0, "", &entries);
    let files = enc_files(&KEY_A, &iv0, &KEY_A, &iv1);
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    client.on_playlist(text.as_bytes()).unwrap();
    let id = drain(&mut client)
        .into_iter()
        .find_map(|a| match a {
            Action::FetchKey { id, .. } => Some(id),
            _ => None,
        })
        .unwrap();
    client
        .on_resource(ResourceId::Segment { msn: 0 }, &files["index0.ts"])
        .unwrap();
    client
        .on_resource(ResourceId::Segment { msn: 1 }, &files["index1.ts"])
        .unwrap();
    let err = client.on_key(id, &KEY_WRONG).unwrap_err();
    assert!(
        matches!(err, Error::DecryptFailed { key_id, .. } if key_id == id),
        "{err:?}"
    );
    assert!(!has_samples(&drain_outputs(&mut client)));
}

#[test]
fn segment_not_multiple_of_16_is_decrypt_failed() {
    let iv0 = iv_seq(0x01);
    let entries = vec![(key_tag("key.bin", Some(&iv0)), "index0.ts")];
    let text = playlist(0, "", &entries);
    let cipher = aes128_encrypt_segment(&KEY_A, &iv0, seg0());
    for bad in [&cipher[..cipher.len() - 1], &cipher[..0]] {
        let mut client = HlsClient::new(URL);
        drain(&mut client);
        client.on_playlist(text.as_bytes()).unwrap();
        let id = drain(&mut client)
            .into_iter()
            .find_map(|a| match a {
                Action::FetchKey { id, .. } => Some(id),
                _ => None,
            })
            .unwrap();
        client.on_key(id, &KEY_A).unwrap();
        let err = client
            .on_resource(ResourceId::Segment { msn: 0 }, bad)
            .unwrap_err();
        assert!(matches!(err, Error::DecryptFailed { .. }), "{err:?}");
        assert!(!has_samples(&drain_outputs(&mut client)));
    }
}

#[test]
fn unsupported_encryption_is_an_error_never_plaintext() {
    let iv = iv_seq(0x02);
    let mut cases: Vec<(String, String, Option<&str>)> = Vec::new();
    for method in ["SAMPLE-AES", "SAMPLE-AES-CTR", "AES-256-GCM"] {
        let tag = format!("#EXT-X-KEY:METHOD={method},URI=\"key.bin\"");
        cases.push((method.into(), playlist(0, "", &[(tag, "index0.ts")]), None));
    }
    cases.push((
        "encrypted EXT-X-MAP".into(),
        playlist(
            0,
            "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"key.bin\"\n#EXT-X-MAP:URI=\"init.mp4\"\n",
            &[(key_tag("key.bin", Some(&iv)), "index0.ts")],
        ),
        Some("EXT-X-MAP"),
    ));
    cases.push((
        "I-FRAMES-ONLY with AES-128".into(),
        playlist(
            0,
            "#EXT-X-I-FRAMES-ONLY\n",
            &[(key_tag("key.bin", Some(&iv)), "index0.ts")],
        ),
        Some("I-FRAMES-ONLY"),
    ));
    for (label, text, what_contains) in cases {
        let mut client = HlsClient::new(URL);
        drain(&mut client);
        let err = client.on_playlist(text.as_bytes()).unwrap_err();
        assert!(
            matches!(&err, Error::UnsupportedEncryption { what, .. }
                if what_contains.is_none_or(|needle| what.contains(needle))),
            "{label}: {err:?}"
        );
        let actions = drain(&mut client);
        assert!(
            !actions
                .iter()
                .any(|a| matches!(a, Action::FetchResource { .. } | Action::FetchKey { .. })),
            "{label}: {actions:?}"
        );
    }
}

#[test]
fn plaintext_playlist_unchanged_no_key_actions() {
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    client
        .on_playlist(playlist(0, "", &plain_entries()).as_bytes())
        .unwrap();
    let actions = drain(&mut client);
    let resources = actions
        .iter()
        .filter(|a| matches!(a, Action::FetchResource { .. }))
        .count();
    assert_eq!(resources, 2, "{actions:?}");
    assert!(
        !actions.iter().any(|a| matches!(a, Action::FetchKey { .. })),
        "{actions:?}"
    );
    assert!(has_samples(
        &run_vod(
            &playlist(0, "", &plain_entries()),
            &plain_files(),
            &BTreeMap::new()
        )
        .unwrap()
        .outputs
    ));
}

#[test]
fn method_none_after_aes_segment_is_plaintext() {
    let iv0 = iv_seq(0x04);
    let entries = vec![
        (key_tag("key.bin", Some(&iv0)), "index0.ts"),
        ("#EXT-X-KEY:METHOD=NONE".to_string(), "index1.ts"),
    ];
    let text = playlist(0, "", &entries);
    let files = BTreeMap::from([
        ("index0.ts", aes128_encrypt_segment(&KEY_A, &iv0, seg0())),
        ("index1.ts", seg1().to_vec()),
    ]);
    let run = run_vod(&text, &files, &key_files()).unwrap();
    assert_eq!(run.key_fetches.len(), 1);
    assert_eq!(dbg_outputs(&run.outputs), plaintext_baseline(0));
}

#[test]
fn on_key_error_then_reload_requests_key_again() {
    let iv0 = iv_seq(0x05);
    let iv1 = iv_seq(0x06);
    let entries = vec![
        (key_tag("key.bin", Some(&iv0)), "index0.ts"),
        (key_tag("key.bin", Some(&iv1)), "index1.ts"),
    ];
    let text = playlist(0, "", &entries);
    let files = enc_files(&KEY_A, &iv0, &KEY_A, &iv1);
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    client.on_playlist(text.as_bytes()).unwrap();
    let first = drain(&mut client);
    let id = first
        .iter()
        .find_map(|a| match a {
            Action::FetchKey { id, .. } => Some(*id),
            _ => None,
        })
        .unwrap();
    client.on_key_error(id);
    assert!(
        !drain(&mut client)
            .iter()
            .any(|a| matches!(a, Action::FetchKey { .. })),
        "no automatic retry"
    );
    client.on_playlist(text.as_bytes()).unwrap();
    let again = drain(&mut client);
    let refetch: Vec<_> = again
        .iter()
        .filter_map(|a| match a {
            Action::FetchKey { id, url } => Some((*id, url.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(refetch, vec![(id, "http://fixture/key.bin".to_string())]);
    client
        .on_resource(ResourceId::Segment { msn: 0 }, &files["index0.ts"])
        .unwrap();
    client
        .on_resource(ResourceId::Segment { msn: 1 }, &files["index1.ts"])
        .unwrap();
    client.on_key(id, &KEY_A).unwrap();
    assert_eq!(
        dbg_outputs(&drain_outputs(&mut client)),
        plaintext_baseline(0)
    );
}

#[test]
fn client_debug_never_prints_key_octets() {
    let iv0 = iv_seq(0x07);
    let text = playlist(0, "", &[(key_tag("key.bin", Some(&iv0)), "index0.ts")]);
    let mut client = HlsClient::new(URL);
    drain(&mut client);
    client.on_playlist(text.as_bytes()).unwrap();
    let id = drain(&mut client)
        .into_iter()
        .find_map(|a| match a {
            Action::FetchKey { id, .. } => Some(id),
            _ => None,
        })
        .unwrap();
    let secret: [u8; 16] = [
        0xDE, 0xAD, 0xBE, 0xEF, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
    ];
    client.on_key(id, &secret).unwrap();
    let shown = format!("{client:?}");
    assert!(shown.contains("redacted"), "{shown}");
    assert!(!shown.contains("222, 173, 190, 239"), "{shown}");
    assert!(!shown.contains("DEAD"), "{shown}");
}
