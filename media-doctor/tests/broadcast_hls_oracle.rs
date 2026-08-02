//! Oracle assertions for `broadcast-hls`-generated HLS playlists.
//!
//! When a playlist built with `broadcast_hls::MediaPlaylist` validates clean
//! under `check_playlist`/`check_hls_playlist`, this proves the renderer's
//! output is RFC-8216-conformant from the outside — an independent,
//! cross-crate oracle assertion rather than a self-consistent unit test.
//!
//! These tests live in `media-doctor/tests/` because `media-doctor`
//! normal-depends on `broadcast-hls` — no new dependency edge, and the test
//! lives in the topologically highest crate it touches (per the
//! governing rule: a cross-crate test may never live in a crate that
//! sits below one of its own dependencies).

use broadcast_hls::{MediaPlaylist, MediaSegment};
use media_doctor::Report;

#[test]
fn broadcast_hls_media_playlist_validates_clean() {
    let pl = MediaPlaylist {
        version: 3,
        target_duration: 10,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: vec![
            MediaSegment {
                uri: "seg0.m4s".into(),
                duration: 9.009,
                discontinuous: false,
                parts: vec![],
                ..Default::default()
            },
            MediaSegment {
                uri: "seg1.m4s".into(),
                duration: 9.009,
                discontinuous: false,
                parts: vec![],
                ..Default::default()
            },
            MediaSegment {
                uri: "seg2.m4s".into(),
                duration: 3.003,
                discontinuous: false,
                parts: vec![],
                ..Default::default()
            },
        ],
        endlist: true,
        extra_tags: vec![
            "#EXT-X-DATERANGE:ID=\"ad-1\",START-DATE=\"2024-01-01T00:00:00.000Z\",DURATION=15.0"
                .into(),
        ],
        low_latency: None,
        iframes_only: false,
        open_segment: None,
        ..Default::default()
    };

    let m3u8 = pl.to_m3u8();

    let mut report = Report::new();
    media_doctor::check_playlist(&m3u8, &mut report);
    assert!(
        report.is_empty(),
        "media playlist must be RFC-valid but got: {report}",
    );
}

#[test]
fn broadcast_hls_playlist_invalid_target_duration_reported() {
    let pl = MediaPlaylist {
        version: 3,
        target_duration: 10,
        media_sequence: 0,
        discontinuity_sequence: 0,
        segments: vec![MediaSegment {
            uri: "long.m4s".into(),
            duration: 15.0,
            discontinuous: false,
            parts: vec![],
            ..Default::default()
        }],
        endlist: true,
        extra_tags: vec![],
        low_latency: None,
        iframes_only: false,
        open_segment: None,
        ..Default::default()
    };

    let m3u8 = pl.to_m3u8();

    let mut report = Report::new();
    media_doctor::check_playlist(&m3u8, &mut report);
    assert!(
        !report.is_empty(),
        "segment duration > target_duration should produce findings"
    );
}
