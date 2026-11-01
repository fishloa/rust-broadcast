//! `MediaPlaylist::segment_keys` — RFC 8216 §4.3.2.4 key carry-forward.

use broadcast_hls::{EncryptionMethod, Error, MediaPlaylist};

const KEY_A: &str = "#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/a.key\"";
const KEY_B: &str =
    "#EXT-X-KEY:METHOD=AES-128,URI=\"https://k/b.key\",IV=0x000102030405060708090A0B0C0D0E0F";
const KEY_NONE: &str = "#EXT-X-KEY:METHOD=NONE";
const FAIRPLAY: &str = "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\",KEYFORMAT=\"com.apple.streamingkeydelivery\",KEYFORMATVERSIONS=\"1\"";

fn playlist(segments: &[(&[&str], &str)]) -> String {
    let mut s = String::from(
        "#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:0\n",
    );
    for (tags, uri) in segments {
        for t in *tags {
            s.push_str(t);
            s.push('\n');
        }
        s.push_str("#EXTINF:4.0,\n");
        s.push_str(uri);
        s.push('\n');
    }
    s.push_str("#EXT-X-ENDLIST\n");
    s
}

fn uris(p: &MediaPlaylist) -> Vec<Option<String>> {
    p.segment_keys()
        .unwrap()
        .into_iter()
        .map(|k| k.and_then(|k| k.uri))
        .collect()
}

#[test]
fn key_applies_to_following_segments_until_next_key() {
    let p = MediaPlaylist::parse(&playlist(&[
        (&[KEY_A], "s0.ts"),
        (&[], "s1.ts"),
        (&[], "s2.ts"),
        (&[KEY_B], "s3.ts"),
        (&[], "s4.ts"),
    ]))
    .unwrap();
    let a = Some("https://k/a.key".to_string());
    let b = Some("https://k/b.key".to_string());
    assert_eq!(uris(&p), vec![a.clone(), a.clone(), a, b.clone(), b]);
}

#[test]
fn method_none_resets_key() {
    let p = MediaPlaylist::parse(&playlist(&[
        (&[KEY_A], "s0.ts"),
        (&[], "s1.ts"),
        (&[KEY_NONE], "s2.ts"),
        (&[], "s3.ts"),
    ]))
    .unwrap();
    let keys = p.segment_keys().unwrap();
    assert_eq!(keys.len(), 4);
    assert!(keys[0].is_some() && keys[1].is_some());
    assert!(keys[2].is_none() && keys[3].is_none());
}

#[test]
fn key_rotation_per_segment() {
    let p = MediaPlaylist::parse(&playlist(&[
        (&[KEY_A], "s0.ts"),
        (&[KEY_B], "s1.ts"),
        (&[KEY_A], "s2.ts"),
    ]))
    .unwrap();
    assert_eq!(
        uris(&p),
        vec![
            Some("https://k/a.key".to_string()),
            Some("https://k/b.key".to_string()),
            Some("https://k/a.key".to_string()),
        ]
    );
}

#[test]
fn keyformat_other_than_identity_ignored_when_identity_present() {
    let p =
        MediaPlaylist::parse(&playlist(&[(&[FAIRPLAY, KEY_A], "s0.ts"), (&[], "s1.ts")])).unwrap();
    let keys = p.segment_keys().unwrap();
    assert_eq!(keys.len(), 2);
    for k in keys {
        let k = k.unwrap();
        assert_eq!(k.method, EncryptionMethod::Aes128);
        assert_eq!(k.uri.as_deref(), Some("https://k/a.key"));
    }
}

#[test]
fn keys_survive_parse_render_parse() {
    let p = MediaPlaylist::parse(&playlist(&[
        (&[KEY_A], "s0.ts"),
        (&[], "s1.ts"),
        (&[KEY_B], "s2.ts"),
        (&[KEY_NONE], "s3.ts"),
    ]))
    .unwrap();
    let again = MediaPlaylist::parse(&p.to_m3u8().unwrap()).unwrap();
    let keys = p.segment_keys().unwrap();
    assert_eq!(keys, again.segment_keys().unwrap());
    assert!(keys[0].is_some() && keys[2].is_some() && keys[3].is_none());
}

#[test]
fn method_none_ends_a_non_identity_key_format() {
    let p = MediaPlaylist::parse(&playlist(&[
        (&[FAIRPLAY], "a.ts"),
        (&[KEY_NONE], "b.ts"),
        (&[], "c.ts"),
    ]))
    .unwrap();
    let r = p.segment_key_results().unwrap();
    assert_eq!(
        r[0],
        Err(Error::UnsupportedKeyFormat {
            keyformat: "com.apple.streamingkeydelivery".to_string()
        })
    );
    assert_eq!(r[1], Ok(None));
    assert_eq!(r[2], Ok(None));
}

#[test]
fn method_none_after_fairplay_leaves_following_segments_clear() {
    let p = MediaPlaylist::parse(&playlist(&[
        (&[FAIRPLAY, KEY_A], "a.ts"),
        (&[KEY_NONE], "b.ts"),
        (&[], "c.ts"),
    ]))
    .unwrap();
    let keys = p.segment_keys().unwrap();
    assert!(keys[0].is_some());
    assert_eq!(&keys[1..], &[None, None]);
}

#[test]
fn segment_key_resolution_cases() {
    type Check = fn(Result<Vec<Option<broadcast_hls::MediaKey>>, Error>) -> Result<(), String>;
    let cases: Vec<(&str, String, Check)> = vec![
        (
            "only a non-identity KEYFORMAT is an unsupported error",
            playlist(&[(&[FAIRPLAY], "s0.ts")]),
            |r| {
                (r == Err(Error::UnsupportedKeyFormat {
                    keyformat: "com.apple.streamingkeydelivery".to_string(),
                }))
                .then_some(())
                .ok_or(format!("{r:?}"))
            },
        ),
        (
            "a key without a URI is an error",
            playlist(&[(&["#EXT-X-KEY:METHOD=AES-128"], "s0.ts")]),
            |r| {
                (r == Err(Error::KeyWithoutUri {
                    method: EncryptionMethod::Aes128,
                }))
                .then_some(())
                .ok_or(format!("{r:?}"))
            },
        ),
        (
            "the IV attribute parses as 128 bits",
            playlist(&[(&[KEY_B], "s0.ts")]),
            |r| {
                let iv = r
                    .map_err(|e| format!("{e:?}"))?
                    .remove(0)
                    .and_then(|k| k.iv);
                let want: [u8; 16] = core::array::from_fn(|i| i as u8);
                (iv == Some(want)).then_some(()).ok_or(format!("{iv:?}"))
            },
        ),
        (
            "no key tag means no key",
            playlist(&[(&[], "s0.ts"), (&[], "s1.ts")]),
            |r| {
                (r == Ok(vec![None, None]))
                    .then_some(())
                    .ok_or(format!("{r:?}"))
            },
        ),
    ];
    let wrong: Vec<_> = cases
        .into_iter()
        .filter_map(|(label, text, check)| {
            let p = MediaPlaylist::parse(&text).unwrap();
            check(p.segment_keys())
                .err()
                .map(|e| format!("{label}: {e}"))
        })
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}
