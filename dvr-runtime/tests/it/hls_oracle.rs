//! Apple `mediastreamvalidator` oracle for the archive playlists (DESIGN
//! §3.7). macOS-only; skips loudly when the tool is absent (as
//! `media-doctor`'s oracle test does).

use std::time::Duration;

use crate::support::{cleanup, temp_dir};

fn closed(dir: &std::path::Path, playlist: &str) -> bool {
    std::fs::read_to_string(dir.join(playlist)).is_ok_and(|text| text.contains("#EXT-X-ENDLIST"))
}

fn validate(dir: &std::path::Path, playlist: &str) {
    let json = test_fixtures::process::msv::MsvRun::new(dir, playlist)
        .timeout(Duration::from_secs(60))
        .deadline(Duration::from_secs(120))
        .run();
    let findings = test_fixtures::process::msv::findings(&json);
    let errors: Vec<_> = findings
        .iter()
        .filter(|(_, level, _)| *level <= 1)
        .collect();
    assert!(
        errors.is_empty(),
        "{playlist}: the validator reported conformance errors: {errors:?}"
    );
    for (path, level, comment) in &findings {
        assert!(
            comment.to_lowercase().contains("unrecognized")
                || comment.to_lowercase().contains("unknown"),
            "{playlist}: unexpected finding [level {level}] {path}: {comment}"
        );
        eprintln!("  warning [{level}] {path}: {comment}");
    }
}

/// Validate one closed archive playlist against Apple's validator, skipping
/// loudly when the tool is absent, so a recording another test makes meets the
/// independent oracle too.
pub fn validate_if_available(dir: &std::path::Path, playlist: &str) {
    if !test_fixtures::process::msv::available() {
        eprintln!(
            "SKIP hls_oracle: `mediastreamvalidator` not on PATH (macOS-only Apple HLS \
             conformance tool). This is a no-op result on this host, not real coverage."
        );
        return;
    }
    validate(dir, playlist);
}

#[test]
fn archive_playlists_pass_mediastreamvalidator() {
    if !test_fixtures::process::msv::available() {
        eprintln!(
            "SKIP hls_oracle: `mediastreamvalidator` not on PATH (macOS-only Apple HLS \
             conformance tool). This is a no-op result on this host, not real coverage."
        );
        return;
    }

    let ts = crate::real_dvbt::record_into(temp_dir("oracle-ts"));
    let dir = ts.root.join("cam");
    for period in ts
        .scan
        .segments
        .iter()
        .map(|s| s.period)
        .collect::<std::collections::BTreeSet<_>>()
    {
        let playlist = format!("p{period}.m3u8");
        if closed(&dir, &playlist) {
            validate(&dir, &playlist);
        }
    }
    cleanup(&ts.root);

    let fmp4 = crate::fmp4::record_into(temp_dir("oracle-fmp4"));
    let dir = fmp4.root.join("cam");
    for period in &fmp4.periods {
        let playlist = format!("p{period}.m3u8");
        if closed(&dir, &playlist) {
            validate(&dir, &playlist);
        }
    }
    cleanup(&fmp4.root);
}
