//! Source selection for DASH: `SourceKind::Dash`, `Auto` for a `.mpd` path (ASCII
//! case-insensitive), and what the other kinds do with the same URL.

use broadcast_client::{Action, Config, Error, Session, SourceKind};

fn first_request(url: &str, kind: SourceKind) -> Option<(String, String)> {
    let mut config = Config::default();
    config.kind = kind;
    let mut session = Session::open(url, config, 0).ok()?;
    std::iter::from_fn(|| session.poll(0)).find_map(|a| match a {
        Action::HttpRequest { method, url, .. } => Some((method, url)),
        _ => None,
    })
}

fn is_adaptive(url: &str, kind: SourceKind) -> bool {
    let mut config = Config::default();
    config.kind = kind;
    let mut session = Session::open(url, config, 0).unwrap();
    !matches!(session.select_variant(0), Err(Error::NotHls))
}

#[test]
fn auto_picks_dash_for_an_mpd_path_in_any_case_with_or_without_a_query() {
    for url in [
        "http://h/live/manifest.mpd",
        "http://h/live/MANIFEST.MPD",
        "http://h/live/manifest.Mpd?token=a.b",
        "http://h/a.b/c.d/manifest.mpd#frag",
    ] {
        assert!(is_adaptive(url, SourceKind::Auto), "{url}");
        let (method, _) = first_request(url, SourceKind::Auto).unwrap();
        assert_eq!(method, "GET");
    }
}

#[test]
fn auto_leaves_other_urls_to_their_own_sources() {
    for url in [
        "http://h/live.ts",
        "http://h/live",
        "http://h/live.mpd.ts",
        "http://h/mpd",
        "http://h/live.mpdx",
    ] {
        assert!(!is_adaptive(url, SourceKind::Auto), "{url}");
    }
    assert!(is_adaptive("http://h/live.m3u8", SourceKind::Auto));
}

#[test]
fn an_explicit_kind_wins_over_the_extension() {
    assert!(is_adaptive("http://h/stream", SourceKind::Dash));
    assert!(!is_adaptive("http://h/stream.mpd", SourceKind::TsHttp));
    let hls = first_request("http://h/stream.mpd", SourceKind::Hls).unwrap();
    assert_eq!(hls.1, "http://h/stream.mpd", "HLS fetches whatever it is given");
}

#[test]
fn dash_is_http_only() {
    let mut config = Config::default();
    config.kind = SourceKind::Dash;
    assert!(matches!(
        Session::open("rtsp://h/stream", config, 0),
        Err(Error::UnsupportedScheme(_))
    ));
    let mut config = Config::default();
    config.kind = SourceKind::Auto;
    assert!(Session::open("rtsp://h/stream.mpd", config, 0).is_ok());
}

#[test]
fn the_kind_has_a_stable_label() {
    assert_eq!(SourceKind::Dash.name(), "dash");
    assert_eq!(SourceKind::Dash.to_string(), "dash");
}

#[test]
fn a_dash_session_asks_for_the_mpd_first_with_get_and_no_other_request() {
    let mut session = Session::open("http://h/a/manifest.mpd", Config::default(), 0).unwrap();
    let requests: Vec<(String, String)> = std::iter::from_fn(|| session.poll(0))
        .filter_map(|a| match a {
            Action::HttpRequest { method, url, .. } => Some((method, url)),
            _ => None,
        })
        .collect();
    assert_eq!(
        requests,
        vec![("GET".to_owned(), "http://h/a/manifest.mpd".to_owned())]
    );
}
