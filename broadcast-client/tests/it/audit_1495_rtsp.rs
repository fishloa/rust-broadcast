use crate::support;

use std::sync::OnceLock;

use broadcast_client::Config;
use sdp_types::Attribute;
use support::rtsp_host::{
    BASE_URL, Host, MediaFixture, OPTIONS, PLAY, SEC, SETUP, Server, TEARDOWN, quiet_config,
};

const AGGREGATE: &str = "rtsp://cam.test/live/agg";
const KEEPALIVE_SECS: u64 = 30;
const RTX_PT: &str = "97";

fn fixture() -> &'static MediaFixture {
    static FIXTURE: OnceLock<MediaFixture> = OnceLock::new();
    FIXTURE.get_or_init(MediaFixture::load)
}

fn fixture_session() -> sdp_types::Session {
    sdp_types::Session::parse(fixture().sdp.as_bytes()).expect("parse fixture sdp")
}

fn write_sdp(session: &sdp_types::Session) -> String {
    let mut out = Vec::new();
    session.write(&mut out).expect("write sdp");
    String::from_utf8(out).expect("utf8")
}

fn with_session_control(control: &str) -> String {
    let mut session = fixture_session();
    session.attributes.push(Attribute {
        attribute: "control".to_string(),
        value: Some(control.to_string()),
    });
    write_sdp(&session)
}

fn uris(host: &Host, method: &str) -> Vec<String> {
    host.server
        .log
        .iter()
        .filter(|r| r.method == method)
        .map(|r| r.uri.clone())
        .collect()
}

fn connected(sdp: &str, config: Config) -> Host {
    let mut host = Host::new(BASE_URL, config, Server::new(sdp));
    host.connect();
    host
}

#[test]
fn a_session_level_control_is_the_aggregate_url_for_play_options_and_teardown() {
    let mut host = connected(&with_session_control(AGGREGATE), quiet_config());
    assert_eq!(uris(&host, PLAY), [AGGREGATE]);
    host.advance_to(KEEPALIVE_SECS * SEC);
    assert_eq!(uris(&host, OPTIONS), [AGGREGATE]);
    host.session.close();
    host.drain();
    let teardown = host.sent_methods().iter().any(|m| m == TEARDOWN);
    assert!(teardown, "{:?}", host.sent_methods());
    assert_eq!(uris(&host, TEARDOWN), [AGGREGATE]);
    assert_eq!(uris(&host, SETUP).len(), 2);
}

#[test]
fn a_relative_star_or_foreign_session_level_control_resolves_against_the_base() {
    let relative_agg = format!("{BASE_URL}agg/");
    let cases: [(&str, &str); 3] = [
        ("agg/", &relative_agg),
        ("*", BASE_URL),
        ("rtsp://other.test/live/agg", BASE_URL),
    ];
    for (control, want) in cases {
        let host = connected(&with_session_control(control), quiet_config());
        assert_eq!(uris(&host, PLAY), [want], "{control}");
    }
}

#[test]
fn without_a_session_level_control_play_uses_the_request_url() {
    let host = connected(&fixture().sdp, quiet_config());
    assert_eq!(uris(&host, PLAY), [BASE_URL]);
}

fn with_rtx_listed_first() -> (String, usize) {
    let mut session = fixture_session();
    let mut rewritten = 0;
    for media in session.medias.iter_mut().filter(|m| m.media == "video") {
        let pt = media.fmt.clone();
        media.fmt = format!("{RTX_PT} {pt}");
        media.attributes.insert(
            0,
            Attribute {
                attribute: "fmtp".to_string(),
                value: Some(format!("{RTX_PT} apt={pt}")),
            },
        );
        media.attributes.insert(
            0,
            Attribute {
                attribute: "rtpmap".to_string(),
                value: Some(format!("{RTX_PT} rtx/90000")),
            },
        );
        rewritten += 1;
    }
    (write_sdp(&session), rewritten)
}

#[test]
fn an_rtx_payload_type_listed_first_does_not_hide_the_playable_one() {
    let (sdp, rewritten) = with_rtx_listed_first();
    assert_eq!(rewritten, 1);
    assert!(
        sdp.contains(&format!("m=video 0 RTP/AVP {RTX_PT} ")),
        "{sdp}"
    );
    let host = connected(&sdp, quiet_config());
    assert_eq!(
        uris(&host, SETUP).len(),
        2,
        "video and audio are both set up"
    );
    assert_eq!(uris(&host, PLAY).len(), 1);
}

#[test]
fn the_rtpmap_and_fmtp_of_the_listed_payload_type_decide_support() {
    let mut session = fixture_session();
    for media in session.medias.iter_mut().filter(|m| m.media == "video") {
        let pt = media.fmt.clone();
        media.fmt = format!("{pt} {RTX_PT}");
        media.attributes.push(Attribute {
            attribute: "rtpmap".to_string(),
            value: Some(format!("{RTX_PT} H265/90000")),
        });
        media.attributes.push(Attribute {
            attribute: "fmtp".to_string(),
            value: Some(format!("{RTX_PT} garbage")),
        });
    }
    let host = connected(&write_sdp(&session), quiet_config());
    assert_eq!(
        uris(&host, SETUP).len(),
        2,
        "the first supported payload type wins and later attributes do not interfere"
    );
}

#[test]
fn a_media_line_listing_only_unsupported_payload_types_is_skipped() {
    let mut session = fixture_session();
    for media in session.medias.iter_mut().filter(|m| m.media == "video") {
        let pt = media.fmt.clone();
        media.fmt = RTX_PT.to_string();
        media.attributes.retain(|a| {
            !(a.attribute == "rtpmap" || a.attribute == "fmtp")
                || a.value.as_deref().is_some_and(|v| !v.starts_with(&pt))
        });
        media.attributes.push(Attribute {
            attribute: "rtpmap".to_string(),
            value: Some(format!("{RTX_PT} rtx/90000")),
        });
        media.attributes.push(Attribute {
            attribute: "fmtp".to_string(),
            value: Some(format!("{RTX_PT} apt={pt}")),
        });
    }
    let host = connected(&write_sdp(&session), quiet_config());
    assert_eq!(uris(&host, SETUP).len(), 1, "only the audio plays");
}
