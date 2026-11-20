use std::time::Duration;
use url::Url;

use broadcast_common::stage::Timestamp;

use broadcast_client::{Config, ConfigField, Error, Session, SourceKind};
use broadcast_common::timeouts::Timeouts;

fn with(kind: SourceKind, timeouts: Timeouts) -> Config {
    let mut config = Config::relay();
    config.kind = kind;
    config.with_timeouts(timeouts)
}

fn timeouts(connect_ms: u64, read_ms: u64) -> Timeouts {
    Timeouts::default()
        .with_connect(Duration::from_millis(connect_ms))
        .with_read_idle(Duration::from_millis(read_ms))
}

#[test]
fn hls_bounds_every_request_by_read_idle() {
    let c = with(SourceKind::Hls, timeouts(7_000, 5_000));
    assert_eq!(c.request_timeout, Duration::from_micros(5_000_000));
    assert_eq!(c.segment_timeout, Duration::from_micros(5_000_000));
    assert_eq!(c.hls_key_timeout, Duration::from_micros(5_000_000));
}

#[test]
fn dash_and_smooth_bound_manifest_and_segment_requests_only() {
    let base = Config::relay();
    for kind in [SourceKind::Dash, SourceKind::Smooth] {
        let c = with(kind, timeouts(7_000, 5_000));
        assert_eq!(
            c.request_timeout,
            Duration::from_micros(5_000_000),
            "{kind:?}"
        );
        assert_eq!(
            c.segment_timeout,
            Duration::from_micros(5_000_000),
            "{kind:?}"
        );
        assert_eq!(c.hls_key_timeout, base.hls_key_timeout, "{kind:?}");
    }
}

#[test]
fn srt_takes_handshake_and_peer_idle_from_the_same_fields_as_srt_config() {
    let t = Timeouts::default()
        .with_connect(Duration::from_millis(7_000))
        .with_handshake(Duration::from_millis(3_000))
        .with_read_idle(Duration::from_millis(5_000));
    let base = Config::relay();
    let c = with(SourceKind::Srt, t);
    assert_eq!(c.srt.timeouts.handshake, Duration::from_secs(3));
    assert_eq!(c.srt.timeouts.read_idle, Duration::from_secs(5));
    let direct = base.srt.clone().with_timeouts(t);
    assert_eq!(c.srt.timeouts, direct.timeouts);
    assert_eq!(c.srt.timeouts.handshake, direct.timeouts.handshake);
    assert_eq!(c.srt.timeouts.read_idle, direct.timeouts.read_idle);
    assert_eq!(c.request_timeout, base.request_timeout);
}

#[test]
fn auto_applies_the_mapping_of_every_protocol_the_url_can_resolve_to() {
    let t = Timeouts::default()
        .with_connect(Duration::from_millis(7_000))
        .with_handshake(Duration::from_millis(3_000))
        .with_read_idle(Duration::from_millis(5_000));
    let base = Config::relay();
    let auto = with(SourceKind::Auto, t);
    for kind in [SourceKind::Hls, SourceKind::Dash, SourceKind::Smooth] {
        let want = with(kind, t);
        assert_eq!(auto.request_timeout, want.request_timeout, "{kind:?}");
        assert_eq!(auto.segment_timeout, want.segment_timeout, "{kind:?}");
    }
    assert_eq!(auto.hls_key_timeout, Duration::from_secs(5));
    assert_eq!(auto.request_timeout, Duration::from_secs(5));
    assert_eq!(auto.srt.timeouts.handshake, Duration::from_secs(3));
    assert_eq!(auto.srt.timeouts.read_idle, Duration::from_secs(5));
    assert_eq!(auto.ts_http.start_timeout, Duration::from_millis(7_000));
    assert_ne!(auto.request_timeout, base.request_timeout);
}

#[test]
fn the_stall_policy_kinds_take_start_from_connect_and_reconnect_from_read_idle() {
    for kind in [
        SourceKind::TsHttp,
        SourceKind::Rtsp,
        SourceKind::Rtmp,
        SourceKind::Udp,
        SourceKind::Rtp,
    ] {
        let c = with(kind, timeouts(7_000, 30_000));
        assert_eq!(
            c.ts_http.start_timeout,
            Duration::from_millis(7_000),
            "{kind:?}"
        );
        assert_eq!(
            c.ts_http.stall_reconnect,
            Duration::from_millis(30_000),
            "{kind:?}"
        );
        assert_eq!(
            c.ts_http.stall_notice,
            Duration::from_millis(15_000),
            "{kind:?}"
        );
    }
}

/// A zero bound never produces a zero timeout: every protocol floors at one
/// millisecond (two for the stall-reconnect threshold, so its half stays
/// positive and the notice stays below it). SRT keeps the bound verbatim: a
/// zero one is rejected by `Config::validate`, never widened silently.
#[test]
fn a_zero_bound_is_floored_for_every_protocol() {
    let zero = timeouts(0, 0).with_handshake(Duration::ZERO);
    let hls = with(SourceKind::Hls, zero);
    assert_eq!(hls.request_timeout, Duration::from_millis(1));
    let srt = with(SourceKind::Srt, zero);
    assert_eq!(srt.srt.timeouts.handshake, Duration::ZERO);
    assert_eq!(srt.srt.timeouts.read_idle, Duration::ZERO);
    assert!(
        Session::open(
            Url::parse("srt://127.0.0.1:9000").unwrap(),
            srt,
            Timestamp::from_micros(0)
        )
        .is_err(),
        "a zero SRT bound is rejected when the session opens"
    );
    let ts = with(SourceKind::TsHttp, zero);
    assert_eq!(
        (
            ts.ts_http.start_timeout,
            ts.ts_http.stall_notice,
            ts.ts_http.stall_reconnect
        ),
        (
            Duration::from_millis(1),
            Duration::from_millis(1),
            Duration::from_millis(2)
        )
    );
    assert!(ts.ts_http.validate().is_ok());
}

#[test]
fn relay_keeps_the_initial_tracks_hold_but_drops_the_start_wait_and_jump_screening() {
    let c = Config::relay();
    assert_eq!(c.start_pending_track_wait, Duration::ZERO);
    assert_eq!(c.start_pending_track_hard_cap, Duration::ZERO);
    assert!(c.timeline_jump.is_none());
    assert_eq!(
        c.tracks_hold_max_wall,
        Config::default().tracks_hold_max_wall,
        "the initial-tracks hold stays for a relay"
    );
}

#[test]
fn tracks_hold_accepts_a_max_equal_to_quiet_and_rejects_one_below_it() {
    let open = |max: usize, quiet: usize| {
        let mut config = Config::default();
        config.tracks_hold_max_input_bytes = max;
        config.tracks_hold_quiet_input_bytes = quiet;
        Session::open(
            Url::parse("http://h/live.ts").unwrap(),
            config,
            Timestamp::from_micros(0),
        )
        .map(|_| ())
    };
    assert!(open(100, 100).is_ok());
    assert!(matches!(
        open(99, 100),
        Err(Error::InvalidConfig(ConfigField::TracksHoldMaxInputBytes))
    ));
    assert!(matches!(
        open(100, 0),
        Err(Error::InvalidConfig(ConfigField::TracksHoldQuietInputBytes))
    ));
}
