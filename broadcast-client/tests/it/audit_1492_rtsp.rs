use crate::support;

use std::collections::BTreeSet;
use std::sync::OnceLock;

use broadcast_auth::{Credentials as AuthCredentials, Verifier};
use broadcast_client::{Config, Credentials, Event, Status};
use sdp_types::{Media, MediaType, TransportProto};
use support::rtsp_host::{
    BASE_URL, Host, MediaFixture, OPTIONS, PLAY, SEC, SETUP, Server, SetupReply, connects,
    interleaved, quiet_config, samples_of,
};
use transmux::rtp::RtpMediaKind;

const MS: u64 = 1_000;
const NO_SLASH_URL: &str = "rtsp://cam.test/axis-media/media.amp";
const USER: &str = "admin";
const PASSWORD: &str = "s3cret";
const MAX_TRACKS: usize = 16;
const MANY_SECTIONS: usize = 200;
const KEEPALIVE_CYCLES: u64 = 5;
const KEEPALIVE_SECS: u64 = 30;

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

fn set_control(media: &mut Media, value: &str) {
    media.attributes.retain(|a| a.attribute != "control");
    media.add_attribute_with_value("control", value.to_string());
}

fn sdp_with_controls(controls: &[&str]) -> String {
    let mut session = fixture_session();
    for (media, control) in session.medias.iter_mut().zip(controls) {
        set_control(media, control);
    }
    write_sdp(&session)
}

fn video_index() -> usize {
    fixture()
        .tracks
        .iter()
        .position(|t| t.kind == RtpMediaKind::H264)
        .expect("fixture has video")
}

fn connected(url: &str, config: Config, server: Server) -> Host {
    let mut host = Host::new(url, config, server);
    host.connect();
    host
}

fn setup_uris(host: &Host) -> Vec<String> {
    host.server
        .log
        .iter()
        .filter(|r| r.method == SETUP)
        .map(|r| r.uri.clone())
        .collect()
}

fn reconnecting_attempts(events: &[Event]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Status(Status::Reconnecting { attempt }) => Some(*attempt),
            _ => None,
        })
        .collect()
}

#[test]
fn relative_control_resolves_against_the_request_url_with_a_trailing_slash_or_the_content_base() {
    let host = connected(NO_SLASH_URL, Config::default(), Server::new(&fixture().sdp));
    assert_eq!(
        setup_uris(&host),
        vec![
            format!("{NO_SLASH_URL}/trackID=1"),
            format!("{NO_SLASH_URL}/trackID=2")
        ]
    );
    assert_eq!(host.server.count(PLAY), 1);

    let mut server = Server::new(&fixture().sdp);
    server.content_base = Some("rtsp://cam.test/base/path/".to_string());
    let host = connected(NO_SLASH_URL, Config::default(), server);
    assert_eq!(
        setup_uris(&host),
        vec![
            "rtsp://cam.test/base/path/trackID=1".to_string(),
            "rtsp://cam.test/base/path/trackID=2".to_string()
        ]
    );
    assert_eq!(host.server.count(PLAY), 1);
}

#[test]
fn absolute_control_on_the_same_host_is_used_verbatim() {
    let sdp = sdp_with_controls(&[
        "rtsp://cam.test/live/stream-a",
        "rtsp://cam.test:554/live/stream-b",
    ]);
    let host = connected(NO_SLASH_URL, Config::default(), Server::new(&sdp));
    assert_eq!(
        setup_uris(&host),
        vec![
            "rtsp://cam.test/live/stream-a".to_string(),
            "rtsp://cam.test:554/live/stream-b".to_string()
        ]
    );
}

#[test]
fn star_control_is_the_base_itself() {
    let sdp = sdp_with_controls(&["*", "*"]);
    let host = connected(NO_SLASH_URL, Config::default(), Server::new(&sdp));
    assert_eq!(
        setup_uris(&host),
        vec![format!("{NO_SLASH_URL}/"), format!("{NO_SLASH_URL}/")]
    );

    let mut server = Server::new(&sdp);
    server.content_base = Some("rtsp://cam.test/base/path/".to_string());
    let host = connected(NO_SLASH_URL, Config::default(), server);
    assert_eq!(
        setup_uris(&host),
        vec![
            "rtsp://cam.test/base/path/".to_string(),
            "rtsp://cam.test/base/path/".to_string()
        ]
    );
}

#[test]
fn an_absolute_control_for_another_authority_skips_that_section() {
    for foreign in [
        "rtsp://other.test/live/trackID=2",
        "rtsp://cam.test:8554/live/trackID=2",
        "rtsps://cam.test/live/trackID=2",
    ] {
        let video = video_index();
        let mut controls = ["trackID=1", "trackID=2"];
        controls[1 - video] = foreign;
        controls[video] = "trackID=1";
        let sdp = sdp_with_controls(&controls);
        let host = connected(BASE_URL, Config::default(), Server::new(&sdp));
        let uris = setup_uris(&host);
        assert_eq!(uris, vec![format!("{BASE_URL}trackID=1")], "{foreign}");
        assert_eq!(host.server.count(PLAY), 1, "{foreign}");
    }
}

#[test]
fn unsupported_media_sections_are_skipped_and_the_rest_plays() {
    let mut session = fixture_session();
    let video = session.medias.remove(video_index());
    let mut pcmu = Media::new(MediaType::Audio, 0, TransportProto::RtpAvp, 0);
    pcmu.add_attribute_with_value("rtpmap", "0 PCMU/8000".to_string());
    set_control(&mut pcmu, "trackID=9");
    session.medias = vec![pcmu, video];
    let sdp = write_sdp(&session);
    let mut host = connected(BASE_URL, quiet_config(), Server::new(&sdp));
    assert_eq!(host.server.count(SETUP), 1, "{:?}", host.server.methods());
    assert_eq!(host.server.count(PLAY), 1);
    let video_track = &fixture().tracks[video_index()];
    let frames: Vec<Vec<u8>> = video_track
        .packets
        .iter()
        .map(|p| interleaved(0, p))
        .collect();
    host.deliver_all(&frames);
    let samples = samples_of(&host.events());
    assert!(!samples.is_empty(), "the video must play");
}

#[test]
fn only_unsupported_codecs_are_fatal_not_a_retry_loop() {
    let mut session = fixture_session();
    let mut pcmu = Media::new(MediaType::Audio, 0, TransportProto::RtpAvp, 0);
    pcmu.add_attribute_with_value("rtpmap", "0 PCMU/8000".to_string());
    let app = Media::new(MediaType::Application, 0, TransportProto::RtpAvp, 96);
    session.medias = vec![pcmu, app];
    let sdp = write_sdp(&session);
    let mut host = connected(BASE_URL, Config::default(), Server::new(&sdp));
    let events = host.events();
    assert!(reconnecting_attempts(&events).is_empty());
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Error {
            kind: broadcast_client::ErrorKind::UnsupportedCodec { fourcc },
            fatal: true,
        } if fourcc == "PCMU"
    )));
    assert_eq!(host.server.count(SETUP), 0);
}

#[test]
fn setup_reply_without_a_transport_header_is_a_failure() {
    let mut server = Server::new(&fixture().sdp);
    server.setup_reply = Some(Box::new(|nth| {
        if nth == 0 {
            SetupReply::Echo
        } else {
            SetupReply::NoTransport
        }
    }));
    let mut host = connected(BASE_URL, Config::default(), server);
    assert_eq!(reconnecting_attempts(&host.events()), [1]);
    assert_eq!(host.server.count(PLAY), 0);
}

#[test]
fn duplicate_interleaved_channels_are_a_failure() {
    let mut server = Server::new(&fixture().sdp);
    server.setup_reply = Some(Box::new(|_| SetupReply::Fixed(0, 1)));
    let mut host = connected(BASE_URL, Config::default(), server);
    assert_eq!(reconnecting_attempts(&host.events()), [1]);
    assert_eq!(host.server.count(PLAY), 0);
}

#[test]
fn overlapping_interleaved_channels_are_a_failure() {
    let mut server = Server::new(&fixture().sdp);
    server.setup_reply = Some(Box::new(|nth| {
        if nth == 0 {
            SetupReply::Fixed(0, 1)
        } else {
            SetupReply::Fixed(1, 2)
        }
    }));
    let mut host = connected(BASE_URL, Config::default(), server);
    assert_eq!(reconnecting_attempts(&host.events()), [1]);
    assert_eq!(host.server.count(PLAY), 0);
}

#[test]
fn media_sections_are_capped_and_channels_stay_distinct() {
    let mut session = fixture_session();
    let template = session.medias[video_index()].clone();
    session.medias = (0..MANY_SECTIONS)
        .map(|n| {
            let mut m = template.clone();
            set_control(&mut m, &format!("trackID={}", n + 1));
            m
        })
        .collect();
    let sdp = write_sdp(&session);
    let host = connected(BASE_URL, Config::default(), Server::new(&sdp));
    assert_eq!(host.server.count(SETUP), MAX_TRACKS);
    let channels: BTreeSet<u8> = host
        .server
        .log
        .iter()
        .filter(|r| r.method == SETUP)
        .filter_map(|r| r.interleaved().map(|(rtp, _)| rtp))
        .collect();
    assert_eq!(channels.len(), MAX_TRACKS, "{channels:?}");
    assert_eq!(host.server.count(PLAY), 1);
}

fn digest_server() -> Server {
    let mut server = Server::new(&fixture().sdp);
    server.rotate_options_nonce = true;
    server
}

#[test]
fn keepalive_challenges_with_rotating_nonces_never_exhaust_the_replay_budget() {
    let mut host = Host::new(BASE_URL, quiet_config(), digest_server());
    host.session
        .set_credentials(Credentials::new(USER, PASSWORD))
        .expect("credentials");
    host.connect();
    for cycle in 1..=KEEPALIVE_CYCLES {
        host.advance_to(cycle * KEEPALIVE_SECS * SEC);
        assert_eq!(
            host.server.count(OPTIONS),
            usize::try_from(cycle * 2).expect("small"),
            "cycle {cycle}: each keepalive is challenged once and retried once"
        );
    }
    let events = host.events();
    assert!(
        !events.iter().any(|e| matches!(
            e,
            Event::AuthRejected { .. } | Event::Status(Status::WaitingForCredentials)
        )),
        "{events:?}"
    );
    assert_eq!(connects(&host.actions).len(), 1, "no reconnect");
}

#[test]
fn a_zero_backoff_entry_is_floored_at_one_millisecond() {
    let mut config = Config::default();
    config.ts_http.backoff_ms = vec![0, 0, 0];
    let mut server = Server::new(&fixture().sdp);
    server.describe_status = Some(500);
    let host = connected(BASE_URL, config, server);
    assert_eq!(
        connects(&host.actions).len(),
        1,
        "a zero backoff must not reconnect within the same instant"
    );
    let last_wake = support::rtsp_host::wakes(&host.actions).last().copied();
    assert_eq!(last_wake, Some(MS));
}

#[test]
fn a_connection_that_played_briefly_waits_out_the_minimum_reconnect_interval() {
    let mut host = connected(BASE_URL, quiet_config(), Server::new(&fixture().sdp));
    let frames = fixture().frames();
    host.deliver_all(&frames[..60]);
    assert!(!samples_of(&host.events()).is_empty());
    let played_for = 100 * MS;
    host.now = played_for;
    let conn = host.conn.expect("live connection");
    host.session.on_closed(conn, None, host.now);
    host.drain();
    assert_eq!(connects(&host.actions).len(), 1, "reconnect is not instant");
    host.advance_to(999 * MS);
    assert_eq!(connects(&host.actions).len(), 1);
    host.advance_to(1_000 * MS);
    assert_eq!(connects(&host.actions).len(), 2);
}

#[test]
fn userinfo_in_an_rtsp_url_authenticates_and_stays_out_of_request_uris() {
    let mut server = Server::new(&fixture().sdp);
    server.auth = Some(Verifier::new(
        AuthCredentials::Basic {
            username: USER.to_string(),
            password: PASSWORD.to_string(),
        },
        "cam",
    ));
    let host = connected(
        "rtsp://admin:s3cret@cam.test/live/",
        Config::default(),
        server,
    );
    assert_eq!(host.server.count(PLAY), 1, "{:?}", host.server.methods());
    assert!(
        host.server
            .log
            .iter()
            .all(|r| r.uri.starts_with("rtsp://cam.test/live/")),
        "{:?}",
        host.server.log.iter().map(|r| &r.uri).collect::<Vec<_>>()
    );
}
