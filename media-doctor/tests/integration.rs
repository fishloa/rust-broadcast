//! Integration tests for `media-doctor`.

use media_doctor::Diagnostic;
use media_doctor::{Finding, Location, Report, Severity, SyncByteCheck};

/// A clean buffer of 2 good TS packets should produce zero findings.
#[test]
fn sync_byte_clean_packets() {
    let mut ts = Vec::new();
    for _ in 0..2 {
        let mut pkt = vec![0x47u8; 188];
        // minimal valid header: sync=0x47, pid=0x1FFF, TSC=00, AFC=01, CC=0
        pkt[3] = 0x10; // AFC=01 (no adaptation), CC=0
        ts.extend_from_slice(&pkt);
    }
    let mut report = Report::new();
    SyncByteCheck.run(&ts, &mut report);
    assert!(
        report.is_empty(),
        "expected no findings, got {}",
        report.len()
    );
}

/// A buffer with one bad sync byte should produce exactly one Error finding.
#[test]
fn sync_byte_one_bad_packet() {
    let mut ts = Vec::new();
    // First packet: good
    let mut pkt1 = vec![0x47u8; 188];
    pkt1[3] = 0x10;
    ts.extend_from_slice(&pkt1);
    // Second packet: bad sync byte (0x00 instead of 0x47)
    let mut pkt2 = vec![0x00u8; 188];
    pkt2[1] = 0x12;
    pkt2[2] = 0x34;
    pkt2[3] = 0x10;
    ts.extend_from_slice(&pkt2);
    // Third packet: good
    let mut pkt3 = vec![0x47u8; 188];
    pkt3[3] = 0x10;
    ts.extend_from_slice(&pkt3);

    let mut report = Report::new();
    SyncByteCheck.run(&ts, &mut report);
    assert_eq!(report.len(), 1);
    let f = &report.findings()[0];
    assert_eq!(f.severity, Severity::Error);
    assert_eq!(f.location.packet, 1);
    assert_eq!(f.rule_id, "sync-byte");
}

/// Report text rendering produces expected output for empty and populated reports.
#[test]
fn report_text_format() {
    // Empty
    let r = Report::new();
    let text = r.to_string();
    assert!(text.contains("No issues found"));

    // Populated
    let mut r = Report::new();
    r.push(Finding::new(
        Severity::Error,
        Location::new(0, 0x0100),
        "sync-byte",
        "bad sync",
    ));
    r.push(Finding::new(
        Severity::Warning,
        Location::new(5, 0x0010),
        "test-rule",
        "warning msg",
    ));
    let text = r.to_string();
    assert!(text.contains("1 error(s), 1 warning(s), 0 info(s)"));
    assert!(text.contains("bad sync"));
    assert!(text.contains("warning msg"));
}

/// JSON round-trip: serialize a Report to JSON and deserialize back.
#[cfg(feature = "serde")]
#[test]
fn report_json_roundtrip() {
    let mut report = Report::new();
    report.push(Finding::new(
        Severity::Error,
        Location::new(42, 0x0100),
        "sync-byte",
        "bad sync byte",
    ));
    report.push(Finding::new(
        Severity::Info,
        Location::new(99, 0),
        "test",
        "info message",
    ));

    let json = serde_json::to_string_pretty(&report).expect("serialize report");
    let deser: Report = serde_json::from_str(&json).expect("deserialize report");
    assert_eq!(report, deser);
}

/// Severity::name() and Display work correctly.
#[test]
fn severity_name_display() {
    assert_eq!(Severity::Error.name(), "error");
    assert_eq!(Severity::Warning.name(), "warning");
    assert_eq!(Severity::Info.name(), "info");
    assert_eq!(Severity::Error.to_string(), "error");
    assert_eq!(Severity::Warning.to_string(), "warning");
    assert_eq!(Severity::Info.to_string(), "info");
}
