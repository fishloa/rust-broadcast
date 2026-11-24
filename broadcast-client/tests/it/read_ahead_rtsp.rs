//! Read-ahead gate for RTSP over interleaved TCP (issue #4086): the session asks
//! the host to stop reading the connection once the media already delivered runs
//! the configured cap ahead of the playback position, and to resume at half the
//! cap. A deliberate pause is not a stall, and the default,
//! [`ReadAhead::Unlimited`], emits neither action.

use crate::support::read_ahead_pace::{Media, clock};
use crate::support::rtsp_host::{BASE_URL, Host, MediaFixture, Server, quiet_config, samples_of};
use broadcast_client::{Action, Config, ConnId, ReadAhead};
use broadcast_common::clock33::PCR_HZ;
use broadcast_common::stage::Timestamp;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

const FRAME_STEP_US: u64 = 10_000;

fn seconds(n: u64) -> u64 {
    n * PCR_HZ
}

fn config(read_ahead: ReadAhead) -> Config {
    let mut config = quiet_config();
    config.ts_http = config.ts_http.with_read_ahead(read_ahead);
    config
}

fn fixture() -> MediaFixture {
    MediaFixture::load()
}

/// Deliver interleaved frames until `stop` says so, reporting the player's
/// position from `pace` after each one, and return the media delivered.
fn play_until(
    host: &mut Host,
    frames: &[Vec<u8>],
    mut pace: impl FnMut(&Media) -> u64,
    mut stop: impl FnMut(&Host) -> bool,
) -> Media {
    let mut media = Media::default();
    for frame in frames {
        host.deliver(frame);
        host.drain();
        for event in host.events() {
            media.observe(&event);
        }
        let position = pace(&media);
        host.session
            .on_playback_position(ts(host.now), Some(clock(position)));
        host.drain();
        for event in host.events() {
            media.observe(&event);
        }
        if stop(host) {
            break;
        }
        host.now = host.now.saturating_add(FRAME_STEP_US);
        host.advance_to(host.now);
    }
    media
}

fn connect(read_ahead: ReadAhead) -> (Host, ConnId) {
    let mut host = Host::new(BASE_URL, config(read_ahead), Server::new(&fixture().sdp));
    host.connect();
    let conn = host.conn.expect("the RTSP session connects");
    (host, conn)
}

#[test]
fn a_fast_rtsp_source_is_paused_at_the_time_cap() {
    let cap = seconds(1);
    let (mut host, conn) = connect(ReadAhead::time(core::time::Duration::from_secs(1)));
    let frames = fixture().frames();
    let mut pauses = 0usize;
    let media = play_until(
        &mut host,
        &frames,
        |_| 0,
        |host| {
            pauses = host.count_actions(|action| matches!(action, Action::PauseRead { .. }));
            pauses > 0
        },
    );
    assert_eq!(pauses, 1, "the cap paused the connection exactly once");
    assert!(host.session.read_paused(conn));
    assert!(
        media.end_ticks() >= cap,
        "paused with {} ticks delivered, before the {cap}-tick cap",
        media.end_ticks()
    );

    host.clear_actions();
    let held = media.position_behind(cap);
    host.session
        .on_playback_position(ts(host.now), Some(clock(held)));
    host.drain();
    assert_eq!(
        host.count_actions(|action| matches!(action, Action::ResumeRead { .. })),
        0,
        "a lead still at the cap resumed reading"
    );
    assert!(host.session.read_paused(conn));

    let half = media.position_behind(cap / 4);
    host.session
        .on_playback_position(ts(host.now), Some(clock(half)));
    host.drain();
    assert_eq!(
        host.count_actions(|action| matches!(action, Action::ResumeRead { .. })),
        1,
        "a lead under half the cap resumed reading exactly once"
    );
    assert!(!host.session.read_paused(conn));
}

#[test]
fn a_fast_rtsp_source_is_paused_at_the_byte_cap() {
    let cap = 20 * 1024u64;
    let (mut host, conn) = connect(ReadAhead::bytes(cap));
    let frames = fixture().frames();
    let mut pauses = 0usize;
    play_until(
        &mut host,
        &frames,
        |_| 0,
        |host| {
            pauses = host.count_actions(|action| matches!(action, Action::PauseRead { .. }));
            pauses > 0
        },
    );
    assert_eq!(pauses, 1, "the byte cap paused the connection exactly once");
    assert!(host.session.read_paused(conn));
    assert!(
        host.recorded.len() as u64 >= cap,
        "paused after {} bytes, before the {cap}-byte cap",
        host.recorded.len()
    );
}

#[test]
fn a_read_pause_is_not_an_rtsp_stall() {
    let mut config = config(ReadAhead::time(core::time::Duration::from_secs(1)));
    config.ts_http.stall_notice = std::time::Duration::from_millis(100);
    config.ts_http.stall_reconnect = std::time::Duration::from_millis(200);
    let mut host = Host::new(BASE_URL, config, Server::new(&fixture().sdp));
    host.connect();
    let conn = host.conn.expect("the RTSP session connects");
    let frames = fixture().frames();
    let mut pauses = 0usize;
    play_until(
        &mut host,
        &frames,
        |_| 0,
        |host| {
            pauses = host.count_actions(|action| matches!(action, Action::PauseRead { .. }));
            pauses > 0
        },
    );
    assert!(host.session.read_paused(conn));

    host.clear_actions();
    for _ in 0..20 {
        host.advance(100_000);
    }
    assert_eq!(
        host.count_actions(|action| matches!(action, Action::Close { .. })),
        0,
        "a deliberate read pause reconnected the RTSP session"
    );
    assert_eq!(
        host.count_actions(|action| matches!(action, Action::TcpConnect { .. })),
        0,
        "a deliberate read pause restarted the RTSP connect"
    );
    let events = host.events();
    for event in &events {
        assert!(
            !matches!(
                event,
                broadcast_client::Event::Status(broadcast_client::Status::Stalled { .. })
                    | broadcast_client::Event::Starved(_)
            ),
            "a read pause was reported as a stall: {event:?}"
        );
    }
    assert!(host.session.read_paused(conn));
}

/// The actions the committed RTSP fixture produced before the read-ahead gate
/// existed for RTSP (`rtsp_pull_golden`'s input): one connect, then the
/// DESCRIBE/SETUP/PLAY exchange, and no pause.
const COMMITTED_ACTIONS: [&str; 5] = ["TcpConnect", "DESCRIBE", "SETUP", "SETUP", "PLAY"];
fn action_log(host: &Host) -> Vec<String> {
    host.actions
        .iter()
        .filter_map(|action| match action {
            Action::TcpConnect { .. } => Some("TcpConnect".to_string()),
            Action::Send { bytes, .. } => {
                let line = String::from_utf8_lossy(bytes);
                line.split_whitespace().next().map(str::to_string)
            }
            Action::Close { .. } => Some("Close".to_string()),
            Action::PauseRead { .. } => Some("PauseRead".to_string()),
            Action::ResumeRead { .. } => Some("ResumeRead".to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn an_unlimited_rtsp_session_never_pauses_the_connection() {
    let (mut host, conn) = connect(ReadAhead::Unlimited);
    let frames = fixture().frames();
    host.deliver_all(&frames);
    let events = host.events();
    assert!(
        !samples_of(&events).is_empty(),
        "the RTSP fixture delivered no sample"
    );
    assert_eq!(
        action_log(&host),
        COMMITTED_ACTIONS,
        "an unlimited RTSP session changed the actions the fixture produced before the gate"
    );
    assert!(!host.session.read_paused(conn));
}
