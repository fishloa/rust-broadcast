use crate::support;

use support::hls_host::{Host, Reply, TS_PLAYLIST_URL, hls_config, ts_playlist, ts_segment};

const CDN_PLAYLIST: &str = "http://cdn.test/live/index.m3u8";
const CDN_SEG0: &str = "http://cdn.test/live/index0.ts";
const CDN_SEG1: &str = "http://cdn.test/live/index1.ts";
const ORIGIN_SEG0: &str = "http://origin.test/hls/index0.ts";

#[test]
fn a_playlist_redirected_to_another_origin_resolves_relative_segments_there() {
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.route(
        TS_PLAYLIST_URL,
        vec![Reply::Redirect(CDN_PLAYLIST.to_string())],
    );
    h.ok(CDN_PLAYLIST, ts_playlist());
    h.ok(CDN_SEG0, ts_segment(0));
    h.ok(CDN_SEG1, ts_segment(1));
    h.run_to_end();
    assert!(
        h.samples() > 0,
        "nothing played: requests {:?}",
        h.seen.iter().map(|s| s.url.as_str()).collect::<Vec<_>>()
    );
    assert_eq!(
        h.hits(ORIGIN_SEG0),
        0,
        "segments must not resolve against the pre-redirect URL"
    );
    assert!(h.hits(CDN_SEG0) >= 1 && h.hits(CDN_SEG1) >= 1);
}

#[test]
fn credentials_stay_scoped_to_the_playlist_origin_after_a_redirect() {
    let mut h = Host::new(TS_PLAYLIST_URL, hls_config());
    h.session
        .set_credentials(broadcast_client::Credentials::bearer("secret"))
        .unwrap();
    h.route(
        TS_PLAYLIST_URL,
        vec![Reply::Redirect(CDN_PLAYLIST.to_string())],
    );
    h.ok(CDN_PLAYLIST, ts_playlist());
    h.ok(CDN_SEG0, ts_segment(0));
    h.ok(CDN_SEG1, ts_segment(1));
    h.run_to_end();
    assert!(h.samples() > 0);
    for req in h
        .seen
        .iter()
        .filter(|s| s.url.starts_with("http://cdn.test"))
    {
        assert!(req.authorization().is_none(), "{req:?}");
    }
    assert!(
        h.requests_to(TS_PLAYLIST_URL)[0].authorization().is_some(),
        "the origin request carries the credentials"
    );
}

const LONG_WAIT_US: u64 = 40_000_000;

#[test]
fn a_redirect_answered_after_a_long_wait_gets_a_fresh_deadline() {
    use broadcast_client::{Action, Config, Event, Session};
    let mut session = Session::open(TS_PLAYLIST_URL, hls_config(), 0).unwrap();
    let mut conn = None;
    while let Some(a) = session.poll(0) {
        if let Action::HttpRequest { conn: c, .. } = a {
            conn = Some(c);
        }
    }
    let conn = conn.expect("playlist request");
    session.on_response(
        conn,
        302,
        &[("Location".to_string(), CDN_PLAYLIST.to_string())],
        LONG_WAIT_US,
    );
    let later = LONG_WAIT_US + 1;
    let mut closes = Vec::new();
    let mut requests = Vec::new();
    while let Some(a) = session.poll(later) {
        match a {
            Action::Close { conn } => closes.push(conn),
            Action::HttpRequest { conn, url, .. } => requests.push((conn, url)),
            _ => {}
        }
    }
    let replay = requests
        .iter()
        .find(|(_, url)| url == CDN_PLAYLIST)
        .map(|(c, _)| *c)
        .expect("the redirect target is requested");
    assert!(
        !closes.contains(&replay),
        "the replayed request was expired by a stale start time: closes {closes:?}"
    );
    let mut events = Vec::new();
    while let Some(e) = session.next_event() {
        events.push(e);
    }
    assert!(!events.iter().any(|e| matches!(
        e,
        Event::Status(broadcast_client::Status::Reconnecting { .. })
    )));
    let _ = Config::default();
}
