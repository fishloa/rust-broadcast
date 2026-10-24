mod support;

use broadcast_client::{Action, Config, Credentials, ErrorKind, Event, Session, Status};
use support::hls_host::*;
use transmux::sample_aes::aes128_encrypt_segment;

const BASE: &str = "http://origin.test/hls";
const KEY_URL: &str = "http://origin.test/hls/key.bin";
const KEY2_URL: &str = "http://origin.test/hls/key2.bin";
const KEY_HOP_A: &str = "http://origin.test/hls/hop-a.bin";
const KEY_HOP_B: &str = "http://origin.test/hls/hop-b.bin";
const OTHER_ORIGIN_KEY_URL: &str = "http://keys.test/key.bin";
const SEG2_URL: &str = "http://origin.test/hls/index2.ts";
const KEY_A: [u8; 16] = [0x11; 16];
const KEY_B: [u8; 16] = [0x22; 16];
const KEY_WRONG: [u8; 16] = [0x33; 16];
const TS_PACKET_LEN: usize = 188;
const LEADING_PACKETS_DROPPED: usize = 3;
const BEARER: &str = "key-token";
const COOKIE_NAME: &str = "sid";
const COOKIE_VALUE: &str = "abc123";
const CUSTOM_HEADER: &str = "X-Key-Test";
const KEY_TIMEOUT_US: u64 = 5 * SEC;
const OVERSIZE_KEY_LEN: usize = 4097;
const MIN_SAMPLES: usize = 100;

fn plain_seg(n: usize) -> Vec<u8> {
    ts_segment(n % 2)[TS_PACKET_LEN * LEADING_PACKETS_DROPPED..].to_vec()
}

fn iv_seq(base: u8) -> [u8; 16] {
    core::array::from_fn(|i| base.wrapping_add(u8::try_from(i).unwrap()))
}

fn msn_iv(msn: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[8..].copy_from_slice(&msn.to_be_bytes());
    iv
}

fn hex(b: &[u8; 16]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect()
}

fn key_tag(uri: &str, iv: Option<&[u8; 16]>) -> String {
    match iv {
        Some(iv) => format!("#EXT-X-KEY:METHOD=AES-128,URI=\"{uri}\",IV=0x{}\n", hex(iv)),
        None => format!("#EXT-X-KEY:METHOD=AES-128,URI=\"{uri}\"\n"),
    }
}

fn playlist(msn: u64, entries: &[(String, String)], endlist: bool) -> Vec<u8> {
    let mut s = format!(
        "#EXTM3U\n#EXT-X-VERSION:5\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:{msn}\n"
    );
    if endlist {
        s.push_str("#EXT-X-PLAYLIST-TYPE:VOD\n");
    }
    for (tags, uri) in entries {
        s.push_str(tags);
        s.push_str("#EXTINF:2.0,\n");
        s.push_str(uri);
        s.push('\n');
    }
    if endlist {
        s.push_str("#EXT-X-ENDLIST\n");
    }
    s.into_bytes()
}

fn entry(tags: String, uri: &str) -> (String, String) {
    (tags, uri.to_string())
}

fn seg_url(n: usize) -> String {
    format!("{BASE}/index{n}.ts")
}

fn plain_host(config: Config) -> Host {
    let entries = vec![
        entry(String::new(), "index0.ts"),
        entry(String::new(), "index1.ts"),
    ];
    let mut h = Host::new(TS_PLAYLIST_URL, config);
    h.ok(TS_PLAYLIST_URL, playlist(0, &entries, true));
    h.ok(TS_SEG0_URL, plain_seg(0));
    h.ok(TS_SEG1_URL, plain_seg(1));
    h
}

fn baseline() -> Vec<Event> {
    let mut h = plain_host(hls_config());
    h.run_to_end();
    let data = data_events(&h.events);
    assert!(h.samples() > MIN_SAMPLES, "baseline must produce samples");
    data
}

fn encrypted_host_with(config: Config, key_url: &str, key_replies: Vec<Reply>) -> Host {
    let iv0 = iv_seq(0x10);
    let iv1 = iv_seq(0x50);
    let entries = vec![
        entry(key_tag(key_url, Some(&iv0)), "index0.ts"),
        entry(key_tag(key_url, Some(&iv1)), "index1.ts"),
    ];
    let mut h = Host::new(TS_PLAYLIST_URL, config);
    h.ok(TS_PLAYLIST_URL, playlist(0, &entries, true));
    h.ok(
        TS_SEG0_URL,
        aes128_encrypt_segment(&KEY_A, &iv0, &plain_seg(0)),
    );
    h.ok(
        TS_SEG1_URL,
        aes128_encrypt_segment(&KEY_A, &iv1, &plain_seg(1)),
    );
    h.route(key_url, key_replies);
    h
}

fn encrypted_host(config: Config, key_replies: Vec<Reply>) -> Host {
    encrypted_host_with(config, KEY_URL, key_replies)
}

fn good_key() -> Reply {
    Reply::Ok(KEY_A.to_vec())
}

fn errors(events: &[Event]) -> Vec<(ErrorKind, bool)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Error { kind, fatal } => Some((kind.clone(), *fatal)),
            _ => None,
        })
        .collect()
}

fn key_requests<'a>(h: &'a Host, url: &str) -> Vec<&'a Seen> {
    h.requests_to(url)
}

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn key_failed(status: Option<u16>) -> ErrorKind {
    ErrorKind::KeyFetchFailed { status }
}

fn has_fatal(events: &[Event]) -> bool {
    errors(events).iter().any(|(_, f)| *f)
}

fn run_until_fatal(h: &mut Host) {
    h.run_until(has_fatal, 10_000);
}

fn count_samples(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, Event::Sample { .. }))
        .count()
}

#[test]
fn encrypted_media_playlist_decrypts_to_same_samples_as_plaintext() {
    let expected = baseline();
    let mut h = encrypted_host(hls_config(), vec![good_key()]);
    h.run_to_end();
    assert_eq!(data_events(&h.events), expected);
    assert!(errors(&h.events).is_empty(), "{:?}", errors(&h.events));
    assert_eq!(h.events.iter().filter(|e| is_ended(e)).count(), 1);
}

#[test]
fn key_requested_once_for_all_segments() {
    let iv = iv_seq(0x20);
    let tag = key_tag(KEY_URL, Some(&iv));
    let entries = vec![
        entry(tag, "index0.ts"),
        entry(String::new(), "index1.ts"),
        entry(String::new(), "index2.ts"),
        entry(String::new(), "index3.ts"),
    ];
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.ok(TS_PLAYLIST_URL, playlist(0, &entries, true));
    for n in 0..4 {
        h.ok(
            &seg_url(n),
            aes128_encrypt_segment(&KEY_A, &iv, &plain_seg(n)),
        );
    }
    h.ok(KEY_URL, KEY_A.to_vec());
    h.run_to_end();
    assert_eq!(h.hits(KEY_URL), 1, "key fetched per segment");
    for n in 0..4 {
        assert_eq!(h.hits(&seg_url(n)), 1);
    }
    assert!(h.samples() > MIN_SAMPLES);
    assert!(errors(&h.events).is_empty());
    assert!(h.events.iter().any(is_ended));
}

#[test]
fn key_request_carries_bearer_authorization_for_session_origin() {
    let mut h = encrypted_host(hls_config(), vec![good_key()]);
    h.session
        .set_credentials(Credentials::bearer(BEARER))
        .unwrap();
    h.run_to_end();
    let keys = key_requests(&h, KEY_URL);
    assert_eq!(keys.len(), 1);
    let expected = format!("Bearer {BEARER}");
    assert_eq!(keys[0].authorization(), Some(expected.as_str()));
    assert!(h.events.iter().any(is_ended));
}

fn cookie_config() -> Config {
    Config {
        extra_headers: vec![(CUSTOM_HEADER.to_string(), "1".to_string())],
        initial_cookies: vec![(COOKIE_NAME.to_string(), COOKIE_VALUE.to_string())],
        ..hls_config()
    }
}

#[test]
fn key_request_carries_cookie_and_extra_headers_for_session_origin() {
    let mut h = encrypted_host(cookie_config(), vec![good_key()]);
    h.run_to_end();
    let keys = key_requests(&h, KEY_URL);
    assert_eq!(keys.len(), 1);
    let cookie = header(keys[0], "cookie").expect("cookie header on the key request");
    assert!(
        cookie.contains(&format!("{COOKIE_NAME}={COOKIE_VALUE}")),
        "{cookie}"
    );
    assert_eq!(header(keys[0], CUSTOM_HEADER), Some("1"));
    assert!(h.events.iter().any(is_ended));
}

#[test]
fn key_on_other_origin_carries_no_authorization_cookie_or_extra_headers() {
    let mut h = encrypted_host_with(cookie_config(), OTHER_ORIGIN_KEY_URL, vec![good_key()]);
    h.session
        .set_credentials(Credentials::bearer(BEARER))
        .unwrap();
    h.run_to_end();
    let keys = key_requests(&h, OTHER_ORIGIN_KEY_URL);
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].authorization(), None);
    assert_eq!(header(keys[0], "cookie"), None);
    assert_eq!(header(keys[0], CUSTOM_HEADER), None);
    let playlist_req = key_requests(&h, TS_PLAYLIST_URL);
    assert!(playlist_req[0].authorization().is_some());
    assert!(header(playlist_req[0], "cookie").is_some());
    assert!(header(playlist_req[0], CUSTOM_HEADER).is_some());
    assert!(h.events.iter().any(is_ended));
}

#[test]
fn key_401_emits_auth_required_then_replays_with_credentials() {
    let challenge = vec![(
        "WWW-Authenticate".to_string(),
        "Basic realm=\"keys\"".to_string(),
    )];
    let mut h = encrypted_host(
        hls_config(),
        vec![Reply::StatusHeaders(401, challenge), good_key()],
    );
    h.run_until(
        |evs| evs.iter().any(|e| matches!(e, Event::AuthRequired { .. })),
        1000,
    );
    let required = h
        .events
        .iter()
        .find(|e| matches!(e, Event::AuthRequired { .. }))
        .expect("AuthRequired");
    match required {
        Event::AuthRequired { origin, realm, .. } => {
            assert_eq!(origin.as_str(), format!("{ORIGIN}/hls/index.m3u8"));
            assert_eq!(realm.as_deref(), Some("keys"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(h.hits(KEY_URL), 1);
    assert!(errors(&h.events).is_empty(), "401 is not an Error event");
    assert!(!h.events.iter().any(is_ended));

    h.session
        .set_credentials(Credentials::new("user", "pass"))
        .unwrap();
    h.run_to_end();
    let keys = key_requests(&h, KEY_URL);
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].authorization(), None);
    assert!(
        keys[1]
            .authorization()
            .is_some_and(|a| a.starts_with("Basic ")),
        "{:?}",
        keys[1].headers
    );
    assert!(errors(&h.events).is_empty());
    assert!(h.samples() > MIN_SAMPLES);
    assert!(h.events.iter().any(is_ended));
}

#[test]
fn key_redirect_followed_and_counted() {
    let mut h = encrypted_host(hls_config(), vec![Reply::Redirect(KEY_HOP_A.to_string())]);
    h.ok(KEY_HOP_A, KEY_A.to_vec());
    h.run_to_end();
    assert_eq!(h.hits(KEY_URL), 1);
    assert_eq!(h.hits(KEY_HOP_A), 1);
    assert!(h.events.iter().any(is_ended));
    assert!(errors(&h.events).is_empty());

    let config = Config {
        max_redirects: 1,
        ..hls_config()
    };
    let mut h = encrypted_host(
        config,
        vec![Reply::Redirect(KEY_HOP_A.to_string()), good_key()],
    );
    h.route(KEY_HOP_A, vec![Reply::Redirect(KEY_HOP_B.to_string())]);
    h.ok(KEY_HOP_B, KEY_A.to_vec());
    h.run_to_end();
    assert_eq!(
        h.hits(KEY_HOP_B),
        0,
        "second hop exceeds the per-request limit"
    );
    assert_eq!(
        h.hits(KEY_URL),
        2,
        "retry starts with a fresh redirect count"
    );
    assert_eq!(errors(&h.events), vec![(key_failed(None), false)]);
    assert!(h.events.iter().any(is_ended));
}

#[test]
fn key_oversized_body_closes_and_fails() {
    let mut h = encrypted_host(
        hls_config(),
        vec![Reply::Ok(vec![0x5A; OVERSIZE_KEY_LEN]), good_key()],
    );
    h.run_to_end();
    let keys = key_requests(&h, KEY_URL);
    assert_eq!(keys.len(), 2);
    assert!(
        h.closes.contains(&keys[0].conn),
        "oversized key connection must be closed"
    );
    assert_eq!(errors(&h.events), vec![(key_failed(None), false)]);
    assert!(h.events.iter().any(is_ended));
}

#[test]
fn key_wrong_length_calls_on_key_error_and_retries_with_backoff() {
    let mut h = encrypted_host(
        hls_config(),
        vec![
            Reply::Ok(KEY_A[..15].to_vec()),
            Reply::Ok([KEY_A.as_slice(), &[0]].concat()),
            good_key(),
        ],
    );
    h.run_to_end();
    let times: Vec<u64> = key_requests(&h, KEY_URL).iter().map(|s| s.at_us).collect();
    assert_eq!(times.len(), 3, "{times:?}");
    assert_eq!(times[1] - times[0], 2_000 * MS);
    assert_eq!(times[2] - times[1], 4_000 * MS);
    assert_eq!(errors(&h.events), vec![(ErrorKind::BadKey, false)]);
    let attempts: Vec<u64> = statuses(&h.events)
        .into_iter()
        .filter_map(|s| match s {
            Status::Reconnecting { attempt } => Some(attempt),
            _ => None,
        })
        .collect();
    assert_eq!(attempts, vec![1, 2]);
    assert_eq!(data_events(&h.events), baseline());
}

#[test]
fn key_404_retries_not_not_found_status() {
    let mut h = encrypted_host(hls_config(), vec![Reply::Status(404), good_key()]);
    h.run_to_end();
    let st = statuses(&h.events);
    assert!(!st.contains(&Status::NotFound), "{st:?}");
    assert!(st.contains(&Status::Reconnecting { attempt: 1 }));
    assert_eq!(h.hits(KEY_URL), 2);
    assert_eq!(errors(&h.events), vec![(key_failed(Some(404)), false)]);
    assert!(h.events.iter().any(is_ended));
}

#[test]
fn key_timeout_aborts_and_retries() {
    let config = Config {
        hls_key_timeout_us: KEY_TIMEOUT_US,
        ..hls_config()
    };
    let mut h = encrypted_host(config, vec![Reply::Hang, good_key()]);
    h.run_to_end();
    let keys = key_requests(&h, KEY_URL);
    assert_eq!(keys.len(), 2);
    assert!(
        h.closes.contains(&keys[0].conn),
        "timed-out key must be closed"
    );
    assert!(keys[1].at_us >= keys[0].at_us + KEY_TIMEOUT_US);
    assert_eq!(errors(&h.events), vec![(key_failed(None), false)]);
    assert!(h.events.iter().any(is_ended));
}

#[test]
fn wrong_key_emits_decrypt_error_event_and_retries() {
    let mut h = encrypted_host(hls_config(), vec![Reply::Ok(KEY_WRONG.to_vec())]);
    run_until_fatal(&mut h);
    let errs = errors(&h.events);
    assert_eq!(
        errs,
        vec![
            (ErrorKind::DecryptFailed, false),
            (ErrorKind::DecryptFailed, true)
        ]
    );
    assert_eq!(
        count_samples(&h.events),
        0,
        "wrong key must not yield samples"
    );
    assert!(
        h.hits(TS_SEG0_URL) > 1,
        "the failed segment is retried before giving up"
    );
    assert!(!h.events.iter().any(is_ended));
}

fn sample_aes_playlist(key_line: &str) -> Vec<u8> {
    playlist(0, &[entry(key_line.to_string(), "index0.ts")], true)
}

fn unsupported_host(key_line: &str) -> Host {
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.ok(TS_PLAYLIST_URL, sample_aes_playlist(key_line));
    h.ok(TS_SEG0_URL, plain_seg(0));
    h.ok(KEY_URL, KEY_A.to_vec());
    h
}

#[test]
fn sample_aes_emits_unsupported_encryption_event_no_samples() {
    let mut h = unsupported_host(&format!(
        "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"{KEY_URL}\",IV=0x{}\n",
        hex(&iv_seq(1))
    ));
    h.run_until(has_fatal, 1000);
    h.run_until(|_| false, 200);
    assert_eq!(
        errors(&h.events),
        vec![(
            ErrorKind::UnsupportedEncryption {
                method: "SAMPLE-AES".to_string()
            },
            true
        )]
    );
    assert_eq!(count_samples(&h.events), 0);
    assert_eq!(h.hits(KEY_URL), 0);
    assert_eq!(h.hits(TS_SEG0_URL), 0);
}

#[test]
fn non_identity_keyformat_emits_error_event() {
    const FAIRPLAY: &str = "com.apple.streamingkeydelivery";
    let mut h = unsupported_host(&format!(
        "#EXT-X-KEY:METHOD=AES-128,URI=\"{KEY_URL}\",KEYFORMAT=\"{FAIRPLAY}\"\n"
    ));
    h.run_until(has_fatal, 1000);
    assert_eq!(
        errors(&h.events),
        vec![(
            ErrorKind::UnsupportedEncryption {
                method: FAIRPLAY.to_string()
            },
            true
        )]
    );
    assert_eq!(count_samples(&h.events), 0);
    assert_eq!(h.hits(KEY_URL), 0);
}

#[test]
fn key_without_uri_is_fatal_unsupported_encryption() {
    let mut h = unsupported_host("#EXT-X-KEY:METHOD=AES-128\n");
    h.run_until(has_fatal, 1000);
    assert_eq!(
        errors(&h.events),
        vec![(
            ErrorKind::UnsupportedEncryption {
                method: "AES-128".to_string()
            },
            true
        )]
    );
    assert_eq!(count_samples(&h.events), 0);
}

#[test]
fn key_rotation_fetches_second_key() {
    let iv0 = iv_seq(0x30);
    let iv1 = iv_seq(0x70);
    let entries = vec![
        entry(key_tag(KEY_URL, Some(&iv0)), "index0.ts"),
        entry(key_tag(KEY2_URL, Some(&iv1)), "index1.ts"),
    ];
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.ok(TS_PLAYLIST_URL, playlist(0, &entries, true));
    h.ok(
        TS_SEG0_URL,
        aes128_encrypt_segment(&KEY_A, &iv0, &plain_seg(0)),
    );
    h.ok(
        TS_SEG1_URL,
        aes128_encrypt_segment(&KEY_B, &iv1, &plain_seg(1)),
    );
    h.ok(KEY_URL, KEY_A.to_vec());
    h.ok(KEY2_URL, KEY_B.to_vec());
    h.run_to_end();
    assert_eq!(h.hits(KEY_URL), 1);
    assert_eq!(h.hits(KEY2_URL), 1);
    assert_eq!(data_events(&h.events), baseline());
    assert!(errors(&h.events).is_empty());
}

#[test]
fn session_reconnect_keeps_key_cache_or_refetches() {
    let live_entries = |n: usize| -> Vec<(String, String)> {
        (0..n)
            .map(|i| {
                let tags = if i == 0 {
                    key_tag(KEY_URL, None)
                } else {
                    String::new()
                };
                entry(tags, &format!("index{i}.ts"))
            })
            .collect()
    };
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.route(
        TS_PLAYLIST_URL,
        vec![
            Reply::Ok(playlist(0, &live_entries(2), false)),
            Reply::Ok(playlist(0, &live_entries(3), false)),
        ],
    );
    for n in 0..3u64 {
        let idx = usize::try_from(n).unwrap();
        h.ok(
            &seg_url(idx),
            aes128_encrypt_segment(&KEY_A, &msn_iv(n), &plain_seg(idx)),
        );
    }
    h.ok(KEY_URL, KEY_A.to_vec());
    h.run_until(
        |evs| evs.iter().any(|e| matches!(e, Event::Sample { .. })),
        1000,
    );
    assert_eq!(h.hits(KEY_URL), 1);
    h.session.reconnect(h.now);
    for _ in 0..2000 {
        if h.hits(SEG2_URL) > 0 {
            break;
        }
        h.step();
    }
    assert_eq!(h.hits(SEG2_URL), 1, "segment after the reconnect fetched");
    h.run_until(|_| false, 50);
    assert_eq!(
        h.hits(KEY_URL),
        1,
        "key cache survives a session reconnect: no refetch"
    );
    assert!(errors(&h.events).is_empty());
    assert!(h.samples() > MIN_SAMPLES);
}

#[test]
fn chunked_key_body_1_byte_chunks_ok() {
    let mut h = encrypted_host(hls_config(), vec![good_key()]);
    h.chunk = 1;
    h.run_to_end();
    assert_eq!(data_events(&h.events), baseline());
    assert_eq!(h.hits(KEY_URL), 1);
    assert!(errors(&h.events).is_empty());
}

#[test]
fn late_key_callbacks_after_close_ignored() {
    let iv = iv_seq(0x40);
    let entries = vec![entry(key_tag(KEY_URL, Some(&iv)), "index0.ts")];
    let mut s = Session::open(TS_PLAYLIST_URL, hls_config(), 0).expect("open");
    let first = drain_actions(&mut s, 0);
    let (playlist_conn, _) = http_requests(&first)[0].clone();
    s.on_response(playlist_conn, 200, &[]);
    s.on_data(playlist_conn, &playlist(0, &entries, true), 0);
    s.on_closed(playlist_conn, None, 0);
    let actions = drain_actions(&mut s, 0);
    let reqs = http_requests(&actions);
    let key_conn = reqs
        .iter()
        .find(|(_, url)| url == KEY_URL)
        .map(|(c, _)| *c)
        .expect("key request issued");
    let _ = drain_events(&mut s);
    s.close();
    let closing = drain_actions(&mut s, 1);
    assert!(
        closing
            .iter()
            .any(|a| matches!(a, Action::Close { conn } if *conn == key_conn))
    );
    s.on_response(key_conn, 200, &[]);
    s.on_data(key_conn, &KEY_A, 2);
    s.on_closed(key_conn, None, 2);
    assert!(s.next_event().is_none());
    assert!(s.poll(3).is_none());
}

#[test]
fn plaintext_playlist_never_requests_a_key() {
    let mut h = plain_host(hls_config());
    h.run_to_end();
    let urls: Vec<&str> = h.seen.iter().map(|s| s.url.as_str()).collect();
    assert_eq!(urls.len(), 3, "{urls:?}");
    assert!(urls.iter().all(|u| !u.contains("key")));
    assert!(errors(&h.events).is_empty());
}

#[test]
fn fatal_error_stops_session_no_more_requests() {
    let iv = iv_seq(0x60);
    let entries = vec![
        entry(key_tag(KEY_URL, Some(&iv)), "index0.ts"),
        entry(String::new(), "index1.ts"),
    ];
    let config = Config {
        max_key_failures: 1,
        ..hls_config()
    };
    let mut s = Session::open(TS_PLAYLIST_URL, config, 0).expect("open");
    let first = drain_actions(&mut s, 0);
    let (pconn, _) = http_requests(&first)[0].clone();
    s.on_response(pconn, 200, &[]);
    s.on_data(pconn, &playlist(0, &entries, true), 0);
    s.on_closed(pconn, None, 0);
    let reqs = http_requests(&drain_actions(&mut s, 0));
    let key_conn = reqs.iter().find(|(_, u)| u == KEY_URL).expect("key").0;
    let seg_conns: Vec<u32> = reqs
        .iter()
        .filter(|(_, u)| u != KEY_URL)
        .map(|(c, _)| *c)
        .collect();
    assert_eq!(seg_conns.len(), 2);
    let _ = drain_events(&mut s);

    s.on_response(key_conn, 200, &[]);
    s.on_data(key_conn, &KEY_A[..9], 0);
    s.on_closed(key_conn, None, 0);
    let events = drain_events(&mut s);
    assert_eq!(errors(&events), vec![(ErrorKind::BadKey, true)]);
    let actions = drain_actions(&mut s, 0);
    for c in &seg_conns {
        assert!(
            actions
                .iter()
                .any(|a| matches!(a, Action::Close { conn } if conn == c)),
            "open connection {c} closed on fatal: {actions:?}"
        );
    }
    assert!(http_requests(&actions).is_empty());

    for c in &seg_conns {
        s.on_response(*c, 200, &[]);
        s.on_data(*c, &plain_seg(0), 1);
        s.on_closed(*c, None, 1);
    }
    s.set_credentials(Credentials::bearer(BEARER)).unwrap();
    s.reconnect(2);
    let mut t = 0u64;
    for _ in 0..50 {
        t += 30 * SEC;
        assert!(s.poll(t).is_none(), "terminal session issues nothing");
    }
    assert!(
        drain_events(&mut s).is_empty(),
        "terminal session is silent"
    );
}

#[test]
fn non_fatal_error_emitted_once_not_per_retry() {
    let mut h = encrypted_host(
        hls_config(),
        vec![
            Reply::Status(500),
            Reply::Status(500),
            Reply::Status(500),
            good_key(),
        ],
    );
    h.run_to_end();
    assert_eq!(h.hits(KEY_URL), 4);
    assert_eq!(errors(&h.events), vec![(key_failed(Some(500)), false)]);
    let attempts = statuses(&h.events)
        .into_iter()
        .filter(|s| matches!(s, Status::Reconnecting { .. }))
        .count();
    assert_eq!(attempts, 3);
    assert!(h.events.iter().any(is_ended));
}

#[test]
fn escalates_to_fatal_after_max_key_failures() {
    const MAX: u32 = 3;
    let config = Config {
        max_key_failures: MAX,
        ..hls_config()
    };
    let mut h = encrypted_host(config, vec![Reply::Status(404)]);
    run_until_fatal(&mut h);
    assert_eq!(h.hits(KEY_URL), usize::try_from(MAX).unwrap());
    assert_eq!(
        errors(&h.events),
        vec![
            (key_failed(Some(404)), false),
            (key_failed(Some(404)), true)
        ]
    );
    let seen = h.seen.len();
    h.run_until(|_| false, 200);
    assert_eq!(h.seen.len(), seen, "no request after the fatal error");
    assert!(!h.events.iter().any(is_ended));
}

#[test]
fn auth_statuses_on_key_requests_are_not_error_events() {
    let mut h = encrypted_host(hls_config(), vec![Reply::Status(403), good_key()]);
    h.run_to_end();
    assert!(
        h.events
            .iter()
            .any(|e| matches!(e, Event::Forbidden { status: 403 }))
    );
    assert!(errors(&h.events).is_empty(), "{:?}", errors(&h.events));
    assert!(h.events.iter().any(is_ended));

    let challenge = vec![(
        "WWW-Authenticate".to_string(),
        "Bearer realm=\"k\"".to_string(),
    )];
    let mut h = encrypted_host(
        hls_config(),
        vec![Reply::StatusHeaders(401, challenge), good_key()],
    );
    h.session
        .set_credentials(Credentials::bearer(BEARER))
        .unwrap();
    h.run_until(
        |evs| evs.iter().any(|e| matches!(e, Event::AuthRejected { .. })),
        1000,
    );
    assert!(
        h.events
            .iter()
            .any(|e| matches!(e, Event::AuthRejected { status: 401 }))
    );
    assert!(errors(&h.events).is_empty(), "{:?}", errors(&h.events));
}

#[test]
fn error_kind_names_are_stable() {
    let cases = [
        (
            ErrorKind::UnsupportedEncryption {
                method: String::new(),
            },
            "unsupported_encryption",
        ),
        (
            ErrorKind::KeyFetchFailed { status: None },
            "key_fetch_failed",
        ),
        (ErrorKind::BadKey, "bad_key"),
        (ErrorKind::DecryptFailed, "decrypt_failed"),
        (
            ErrorKind::UnsupportedCodec {
                fourcc: String::new(),
            },
            "unsupported_codec",
        ),
    ];
    for (kind, name) in cases {
        assert_eq!(kind.name(), name);
        assert_eq!(kind.to_string(), name);
    }
}
