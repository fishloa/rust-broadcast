use std::time::Duration;

use broadcast_client::{Config, SourceKind};
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
fn srt_takes_its_handshake_bound_from_connect_and_keeps_its_idle_bound() {
    let base = Config::relay();
    let c = with(SourceKind::Srt, timeouts(7_000, 5_000));
    assert_eq!(c.srt.handshake_timeout_ms, 7_000);
    assert_eq!(c.srt.peer_idle_timeout_ms, base.srt.peer_idle_timeout_ms);
    assert_eq!(c.request_timeout_us, base.request_timeout_us);
}

#[test]
fn the_stall_policy_kinds_take_start_from_connect_and_reconnect_from_read_idle() {
    for kind in [
        SourceKind::TsHttp,
        SourceKind::Rtsp,
        SourceKind::Rtmp,
        SourceKind::Udp,
        SourceKind::Rtp,
        SourceKind::Auto,
    ] {
        let c = with(kind, timeouts(7_000, 30_000));
        assert_eq!(c.ts_http.start_timeout_ms, 7_000, "{kind:?}");
        assert_eq!(c.ts_http.stall_reconnect_ms, 30_000, "{kind:?}");
        assert_eq!(c.ts_http.stall_notice_ms, 15_000, "{kind:?}");
    }
}

/// A zero bound never produces a zero timeout: every protocol floors at one
/// millisecond (two for the stall-reconnect threshold, so its half stays
/// positive and the notice stays below it).
#[test]
fn a_zero_bound_is_floored_for_every_protocol() {
    let zero = timeouts(0, 0);
    let hls = with(SourceKind::Hls, zero);
    assert_eq!(hls.request_timeout_us, 1_000);
    let srt = with(SourceKind::Srt, zero);
    assert_eq!(srt.srt.handshake_timeout_ms, 1);
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
