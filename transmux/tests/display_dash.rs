//! DASH display geometry (display-geometry Phase 4c): `@sar` on each video
//! `Representation`, `@par` on the video `AdaptationSet` (ETSI TS 103 285
//! v1.4.1 §4.4). Expected values are independent of the code under test:
//! `ffprobe -show_streams` reports width/height/SAR/DAR for each capture.
//!
//! | capture | coded | SAR | ffprobe DAR |
//! |---|---|---|---|
//! | melodie-anamorphic-sar-64-45 | 720x576 | 64:45 | 16:9 |
//! | krone-anamorphic-sar-16-11 | 720x576 | 16:11 | 20:11 |
//! | france-tnt-m6-eac3-zenith | 1920x1080 | 1:1 | 16:9 |

#![cfg(feature = "std")]

use broadcast_common::{Package, Unpackage};
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use transmux::DashPackager;
use transmux::ts_demux::TsDemux;

/// `(@par of the video AdaptationSet, @sar of each video Representation)`,
/// read by parsing the MPD (never by substring search), so an attribute on the
/// wrong element is a failure.
fn video_geometry_attrs(mpd: &str) -> (Option<String>, Vec<Option<String>>) {
    fn attr(e: &BytesStart<'_>, key: &str) -> Option<String> {
        e.attributes()
            .filter_map(Result::ok)
            .find(|a| a.key.as_ref() == key)
            .map(|a| {
                a.normalized_value(XmlVersion::Implicit1_0)
                    .expect("resolvable attribute value")
                    .into_owned()
            })
    }
    let mut reader = Reader::from_str(mpd);
    let mut in_video_set = false;
    let mut par = None;
    let mut sars = Vec::new();
    loop {
        match reader.read_event().expect("well-formed MPD") {
            Event::Start(e) | Event::Empty(e) => match e.name().as_ref() {
                "AdaptationSet" => {
                    in_video_set = attr(&e, "contentType").as_deref() == Some("video");
                    if in_video_set {
                        par = attr(&e, "par");
                    }
                }
                "Representation" if in_video_set => sars.push(attr(&e, "sar")),
                _ => {}
            },
            Event::End(e) if e.name().as_ref() == "AdaptationSet" => in_video_set = false,
            Event::Eof => break,
            _ => {}
        }
    }
    (par, sars)
}

fn mpd_for(fixture: &str) -> Option<String> {
    let data = test_fixtures::require_bytes(fixture, "display_dash")?;
    let media = TsDemux::new().unpackage(&data).expect("demux fixture");
    Some(
        DashPackager::default()
            .package(&media)
            .expect("package DASH"),
    )
}

#[test]
fn melodie_anamorphic_64_45() {
    let Some(mpd) = mpd_for("ts/melodie-anamorphic-sar-64-45-zenith-tv.ts") else {
        return;
    };
    let (par, sars) = video_geometry_attrs(&mpd);
    assert_eq!(sars, vec![Some("64:45".to_string())]);
    assert_eq!(par.as_deref(), Some("16:9"));
}

#[test]
fn krone_anamorphic_16_11() {
    let Some(mpd) = mpd_for("ts/krone-anamorphic-sar-16-11-zenith-tv.ts") else {
        return;
    };
    let (par, sars) = video_geometry_attrs(&mpd);
    assert_eq!(sars, vec![Some("16:11".to_string())]);
    assert_eq!(par.as_deref(), Some("20:11"));
}

#[test]
fn square_pixel_1080p() {
    let Some(mpd) = mpd_for("ts/france-tnt-m6-eac3-zenith.ts") else {
        return;
    };
    let (par, sars) = video_geometry_attrs(&mpd);
    assert_eq!(sars, vec![Some("1:1".to_string())]);
    assert_eq!(par.as_deref(), Some("16:9"));
}
