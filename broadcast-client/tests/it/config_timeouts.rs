use std::time::Duration;

use broadcast_client::{Config, Session, SourceKind};
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
    assert_eq!(c.request_timeout_us, 5_000_000);
    assert_eq!(c.segment_timeout_us, 5_000_000);
    assert_eq!(c.hls_key_timeout_us, 5_000_000);
}

#[test]
fn dash_and_smooth_bound_manifest_and_segment_requests_only() {
    let base = Config::relay();
    for kind in [SourceKind::Dash, SourceKind::Smooth] {
        let c = with(kind, timeouts(7_000, 5_000));
        assert_eq!(c.request_timeout_us, 5_000_000, "{kind:?}");
        assert_eq!(c.segment_timeout_us, 5_000_000, "{kind:?}");
        assert_eq!(c.hls_key_timeout_us, base.hls_key_timeout_us, "{kind:?}");
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
    assert_eq!(c.request_timeout_us, base.request_timeout_us);
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
        assert_eq!(auto.request_timeout_us, want.request_timeout_us, "{kind:?}");
        assert_eq!(auto.segment_timeout_us, want.segment_timeout_us, "{kind:?}");
    }
    assert_eq!(auto.hls_key_timeout_us, 5_000_000);
    assert_eq!(auto.request_timeout_us, 5_000_000);
    assert_eq!(auto.srt.timeouts.handshake, Duration::from_secs(3));
    assert_eq!(auto.srt.timeouts.read_idle, Duration::from_secs(5));
    assert_eq!(auto.ts_http.start_timeout_ms, 7_000);
    assert_ne!(auto.request_timeout_us, base.request_timeout_us);
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
        assert_eq!(c.ts_http.start_timeout_ms, 7_000, "{kind:?}");
        assert_eq!(c.ts_http.stall_reconnect_ms, 30_000, "{kind:?}");
        assert_eq!(c.ts_http.stall_notice_ms, 15_000, "{kind:?}");
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
    assert_eq!(hls.request_timeout_us, 1_000);
    let srt = with(SourceKind::Srt, zero);
    assert_eq!(srt.srt.timeouts.handshake, Duration::ZERO);
    assert_eq!(srt.srt.timeouts.read_idle, Duration::ZERO);
    assert!(
        Session::open("srt://127.0.0.1:9000", srt, 0).is_err(),
        "a zero SRT bound is rejected when the session opens"
    );
    let ts = with(SourceKind::TsHttp, zero);
    assert_eq!(
        (
            ts.ts_http.start_timeout_ms,
            ts.ts_http.stall_notice_ms,
            ts.ts_http.stall_reconnect_ms
        ),
        (1, 1, 2)
    );
    assert!(ts.ts_http.validate().is_ok());
}
