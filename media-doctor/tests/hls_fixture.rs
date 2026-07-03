//! Fixture tests for `check_playlist` — HLS playlist validation against real
//! committed `.m3u8` files.
//!
//! Fixtures live in `fixtures/hls/`:
//! - `valid.m3u8` — well-formed playlist → zero findings.
//! - `missing-extm3u.m3u8` — missing `#EXTM3U` header.
//! - `bad-extinf.m3u8` — EXTINF 15.0 vs TARGETDURATION 10.
//! - `malformed-daterange.m3u8` — DATERANGE missing required `ID` attribute.

use std::fs;

use media_doctor::{Report, check_playlist};

fn read_fixture(name: &str) -> String {
    let path = format!("{}/../fixtures/hls/{}", env!("CARGO_MANIFEST_DIR"), name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read fixture {path}: {e}"))
}

fn findings<'a>(report: &'a Report, rule_id: &str) -> Vec<&'a media_doctor::Finding> {
    report
        .findings()
        .iter()
        .filter(|f| f.rule_id == rule_id)
        .collect()
}

/// `valid.m3u8` — well-formed playlist: #EXTM3U header, TARGETDURATION, EXTINF
/// durations ≤ target, valid DATERANGE. Expect ZERO findings.
#[test]
fn valid_fixture() {
    let text = read_fixture("valid.m3u8");
    let mut report = Report::new();
    check_playlist(&text, &mut report);

    assert!(
        report.is_empty(),
        "valid playlist should produce no findings, got {}: {:?}",
        report.len(),
        report.findings(),
    );
}

/// `missing-extm3u.m3u8` — first non-empty line is `#EXT-X-VERSION:3` instead of
/// `#EXTM3U`. Expect at least one `hls-missing-extm3u` finding.
#[test]
fn missing_extm3u_fixture() {
    let text = read_fixture("missing-extm3u.m3u8");
    let mut report = Report::new();
    check_playlist(&text, &mut report);

    let extm3u = findings(&report, "hls-missing-extm3u");
    assert!(
        !extm3u.is_empty(),
        "missing-extm3u fixture should produce hls-missing-extm3u finding(s), got {}: {:?}",
        extm3u.len(),
        report.findings(),
    );
}

/// `bad-extinf.m3u8` — EXTINF 15.0 vs TARGETDURATION 10 (15 rounded = 15 > 10).
/// Expect at least one `hls-extinf-exceeds-target` finding.
#[test]
fn bad_extinf_fixture() {
    let text = read_fixture("bad-extinf.m3u8");
    let mut report = Report::new();
    check_playlist(&text, &mut report);

    let exceeds = findings(&report, "hls-extinf-exceeds-target");
    assert!(
        !exceeds.is_empty(),
        "bad-extinf fixture should produce hls-extinf-exceeds-target finding(s), got {}: {:?}",
        exceeds.len(),
        report.findings(),
    );
}

/// `malformed-daterange.m3u8` — DATERANGE line missing required `ID` attribute.
/// Expect at least one `hls-malformed-daterange` finding.
#[test]
fn malformed_daterange_fixture() {
    let text = read_fixture("malformed-daterange.m3u8");
    let mut report = Report::new();
    check_playlist(&text, &mut report);

    let dr = findings(&report, "hls-malformed-daterange");
    assert!(
        !dr.is_empty(),
        "malformed-daterange fixture should produce hls-malformed-daterange finding(s), got {}: {:?}",
        dr.len(),
        report.findings(),
    );
}

/// `real-apple-vod.m3u8` — a genuine Apple HLS VOD media playlist (bipbop sample,
/// 181 real segments). A real, well-formed playlist must produce zero findings.
#[test]
fn real_apple_vod_playlist_clean() {
    let text = read_fixture("real-apple-vod.m3u8");
    let mut report = Report::new();
    media_doctor::check_playlist(&text, &mut report);
    assert!(
        report.is_empty(),
        "real Apple VOD playlist should have no findings, got {}: {:?}",
        report.len(),
        report.findings(),
    );
}
