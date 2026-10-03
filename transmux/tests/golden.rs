//! Byte-for-byte golden gate for the XML renderers and parsers: DASH MPD,
//! LL-DASH MPD, Smooth manifest, PlayReady WRMHEADER, and the parse results
//! (`Debug` dumps) of the committed MPD fixtures. The expected files in
//! `tests/golden/` were generated from the pre-quick-xml code on `origin/main`
//! (see `tests/golden/README.md`). Set `GOLDEN_BLESS=<dir>` to write the files
//! instead of comparing.
#![cfg(feature = "std")]

use std::fs;
use std::path::{Path, PathBuf};

use broadcast_common::{Package, Unpackage};
use transmux::dash::{
    Addressing, ContentProtectionSystem, DashPackager, InbandEventStream, TrackSegments,
    TrickModeAdaptationSet, TrickModeRepr,
};
use transmux::ll_dash::{LlDashPackager, UTCTIMING_HTTP_HEAD_2014};
use transmux::ts_demux::TsDemux;
use transmux::{Mpd, SmoothManifest, SmoothPackager, playready_wrmheader};

fn check(name: &str, actual: &str) {
    if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
        let path = Path::new(&dir).join(name);
        fs::create_dir_all(&dir).expect("create golden dir");
        fs::write(&path, actual).expect("write golden");
        return;
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    assert_eq!(actual, expected, "{name} differs from the golden output");
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures")
}

fn media() -> transmux::media::Media {
    let ts = fs::read(fixtures().join("ts/h264_aac.ts")).expect("ts fixture");
    TsDemux::new().unpackage(&ts[..]).expect("demux")
}

/// Run-length-encodable segment durations for every track.
fn timeline_segments(media: &transmux::media::Media) -> Vec<TrackSegments> {
    media
        .tracks
        .iter()
        .map(|t| TrackSegments {
            track_id: t.spec.track_id,
            durations: vec![180_000, 180_000, 180_000, 90_000, 45_000],
        })
        .collect()
}

#[test]
fn dash_mpd_variants_match_golden() {
    let media = media();
    let kid = [
        0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
        0x00,
    ];
    let variants: Vec<(&str, DashPackager)> = vec![
        ("default", DashPackager::default()),
        (
            "dynamic",
            DashPackager {
                dynamic: true,
                availability_start_time: Some("2026-01-01T00:00:00Z".into()),
                publish_time: Some("2026-01-01T00:00:01Z".into()),
                minimum_update_period: Some("PT2S".into()),
                time_shift_buffer_depth: Some("PT30S".into()),
                suggested_presentation_delay: Some("PT4S".into()),
                ..DashPackager::default()
            },
        ),
        (
            "timeline",
            DashPackager {
                addressing: Addressing::Timeline,
                segments: timeline_segments(&media),
                ..DashPackager::default()
            },
        ),
        (
            "protected",
            DashPackager {
                content_protection: vec![
                    ContentProtectionSystem {
                        scheme_id_uri: "urn:mpeg:dash:mp4protection:2011".to_string(),
                        value: Some("cenc".to_string()),
                        default_kid: Some(kid),
                        pssh: None,
                    },
                    ContentProtectionSystem {
                        scheme_id_uri: "urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed".to_string(),
                        value: None,
                        default_kid: None,
                        pssh: Some(vec![0, 1, 2, 3, 250, 251, 252]),
                    },
                ],
                inband_event_streams: vec![InbandEventStream {
                    scheme_id_uri: "urn:scte:scte35:2013:bin".to_string(),
                    value: None,
                }],
                ..DashPackager::default()
            },
        ),
        (
            "trick",
            DashPackager {
                trick_mode: Some(TrickModeAdaptationSet {
                    id: "trick-0".into(),
                    main_adaptation_set_id: "main-0".into(),
                    max_playout_rate: 8,
                    repr: TrickModeRepr {
                        id: "trick-repr-0".into(),
                        codecs: "avc1.64001e".into(),
                        bandwidth: 150_000,
                        width: Some(320),
                        height: Some(180),
                        timescale: 90000,
                        total_duration: 900000,
                    },
                }),
                ..DashPackager::default()
            },
        ),
    ];
    for (name, mut pkg) in variants {
        let mpd = pkg
            .package(&media)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        check(&format!("dash-{name}.mpd"), &mpd);
    }
}

#[test]
fn ll_dash_mpd_variants_match_golden() {
    let media = media();
    let plain = LlDashPackager::new(2.0, 0.5, 3000, "2026-01-01T00:00:00Z").expect("packager");
    let rate = LlDashPackager::new(4.0, 1.0, 5000, "2026-01-01T00:00:00Z")
        .expect("packager")
        .with_playback_rate(0.96, 1.04);
    let utc = LlDashPackager::new(2.0, 0.5, 3000, "2026-01-01T00:00:00Z")
        .expect("packager")
        .with_utc_timing(UTCTIMING_HTTP_HEAD_2014, "https://time.example/x?a=1&b=2");
    let mut timeline =
        LlDashPackager::new(2.0, 0.5, 3000, "2026-01-01T00:00:00Z").expect("packager");
    timeline.base.addressing = Addressing::Timeline;
    timeline.base.segments = timeline_segments(&media);
    for (name, mut pkg) in [
        ("plain", plain),
        ("rate", rate),
        ("utc", utc),
        ("timeline", timeline),
    ] {
        let mpd = pkg
            .package(&media)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        check(&format!("lldash-{name}.mpd"), &mpd);
    }
}

#[test]
fn smooth_manifest_matches_golden() {
    let out = SmoothPackager::default().package(&media()).expect("smooth");
    check("smooth.manifest.xml", &out.manifest);
    let parsed = SmoothManifest::parse(&out.manifest).expect("parse own manifest");
    check("smooth.parse.txt", &format!("{parsed:#?}\n"));
}

#[test]
fn playready_wrmheader_matches_golden() {
    let kid_a = [0x11u8; 16];
    let kid_b: [u8; 16] = core::array::from_fn(|i| i as u8);
    check(
        "wrm-two-kids.xml",
        &playready_wrmheader(&[kid_a, kid_b], Some("https://la.example.com/x?a=1&b=2")),
    );
    check("wrm-no-url.xml", &playready_wrmheader(&[kid_a], None));
    check(
        "wrm-specials.xml",
        &playready_wrmheader(&[kid_a], Some("a&b<c>d\"e'f")),
    );
}

#[test]
fn mpd_parse_results_match_golden() {
    for name in ["manifest", "manifest-inheritance"] {
        let text =
            fs::read_to_string(fixtures().join(format!("dash/{name}.mpd"))).expect("fixture");
        let parsed = Mpd::parse(&text).expect("parse");
        check(&format!("parse-{name}.txt"), &format!("{parsed:#?}\n"));
    }
    let own = DashPackager::default().package(&media()).expect("mpd");
    let parsed = Mpd::parse(&own).expect("parse own mpd");
    check("parse-own-mpd.txt", &format!("{parsed:#?}\n"));
}
