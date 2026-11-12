use broadcast_dash::Mpd;
use broadcast_dash::limits::Limits;

use super::corpus;

const MIN_VALIDATED: usize = 30;

#[test]
fn every_valid_corpus_mpd_still_validates_after_parse_and_write() {
    let Some(schema) = test_fixtures::process::xsd::dash_mpd_schema("mpd_xsd") else {
        return;
    };
    let mut checked = 0usize;
    let mut failures = Vec::new();
    for (name, path) in corpus::corpus() {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        if test_fixtures::process::xsd::validate(&schema, "source", &text).is_err() {
            continue;
        }
        let Ok(mpd) = Mpd::parse_with(text.as_bytes(), &Limits::default()) else {
            continue;
        };
        let xml = mpd.to_xml().unwrap_or_else(|e| panic!("{name}: {e}"));
        checked += 1;
        if let Err(e) = test_fixtures::process::xsd::validate(&schema, "written", &xml) {
            failures.push(format!("{name}:\n{e}"));
        }
    }
    assert!(checked >= MIN_VALIDATED, "validated only {checked} sources");
    assert!(
        failures.is_empty(),
        "{} of {checked} written MPDs fail the schema:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
