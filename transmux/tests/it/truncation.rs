//! Truncation gate for the MPD and Smooth manifest parsers: cutting a document
//! at ANY offset before the end of its root end tag must be a structured error;
//! only the complete document plus trailing whitespace may parse, asserted
//! explicitly as `Ok`.
#![cfg(feature = "std")]

use std::fs;
use std::path::PathBuf;

use broadcast_common::{Package, Unpackage};
use transmux::ts_demux::TsDemux;
use transmux::{DashPackager, Mpd, SmoothManifest, SmoothPackager};

fn assert_every_truncation_errors<T, E: std::fmt::Debug>(
    name: &str,
    text: &str,
    parse: impl Fn(&str) -> Result<T, E>,
) {
    let complete = text.trim_end().len();
    for end in 1..text.len() {
        if !text.is_char_boundary(end) {
            continue;
        }
        let result = parse(&text[..end]);
        if end < complete {
            assert!(
                result.is_err(),
                "{name}: truncation at byte {end}/{} must be Err",
                text.len()
            );
        } else {
            assert!(
                result.is_ok(),
                "{name}: prefix of {end} bytes is the complete document (plus whitespace) and must parse: {:?}",
                result.err()
            );
        }
    }
    assert!(parse(text).is_ok(), "{name}: the full document parses");
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures")
}

#[test]
fn committed_mpd_fixtures_truncations_all_error() {
    for name in ["dash/manifest.mpd", "dash/manifest-inheritance.mpd"] {
        let text = fs::read_to_string(fixtures().join(name)).expect("fixture");
        assert_every_truncation_errors(name, &text, Mpd::parse);
    }
}

#[test]
fn packager_output_truncations_all_error() {
    let ts = fs::read(fixtures().join("ts/h264_aac.ts")).expect("ts fixture");
    let media = TsDemux::new().unpackage(&ts[..]).expect("demux");

    let mpd = DashPackager::default().package(&media).expect("mpd");
    assert_every_truncation_errors("DashPackager MPD", &mpd, Mpd::parse);

    let smooth = SmoothPackager::default().package(&media).expect("smooth");
    assert_every_truncation_errors(
        "SmoothPackager manifest",
        &smooth.manifest,
        SmoothManifest::parse,
    );
}

/// Hostile input: very deep nesting of unknown elements is skipped iteratively
/// (no recursion), so a 60k-deep MPD parses without a stack overflow.
#[test]
fn deeply_nested_unknown_elements_do_not_overflow() {
    let depth = 60_000;
    let xml = format!(
        r#"<MPD profiles="p"><Period>{}{}</Period></MPD>"#,
        "<x>".repeat(depth),
        "</x>".repeat(depth)
    );
    assert!(Mpd::parse(&xml).is_ok());
}
