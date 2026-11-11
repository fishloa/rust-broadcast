//! Truncation gate for the Smooth manifest parser: cutting a manifest at ANY
//! offset before the end of its root end tag must be a structured error; only
//! the complete document plus trailing whitespace may parse, asserted
//! explicitly as `Ok`. The MPD reader is dash-runtime's.
#![cfg(feature = "std")]

use broadcast_common::{Package, Unpackage};
use broadcast_smooth::SmoothManifest;
use transmux::SmoothPackager;
use transmux::ts_demux::TsDemux;

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

#[test]
fn smooth_manifest_truncations_all_error() {
    let ts = std::fs::read(test_fixtures::path("ts/h264-aac.ts")).expect("ts fixture");
    let media = TsDemux::new().unpackage(&ts[..]).expect("demux");
    let smooth = SmoothPackager::default().package(&media).expect("smooth");
    assert_every_truncation_errors(
        "SmoothPackager manifest",
        &smooth.manifest,
        SmoothManifest::parse,
    );
}
