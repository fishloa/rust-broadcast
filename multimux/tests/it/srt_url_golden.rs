//! The `srt://` URL parse table (issue #2369), compared byte for byte with
//! the golden recorded in `fixtures/golden/srt/push-url-table.txt`.

use std::fmt::Write as _;

/// The `srt://` URL shapes the push accepts or rejects (push policy:
/// Caller only, no passphrase).
const SRT_URL_CASES: &[&str] = &[
    "example.com",
    "srt://example.com",
    "example.com:9001",
    "srt://example.com:9001",
    "[::1]",
    "srt://[::1]:9000",
    "::1",
    "::1:9000",
    "srt://::1",
    "srt://user:key@host:9000",
    "srt://user@host:9000",
    "srt://h:9000?streamid=#!::r=live/cam,m=publish",
    "srt://h:9000?streamid=%23!::r=live%2Fcam%2Cm=publish",
    "srt://h:9000?streamid=100%",
    "srt://h:9000?latency=0",
    "srt://h:9000?latency=200",
    "srt://h:9000?latency=8000",
    "srt://h:9000?latency=8001",
    "srt://h:9000?latency=99999",
    "srt://h:9000?latency=notanumber",
    "srt://h:9000?mode=caller",
    "srt://h:9000?mode=listener",
    "srt://h:9000?passphrase=secret",
    "srt://h:9000?unknownkey=value",
    "srt://h:9000/",
    "srt://h:9000?",
    "srt://h:9000?streamid=live&",
    "",
    "srt://",
    ":9000",
    "host:notaport",
    "host:99999",
];

#[test]
fn srt_push_url_table_matches_the_recorded_golden() {
    let mut record = String::new();
    for case in SRT_URL_CASES {
        let outcome = match multimux::test_seam::parse_srt_url_for_test(case) {
            Ok((addr, parsed)) => format!(
                "ok addr={addr} stream_id={:?} latency_ms={:?}",
                parsed.stream_id,
                parsed.latency.map(|l| l.as_millis())
            ),
            Err(e) => format!("err {e}"),
        };
        let _ = writeln!(record, "{case}\t{outcome}");
    }

    let path = "golden/srt/push-url-table.txt";
    let golden = test_fixtures::path(path);
    if std::env::var_os("REGENERATE_PUSH_URL_TABLE").is_some() {
        std::fs::write(&golden, &record).expect("write golden");
        return;
    }
    let expected = test_fixtures::read_to_string(path);
    assert_eq!(
        record, expected,
        "the srt:// URL parse table changed; if that is intended, regenerate \
         {path} with a reviewed diff"
    );
}
