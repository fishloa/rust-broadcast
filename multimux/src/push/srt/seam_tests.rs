use super::parse_srt_url_for_test;

/// The seam hands back the typed parse, not a rendered line: a caller reads
/// the host/port to dial and the query overrides as fields.
#[test]
fn the_typed_seam_reports_the_dial_address_and_the_query_overrides() {
    let (addr, parsed) =
        parse_srt_url_for_test("srt://[::1]:9001?latency=120").expect("valid caller URL");
    assert_eq!(addr, "[::1]:9001");
    assert_eq!(parsed.host, "[::1]");
    assert_eq!(parsed.port, 9001);
    assert_eq!(parsed.latency, Some(std::time::Duration::from_millis(120)));
    assert_eq!(parsed.stream_id, None);
}

#[test]
fn the_typed_seam_reports_the_policy_rejection_as_its_error() {
    let err = parse_srt_url_for_test("srt://h:9000?mode=listener")
        .expect_err("a listener-mode URL is rejected for a push output");
    assert!(err.to_string().contains("mode=caller"), "{err}");
}
