#![cfg(feature = "std")]

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use broadcast_common::{Package, Unpackage};
use transmux::TrackId;
use transmux::dash::{
    Addressing, ContentProtectionSystem, DashPackager, TrackSegments, TrickModeAdaptationSet,
    TrickModeRepr,
};
use transmux::ll_dash::{LlDashPackager, UTCTIMING_HTTP_HEAD_2014};
use transmux::media::Media;
use transmux::ts_demux::TsDemux;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures")
}

fn media() -> Media {
    let ts = fs::read(fixtures().join("ts/h264_aac.ts")).expect("ts fixture");
    TsDemux::new().unpackage(&ts[..]).expect("demux")
}

fn four_audio_media() -> Media {
    let mut tracks = Vec::new();
    for rel in [
        "ts/h264_aac.ts",
        "ts/dolby/ac3.ts",
        "ts/dts/dts_core.ts",
        "ts/legacy/mpeg2_mp2.ts",
    ] {
        let bytes = test_fixtures::read(rel);
        let m = TsDemux::new().unpackage(&bytes).expect("demux");
        tracks.extend(m.tracks);
    }
    for (i, t) in tracks.iter_mut().enumerate() {
        t.spec.track_id = TrackId::new((i + 1) as u32);
    }
    Media::new(tracks, 90_000)
}

fn timeline_segments(media: &Media) -> Vec<TrackSegments> {
    media
        .tracks
        .iter()
        .map(|t| TrackSegments {
            track_id: t.spec.track_id,
            durations: vec![180_000, 180_000, 180_000, 90_000, 45_000],
        })
        .collect()
}

fn schema_path() -> Option<PathBuf> {
    let p = test_fixtures::private_root()?
        .join("specs/iso_iec_23009-1_dash_2026_ed6_schema/DASH-MPD.xsd");
    p.exists().then_some(p)
}

fn xmllint_available() -> bool {
    Command::new("xmllint")
        .arg("--version")
        .output()
        .map(|o| o.status.success() || !o.stderr.is_empty())
        .unwrap_or(false)
}

fn validate(schema: &Path, name: &str, mpd: &str) {
    let tmp = std::env::temp_dir().join(format!("dash_xsd_{}.mpd", name));
    fs::write(&tmp, mpd).expect("write temp mpd");
    let out = Command::new("xmllint")
        .arg("--noout")
        .arg("--schema")
        .arg(schema)
        .arg(&tmp)
        .output()
        .expect("run xmllint");
    if !out.status.success() {
        panic!(
            "xmllint rejected {name}:\n{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn dash_mpds_validate_against_iso_schema() {
    if !xmllint_available() {
        eprintln!("SKIPPED dash_mpds_validate_against_iso_schema: xmllint not on PATH");
        return;
    }
    let Some(schema) = schema_path() else {
        eprintln!("SKIPPED dash_mpds_validate_against_iso_schema: private schema not checked out");
        return;
    };

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
                content_protection: vec![ContentProtectionSystem {
                    scheme_id_uri: "urn:mpeg:dash:mp4protection:2011".to_string(),
                    value: Some("cenc".to_string()),
                    default_kid: Some(kid),
                    pssh: None,
                }],
                ..DashPackager::default()
            },
        ),
        (
            "trick",
            DashPackager {
                trick_mode: Some(TrickModeAdaptationSet {
                    id: 7,
                    main_adaptation_set_id: 0,
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
        validate(&schema, &format!("dash-{name}"), &mpd);
    }

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
        validate(&schema, &format!("lldash-{name}"), &mpd);
    }
}

use quick_xml::Reader;
use quick_xml::events::Event;

fn adaptation_set_ids(mpd: &str) -> Vec<String> {
    let mut reader = Reader::from_str(mpd);
    let mut ids = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.name().as_ref() == "AdaptationSet" => {
                for a in e.attributes().flatten() {
                    if a.key.as_ref() == "id" {
                        ids.push(a.value.to_string());
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    ids
}

#[test]
fn adaptation_set_ids_are_unsigned_integers_and_unique() {
    let media = media();
    let mpd = DashPackager::default().package(&media).unwrap();
    let ids = adaptation_set_ids(&mpd);
    assert!(!ids.is_empty());
    let mut seen = HashSet::new();
    for id in &ids {
        assert!(
            id.chars().all(|c| c.is_ascii_digit()),
            "not unsigned int: {id}"
        );
        assert!(seen.insert(id.clone()), "duplicate id: {id}");
    }
}

#[test]
fn trick_mode_supplemental_property_references_existing_set() {
    let media = media();
    let mut pkg = DashPackager {
        trick_mode: Some(TrickModeAdaptationSet {
            id: 7,
            main_adaptation_set_id: 0,
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
    };
    let mpd = pkg.package(&media).unwrap();

    let mut reader = Reader::from_str(&mpd);
    let mut trick_id: Option<String> = None;
    let mut prop_value: Option<String> = None;
    let mut in_trick = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if e.name().as_ref() == "AdaptationSet" {
                    let is_trick = e
                        .attributes()
                        .flatten()
                        .any(|a| a.key.as_ref() == "maxPlayoutRate");
                    if is_trick {
                        in_trick = true;
                        trick_id = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.as_ref() == "id")
                            .map(|a| a.value.to_string());
                    }
                }
                if e.name().as_ref() == "SupplementalProperty" && in_trick {
                    prop_value = e
                        .attributes()
                        .flatten()
                        .find(|a| a.key.as_ref() == "value")
                        .map(|a| a.value.to_string());
                }
            }
            Ok(Event::End(e)) if e.name().as_ref() == "AdaptationSet" => in_trick = false,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    let ids = adaptation_set_ids(&mpd);
    assert!(ids.contains(&prop_value.clone().unwrap()));
    assert_eq!(trick_id.as_deref(), Some("7"));
    assert_eq!(prop_value.as_deref(), Some("0"));
}

#[test]
fn trick_mode_distinct_values() {
    let media = four_audio_media();
    let mut pkg = DashPackager {
        trick_mode: Some(TrickModeAdaptationSet {
            id: 7,
            main_adaptation_set_id: 3,
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
    };
    let mpd = pkg.package(&media).unwrap();

    let mut reader = Reader::from_str(&mpd);
    let mut trick_id: Option<String> = None;
    let mut prop_value: Option<String> = None;
    let mut in_trick = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                if e.name().as_ref() == "AdaptationSet" {
                    let is_trick = e
                        .attributes()
                        .flatten()
                        .any(|a| a.key.as_ref() == "maxPlayoutRate");
                    if is_trick {
                        in_trick = true;
                        trick_id = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.as_ref() == "id")
                            .map(|a| a.value.to_string());
                    }
                }
                if e.name().as_ref() == "SupplementalProperty" && in_trick {
                    prop_value = e
                        .attributes()
                        .flatten()
                        .find(|a| a.key.as_ref() == "value")
                        .map(|a| a.value.to_string());
                }
            }
            Ok(Event::End(e)) if e.name().as_ref() == "AdaptationSet" => in_trick = false,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    assert_eq!(trick_id.as_deref(), Some("7"));
    assert_eq!(prop_value.as_deref(), Some("3"));
}

#[test]
fn trick_mode_dangling_reference_errors() {
    let media = media();
    let mut pkg = DashPackager {
        trick_mode: Some(TrickModeAdaptationSet {
            id: 7,
            main_adaptation_set_id: 999,
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
    };
    let err = pkg.package(&media).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("unknown main AdaptationSet"), "got: {msg}");
}
