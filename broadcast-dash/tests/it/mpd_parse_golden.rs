//! Pins the parsed MPD model on every committed MPD fixture (#2355).
//!
//! Every fixture must yield byte-identical `{:#?}` of the parse result (`Ok`
//! model or `Err`) to the file recorded before the model moved here from
//! `dash-runtime`. Without a private checkout only the public rows are
//! compared.

use super::corpus;
use broadcast_dash::Mpd;
use broadcast_dash::error::MpdError;
use broadcast_dash::limits::Limits;

const GOLDEN: &str = "golden/dash/mpd-parse.golden";
const PUBLIC_PREFIX: &str = "fixtures/";
const PRIVATE_PREFIX: &str = "private/";
const REGENERATE: &str = "REGENERATE_MPD_PARSE_GOLDEN";

fn recorded(name: &str, text: &[u8]) -> String {
    let parsed: Result<Mpd, MpdError> = Mpd::parse_with(text, &Limits::default());
    format!("{name}\n{parsed:#?}\n")
}

fn goldens() -> String {
    let mut out = String::new();
    for (name, path) in corpus::corpus() {
        let text = std::fs::read(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        out.push_str(&recorded(&name, &text));
    }
    out
}

#[test]
fn every_fixture_parses_to_the_recorded_model() {
    let got = goldens();
    if std::env::var_os(REGENERATE).is_some() {
        std::fs::write(test_fixtures::path(GOLDEN), &got).expect("write golden");
        return;
    }
    let want = test_fixtures::read_to_string(GOLDEN);
    let have_private = test_fixtures::private_root().is_some();
    let keep =
        |l: &&str| have_private || l.starts_with(PUBLIC_PREFIX) || !l.starts_with(PRIVATE_PREFIX);
    let want: Vec<&str> = want.lines().filter(keep).collect();
    let got: Vec<&str> = got.lines().filter(keep).collect();
    assert!(
        !want.is_empty(),
        "golden is empty; regenerate it with {REGENERATE}=1"
    );
    assert_eq!(got, want, "parsed MPD model changed on the fixture corpus");
}

#[test]
fn a_refused_mpd_records_its_error() {
    let rows = recorded("fixtures/deliberately-unparseable.mpd", b"<MPD/>");
    assert!(rows.contains("Err("), "got {rows}");
}

#[test]
fn the_corpus_is_not_empty() {
    assert!(
        corpus::corpus()
            .iter()
            .any(|(n, _)| n.contains("/manifest.mpd")),
        "the public corpus must include the reference MPD"
    );
}
