mod support;

use std::sync::OnceLock;

use broadcast_auth::{Credentials as AuthCredentials, Verifier};
use broadcast_client::{Action, Config, Credentials, Event, Status};
use support::rtsp_host::{
    BASE_URL, DESCRIBE, Host, MediaFixture, SEC, Server, connects, quiet_config, samples_of,
};

const REDIRECT_TARGET: &str = "rtsp://other.test:8554/live2/";
const TLS_TARGET: &str = "rtsps://other.test/live2/";
const HTTP_TARGET: &str = "http://other.test/live2/";
const SOME_FRAMES: usize = 60;
const REALM: &str = "cam";
const USER: &str = "admin";
const PASSWORD: &str = "s3cret";
const MIN_RECONNECT: u64 = SEC;

fn fixture() -> &'static MediaFixture {
    static FIXTURE: OnceLock<MediaFixture> = OnceLock::new();
    FIXTURE.get_or_init(MediaFixture::load)
}

fn redirecting(status: u16, location: &str) -> Server {
    let mut server = Server::new(&fixture().sdp);
    server.describe_status = Some(status);
    server.describe_location = Some(location.to_string());
    server
}

fn connect_targets(host: &Host) -> Vec<(String, u16, bool)> {
    host.actions
        .iter()
        .filter_map(|a| match a {
            Action::TcpConnect {
                host, port, tls, ..
            } => Some((host.clone(), *port, *tls)),
            _ => None,
        })
        .collect()
}

fn serve_normally(host: &mut Host) {
    host.server.describe_status = None;
    host.server.describe_location = None;
}

fn finish_connect(host: &mut Host) {
    let conn = host.conn.expect("a TcpConnect was requested");
    host.session.on_connected(conn);
    host.drain();
}

fn describes(host: &Host) -> Vec<String> {
    host.server
        .log
        .iter()
        .filter(|r| r.method == DESCRIBE)
        .map(|r| r.uri.clone())
        .collect()
}

fn discontinuities(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, Event::Discontinuity))
        .count()
}

fn reconnecting(events: &[Event]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Status(Status::Reconnecting { attempt }) => Some(*attempt),
            _ => None,
        })
        .collect()
}

#[test]
fn a_describe_redirect_is_followed_to_the_new_authority() {
    for code in [301u16, 302, 303, 307, 308] {
        let mut host = Host::new(
            BASE_URL,
            Config::default(),
            redirecting(code, REDIRECT_TARGET),
        );
        host.connect();
        assert_eq!(
            connect_targets(&host),
            vec![
                ("cam.test".to_string(), 554, false),
                ("other.test".to_string(), 8554, false)
            ],
            "code {code}"
        );
        serve_normally(&mut host);
        finish_connect(&mut host);
        host.deliver_all(&fixture().frames()[..SOME_FRAMES]);
        let events = host.events();
        assert!(!samples_of(&events).is_empty(), "code {code}: no media");
        assert_eq!(
            describes(&host),
            vec![BASE_URL.to_string(), REDIRECT_TARGET.to_string()],
            "code {code}"
        );
        assert!(reconnecting(&host.history).is_empty(), "code {code}");
        assert_eq!(
            discontinuities(&host.history),
            0,
            "a connection that never produced media is not a discontinuity"
        );
    }
}

#[test]
fn redirects_are_bounded_by_max_redirects() {
    let config = {
        let mut new_config = Config::default();
        new_config.max_redirects = 2;
        new_config
    };
    let mut host = Host::new(BASE_URL, config, redirecting(302, REDIRECT_TARGET));
    host.connect();
    for _ in 0..8 {
        if host.conn.is_none() {
            break;
        }
        finish_connect(&mut host);
    }
    host.events();
    assert_eq!(
        describes(&host).len(),
        3,
        "the original request plus two followed redirects: {:?}",
        describes(&host)
    );
    assert_eq!(connects(&host.actions).len(), 3);
    assert_eq!(reconnecting(&host.history), vec![1]);
}

#[test]
fn a_redirect_to_a_scheme_that_downgrades_or_leaves_rtsp_is_not_followed() {
    let mut host = Host::new(BASE_URL, Config::default(), redirecting(302, HTTP_TARGET));
    host.connect();
    host.events();
    assert_eq!(connects(&host.actions).len(), 1);
    assert_eq!(reconnecting(&host.history), vec![1]);
    let mut tls = Host::new(
        "rtsps://cam.test/live/",
        Config::default(),
        redirecting(302, "rtsp://other.test/live2/"),
    );
    tls.connect();
    assert_eq!(
        connects(&tls.actions).len(),
        1,
        "rtsps must not fall back to rtsp"
    );
}

#[test]
fn a_redirect_may_upgrade_to_rtsps() {
    let mut host = Host::new(BASE_URL, Config::default(), redirecting(302, TLS_TARGET));
    host.connect();
    assert_eq!(
        connect_targets(&host).last(),
        Some(&("other.test".to_string(), 322, true))
    );
}

#[test]
fn credentials_are_dropped_when_the_redirect_changes_host() {
    let verifier = Verifier::new(
        AuthCredentials::Basic {
            username: USER.to_string(),
            password: PASSWORD.to_string(),
        },
        REALM,
    );
    let mut server = redirecting(302, REDIRECT_TARGET);
    server.auth = Some(verifier);
    let mut host = Host::new(BASE_URL, Config::default(), server);
    host.session
        .set_credentials(Credentials::new(USER, PASSWORD))
        .unwrap();
    host.connect();
    let first = host
        .server
        .log
        .iter()
        .filter(|r| r.method == DESCRIBE)
        .any(|r| r.header("authorization").is_some());
    assert!(first, "the original authority is sent the credentials");
    host.server.auth = None;
    serve_normally(&mut host);
    finish_connect(&mut host);
    let redirected = host
        .server
        .log
        .iter()
        .rfind(|r| r.method == DESCRIBE)
        .expect("a DESCRIBE after the redirect");
    assert_eq!(redirected.uri, REDIRECT_TARGET);
    assert!(
        redirected.header("authorization").is_none(),
        "credentials must not follow a redirect to another host: {redirected:?}"
    );
}

#[test]
fn a_same_host_redirect_keeps_the_credentials() {
    let verifier = Verifier::new(
        AuthCredentials::Basic {
            username: USER.to_string(),
            password: PASSWORD.to_string(),
        },
        REALM,
    );
    let mut server = redirecting(302, "rtsp://cam.test/live2/");
    server.auth = Some(verifier);
    let mut host = Host::new(BASE_URL, Config::default(), server);
    host.session
        .set_credentials(Credentials::new(USER, PASSWORD))
        .unwrap();
    host.connect();
    serve_normally(&mut host);
    finish_connect(&mut host);
    let redirected = host
        .server
        .log
        .iter()
        .rfind(|r| r.method == DESCRIBE)
        .unwrap();
    assert_eq!(redirected.uri, "rtsp://cam.test/live2/");
    assert!(redirected.header("authorization").is_some());
}

fn redirect_keeps_credentials(start: &str, target: &str) {
    let verifier = Verifier::new(
        AuthCredentials::Basic {
            username: USER.to_string(),
            password: PASSWORD.to_string(),
        },
        REALM,
    );
    let mut server = redirecting(302, target);
    server.auth = Some(verifier);
    let mut host = Host::new(start, Config::default(), server);
    host.session
        .set_credentials(Credentials::new(USER, PASSWORD))
        .unwrap();
    host.connect();
    serve_normally(&mut host);
    finish_connect(&mut host);
    let redirected = host
        .server
        .log
        .iter()
        .rfind(|r| r.method == DESCRIBE)
        .unwrap();
    assert_eq!(redirected.uri, target);
    assert!(
        redirected.header("authorization").is_some(),
        "{start} -> {target} is the same authority, credentials must follow"
    );
}

#[test]
fn a_redirect_between_implicit_and_explicit_default_port_keeps_the_credentials() {
    redirect_keeps_credentials("rtsp://cam.test/live/", "rtsp://cam.test:554/live2/");
    redirect_keeps_credentials("rtsp://cam.test:554/live/", "rtsp://cam.test/live2/");
    redirect_keeps_credentials("rtsps://cam.test/live/", "rtsps://cam.test:322/live2/");
    redirect_keeps_credentials("rtsps://cam.test:322/live/", "rtsps://cam.test/live2/");
}

#[test]
fn a_connection_that_never_produced_media_is_not_a_discontinuity() {
    let mut server = Server::new(&fixture().sdp);
    server.describe_status = Some(503);
    let mut host = Host::new(BASE_URL, quiet_config(), server);
    host.connect();
    for round in 1..=3u64 {
        host.advance_to(host.now + 11 * SEC);
        finish_connect(&mut host);
        assert!(round > 0);
    }
    host.events();
    assert!(reconnecting(&host.history).len() >= 3, "{:?}", host.history);
    assert_eq!(discontinuities(&host.history), 0);
}

#[test]
fn the_discontinuity_follows_a_connection_that_played_and_precedes_the_first_new_sample() {
    let mut host = Host::new(BASE_URL, Config::default(), Server::new(&fixture().sdp));
    host.connect();
    host.deliver_all(&fixture().frames()[..SOME_FRAMES]);
    host.events();
    host.session.reconnect(host.now + MIN_RECONNECT);
    host.advance_to(host.now + MIN_RECONNECT);
    finish_connect(&mut host);
    host.deliver_all(&fixture().frames()[..SOME_FRAMES]);
    let events = host.events();
    let established = events
        .iter()
        .position(|e| matches!(e, Event::Status(Status::Establishing)))
        .expect("the new connection establishes");
    let d = events
        .iter()
        .position(|e| matches!(e, Event::Discontinuity))
        .expect("a Discontinuity");
    let first_new_sample = events
        .iter()
        .enumerate()
        .skip(established)
        .find_map(|(i, e)| matches!(e, Event::Sample { .. }).then_some(i))
        .expect("a sample on the new connection");
    assert!(
        established < d && d < first_new_sample,
        "{established} {d} {first_new_sample}"
    );
}

#[test]
fn a_played_connection_that_ends_reports_reconnecting_attempt_zero_then_live() {
    let mut host = Host::new(BASE_URL, Config::default(), Server::new(&fixture().sdp));
    host.connect();
    host.deliver_all(&fixture().frames()[..SOME_FRAMES]);
    host.events();
    let conn = host.conn.expect("live connection");
    host.session.on_closed(conn, None, host.now);
    host.drain();
    host.advance_to(host.now + MIN_RECONNECT);
    finish_connect(&mut host);
    host.deliver_all(&fixture().frames()[..SOME_FRAMES]);
    let events = host.events();
    assert_eq!(reconnecting(&events), vec![0]);
    let statuses: Vec<Status> = events
        .iter()
        .filter_map(|e| match e {
            Event::Status(s) => Some(*s),
            _ => None,
        })
        .collect();
    assert_eq!(
        statuses,
        [
            Status::Reconnecting { attempt: 0 },
            Status::Establishing,
            Status::Live
        ]
    );
}
