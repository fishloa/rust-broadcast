//! XML escaping gate for every `quick-xml`-backed writer in `transmux`: a value
//! carrying all five XML specials (`& < > " '`) must be escaped on the wire
//! (exact output asserted) and must re-parse to the original string.
#![cfg(feature = "std")]

use std::path::PathBuf;

use broadcast_common::{Package, Unpackage};
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::Event;
use transmux::dash::DashPackager;
use transmux::ll_dash::{LlDashPackager, UTCTIMING_HTTP_HEAD_2014};
use transmux::ts_demux::TsDemux;
use transmux::{Mpd, playready_wrmheader};

/// All five XML specials in one string.
const SPECIALS: &str = "a&b<c>d\"e'f";
/// `SPECIALS` as quick-xml writes it into an attribute value.
const SPECIALS_ATTR: &str = "a&amp;b&lt;c&gt;d&quot;e&apos;f";

fn demux_media() -> transmux::media::Media {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264_aac.ts");
    let ts = std::fs::read(path).expect("h264_aac.ts fixture must exist");
    TsDemux::new()
        .unpackage(&ts[..])
        .expect("demux h264_aac.ts")
}

/// Collect `(element, attribute, value)` for every start tag via quick-xml.
fn attrs_of(xml: &str, element: &str, attr: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    loop {
        match reader.read_event().expect("well-formed XML") {
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == element => {
                for a in e.attributes() {
                    let a = a.expect("attribute");
                    if a.key.as_ref() == attr {
                        return Some(
                            a.normalized_value(XmlVersion::Implicit1_0)
                                .expect("value")
                                .into_owned(),
                        );
                    }
                }
            }
            Event::Eof => return None,
            _ => {}
        }
    }
}

/// The unescaped text of the first `<element>` (entity references resolved).
fn text_of(xml: &str, element: &str) -> String {
    let mut reader = Reader::from_str(xml);
    let mut inside = false;
    let mut out = String::new();
    loop {
        match reader.read_event().expect("well-formed XML") {
            Event::Start(e) if e.name().as_ref() == element => inside = true,
            Event::End(e) if e.name().as_ref() == element => return out,
            Event::Text(t) if inside => out.push_str(&t.xml10_content()),
            Event::GeneralRef(r) if inside => {
                let c = r.resolve_char_ref().expect("char ref");
                match c {
                    Some(c) => out.push(c),
                    None => out.push_str(resolve_predefined_entity(&r).expect("entity")),
                }
            }
            Event::Eof => panic!("no <{element}>"),
            _ => {}
        }
    }
}

#[test]
fn dash_mpd_escapes_caller_strings_and_round_trips() {
    let media = demux_media();
    let mut pkg = DashPackager {
        profiles: SPECIALS.to_string(),
        availability_start_time: Some(SPECIALS.to_string()),
        dynamic: true,
        ..DashPackager::default()
    };
    let xml = pkg.package(&media).expect("package MPD");

    assert!(
        xml.contains(&format!("profiles=\"{SPECIALS_ATTR}\"")),
        "profiles must be escaped on the wire: {xml}"
    );
    assert!(
        xml.contains(&format!("availabilityStartTime=\"{SPECIALS_ATTR}\"")),
        "availabilityStartTime must be escaped on the wire"
    );
    // Round trip through the crate's own parser.
    let mpd = Mpd::parse(&xml).expect("re-parse");
    assert_eq!(mpd.profiles, SPECIALS);
    assert_eq!(mpd.availability_start_time.as_deref(), Some(SPECIALS));
}

#[test]
fn ll_dash_utc_timing_escapes_scheme_and_value() {
    let media = demux_media();
    let mut pkg = LlDashPackager::new(2.0, 0.5, 3000, "2026-01-01T00:00:00Z")
        .expect("packager")
        .with_utc_timing(SPECIALS, SPECIALS);
    let xml = pkg.package(&media).expect("LL MPD");

    assert!(
        xml.contains(&format!(
            "<UTCTiming schemeIdUri=\"{SPECIALS_ATTR}\" value=\"{SPECIALS_ATTR}\"/>"
        )),
        "UTCTiming attributes must be escaped on the wire: {xml}"
    );
    assert_eq!(
        attrs_of(&xml, "UTCTiming", "value").as_deref(),
        Some(SPECIALS)
    );
    assert_eq!(
        attrs_of(&xml, "UTCTiming", "schemeIdUri").as_deref(),
        Some(SPECIALS)
    );
    // The registered scheme still round-trips untouched.
    let mut pkg = LlDashPackager::new(2.0, 0.5, 3000, "2026-01-01T00:00:00Z")
        .expect("packager")
        .with_utc_timing(UTCTIMING_HTTP_HEAD_2014, "https://t.example/x?a=1&b=2");
    let xml = pkg.package(&media).expect("LL MPD");
    assert_eq!(
        attrs_of(&xml, "UTCTiming", "value").as_deref(),
        Some("https://t.example/x?a=1&b=2")
    );
}

#[test]
fn ll_dash_rewrite_preserves_base_mpd_attribute_escaping() {
    // The LL rewrite re-reads the base MPD: an escaped `profiles` must come out
    // exactly as the base writer produced it (no double escaping).
    let media = demux_media();
    let mut pkg = LlDashPackager::new(2.0, 0.5, 3000, "2026-01-01T00:00:00Z").expect("packager");
    pkg.base.profiles = SPECIALS.to_string();
    let xml = pkg.package(&media).expect("LL MPD");
    assert!(
        xml.contains(&format!("profiles=\"{SPECIALS_ATTR}\"")),
        "{xml}"
    );
    assert!(!xml.contains("&amp;amp;"), "double escaping: {xml}");
    assert_eq!(attrs_of(&xml, "MPD", "profiles").as_deref(), Some(SPECIALS));
}

#[test]
fn playready_wrmheader_escapes_la_url_text() {
    let kid = [0x11u8; 16];
    let xml = playready_wrmheader(&[kid], Some(SPECIALS));
    // Element content: `& < >` escaped, quotes left alone (not required in text).
    assert!(
        xml.contains("<LA_URL>a&amp;b&lt;c&gt;d\"e'f</LA_URL>"),
        "LA_URL text must be escaped on the wire: {xml}"
    );
    assert_eq!(text_of(&xml, "LA_URL"), SPECIALS);
    // The whole header re-parses as well-formed XML.
    assert_eq!(
        attrs_of(&xml, "WRMHEADER", "version").as_deref(),
        Some("4.2.0.0")
    );
}
