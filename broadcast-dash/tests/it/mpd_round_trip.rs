use broadcast_dash::Mpd;
use broadcast_dash::limits::Limits;

use super::corpus;

#[test]
fn every_parsed_fixture_round_trips_through_the_writer() {
    let mut count = 0usize;
    for (name, path) in corpus::corpus() {
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        let Ok(mpd) = Mpd::parse_with(&bytes, &Limits::default()) else {
            continue;
        };
        let xml = mpd.to_xml().unwrap_or_else(|e| panic!("{name}: {e}"));
        let again = Mpd::parse_with(xml.as_bytes(), &Limits::default())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(again, mpd, "{name}");
        count += 1;
    }
    assert!(count >= 20, "round-tripped only {count} fixtures");
}

#[test]
fn writer_emits_mutated_fields() {
    let bytes = std::fs::read(test_fixtures::path("dash/annex-g/example-F1.mpd")).unwrap();
    let mut mpd = Mpd::parse_with(&bytes, &Limits::default()).unwrap();
    // mutate a Representation bandwidth
    mpd.periods[0].adaptation_sets[0].representations[0].bandwidth += 1;
    // mutate a Period id
    mpd.periods[0].id = Some("mutated-period".to_string());
    let original_bytes = bytes.clone();
    let original = Mpd::parse_with(&original_bytes, &Limits::default()).unwrap();
    let xml = mpd.to_xml().unwrap();
    let again = Mpd::parse_with(xml.as_bytes(), &Limits::default()).unwrap();
    assert_ne!(again, original);
}
