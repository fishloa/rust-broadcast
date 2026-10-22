//! `SegmentTemplate::match_url` is the exact inverse of `SegmentTemplate::resolve`
//! (ISO/IEC 23009-1 §5.3.9.4.4 identifier grammar; issue: DASH-IF ingest P1).
//! The receiver of a DASH-IF Live Media Ingest POST has only the request path
//! and must recover which Representation/segment it names.

use transmux::dash_parse::SegmentTemplate;

const IDS: &[&str] = &["video", "audio-en", "v1", "1080p", "7"];

const TEMPLATES: &[&str] = &[
    "$RepresentationID$/seg-$Number$.m4s",
    "chunk-$RepresentationID$-$Number%05d$.m4s",
    "$RepresentationID$_$Time$.m4s",
    "init-$RepresentationID$.m4s",
    "$RepresentationID$-$Bandwidth$-$Number$.m4s",
    "a$$b-$RepresentationID$-$Time%09d$.m4s",
];

const VALUES: &[u64] = &[0, 1, 7, 42, 99_999, 100_000, 123_456_789, u64::MAX];

fn m(t: &str, c: &str, ids: &[&str]) -> Option<(String, Option<u64>, Option<u64>, Option<u64>)> {
    SegmentTemplate::match_url(t, c, ids)
        .map(|x| (x.representation_id, x.number, x.time, x.bandwidth))
}

/// resolve -> match_url returns exactly the inputs that were substituted, for a
/// grid of templates x ids x values. The templates all carry a literal
/// separator next to every digit run, so each URL has exactly one reading.
#[test]
fn match_inverts_resolve_over_a_grid() {
    let mut checked = 0usize;
    for t in TEMPLATES {
        let has_number = t.contains("$Number");
        let has_time = t.contains("$Time");
        let has_bw = t.contains("$Bandwidth");
        for id in IDS {
            for &n in VALUES {
                let number = has_number.then_some(n);
                let time = has_time.then_some(n.wrapping_add(3));
                let bw = has_bw.then_some(n / 2 + 1);
                let url = SegmentTemplate::resolve(t, id, number, time, bw);
                let got = m(t, &url, IDS).unwrap_or_else(|| panic!("no match: {t:?} <- {url:?}"));
                assert_eq!(
                    got,
                    (id.to_string(), number, time, bw),
                    "template {t:?} url {url:?}"
                );
                checked += 1;
            }
        }
    }
    assert_eq!(checked, TEMPLATES.len() * IDS.len() * VALUES.len());
}

#[test]
fn zero_padded_width_is_exact() {
    let t = "s-$Number%05d$.m4s";
    assert_eq!(
        m(t, "s-00042.m4s", &[]),
        Some((String::new(), Some(42), None, None))
    );
    // Too short for the width, or padded beyond what `%05d` would render.
    assert_eq!(m(t, "s-42.m4s", &[]), None);
    assert_eq!(m(t, "s-0000042.m4s", &[]), None);
    // A value wider than the width renders unpadded and matches.
    assert_eq!(
        m(t, "s-1234567.m4s", &[]),
        Some((String::new(), Some(1_234_567), None, None))
    );
}

#[test]
fn default_width_has_no_leading_zeros() {
    let t = "s-$Number$.m4s";
    assert_eq!(
        m(t, "s-0.m4s", &[]),
        Some((String::new(), Some(0), None, None))
    );
    assert_eq!(m(t, "s-007.m4s", &[]), None);
    assert_eq!(m(t, "s-00.m4s", &[]), None);
}

#[test]
fn rejects_what_resolve_could_not_have_produced() {
    let t = "$RepresentationID$/seg-$Number$.m4s";
    // Wrong literal / trailing garbage / unknown representation.
    assert_eq!(m(t, "video/segx-5.m4s", IDS), None);
    assert_eq!(m(t, "video/seg-5.m4s.bak", IDS), None);
    assert_eq!(m(t, "nope/seg-5.m4s", IDS), None);
    // A sign is not a digit; whitespace is not a digit; empty digit run.
    assert_eq!(m(t, "video/seg--5.m4s", IDS), None);
    assert_eq!(m(t, "video/seg-+5.m4s", IDS), None);
    assert_eq!(m(t, "video/seg- 5.m4s", IDS), None);
    assert_eq!(m(t, "video/seg-.m4s", IDS), None);
    // u64 overflow (u64::MAX + 1).
    assert_eq!(m(t, "video/seg-18446744073709551616.m4s", IDS), None);
    assert_eq!(
        m(t, "video/seg-18446744073709551615.m4s", IDS),
        Some(("video".into(), Some(u64::MAX), None, None))
    );
}

#[test]
fn dollar_escape_and_malformed_templates() {
    assert_eq!(
        m("a$$b-$Number$", "a$b-3", &[]),
        Some((String::new(), Some(3), None, None))
    );
    assert_eq!(m("a$$b-$Number$", "ab-3", &[]), None);
    // The escape consumes exactly a literal `$`, not any character.
    assert_eq!(m("a$$b-$Number$", "axb-3", &[]), None);
    // An unescaped `$` that does not close an identifier: undefined => no match.
    assert_eq!(m("seg-$Number", "seg-3", &[]), None);
}

/// `$RepresentationID$$Number$` with ids that end in digits is ambiguous
/// (`v1`+`23` vs `v`+`123`): §6.2.16.a recommends a non-digit separator, and
/// the matcher refuses to guess.
#[test]
fn ambiguous_reading_is_refused() {
    let t = "$RepresentationID$$Number$.m4s";
    assert_eq!(m(t, "v123.m4s", &["v", "v1"]), None);
    // With only one candidate id the reading is unique.
    assert_eq!(
        m(t, "v123.m4s", &["v"]),
        Some(("v".into(), Some(123), None, None))
    );
    assert_eq!(
        m(t, "v123.m4s", &["v1"]),
        Some(("v1".into(), Some(23), None, None))
    );
}

#[test]
fn unicode_and_boundary_inputs_never_panic() {
    let t = "$RepresentationID$/s-$Number$.m4s";
    for c in [
        "",
        "é",
        "video/é",
        "video/s-é.m4s",
        "日本語",
        "video/s-1.m4s\u{0}",
    ] {
        let _ = SegmentTemplate::match_url(t, c, IDS);
    }
}
