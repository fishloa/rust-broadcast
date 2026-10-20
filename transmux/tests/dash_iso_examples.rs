//! The ISO/IEC 23009-1:2026 (6th edition) example MPDs as an independent parse corpus.
//!
//! `private/specs/iso_iec_23009-1_dash_2026_ed6_schema/` ships 41 example MPDs written by the
//! standard's editors (electronic inserts of the standard, kept private). Every one validates
//! against the same directory's `DASH-MPD.xsd`, so they are an oracle that is NOT our own
//! writer round-tripping itself. For each example this test asserts that
//! [`transmux::dash_parse::Mpd::parse`] accepts it and that the number of `Period`,
//! `AdaptationSet` and `Representation` elements it extracts equals an independent count of
//! those start tags (quick-xml, no shared code with the parser).
//!
//! One example is a documented gap: `example_G27.mpd` uses the 6th edition's `SegmentTimeline`
//! `Pattern` element and `S` entries without `@d` (`<S t=".." r=".." p=".." pE=".."/>`). The
//! XSD gives the syntax only; the semantics are in the paywalled 2026 text, which we do not
//! hold, so the parser must keep rejecting it (loudly, with `@d` named) rather than guess.
//! The expectation is pinned so that adding pattern support makes this test fail and forces
//! the transcription first.
#![cfg(feature = "std")]

use quick_xml::Reader;
use quick_xml::events::Event;
use transmux::dash_parse::Mpd;

/// 6th-edition-only examples whose semantics we cannot implement without the paywalled text.
const UNSUPPORTED_SIXTH_EDITION: &[&str] = &["example_G27.mpd"];

fn schema_dir() -> Option<std::path::PathBuf> {
    // `private_root()` honours SKIP_PRIVATE_FIXTURES=1 and the checkout rules of
    // test_fixtures (it also resolves a linked worktree's main checkout).
    let Some(root) = test_fixtures::private_root() else {
        eprintln!("SKIPPED dash_iso_examples: private repo not available");
        return None;
    };
    let dir = root.join("specs/iso_iec_23009-1_dash_2026_ed6_schema");
    if dir.join("DASH-MPD.xsd").is_file() {
        Some(dir)
    } else {
        panic!(
            "dash_iso_examples: private repo is checked out but {} is missing",
            dir.display()
        );
    }
}

/// `(periods, adaptation_sets, representations)` counted straight from the XML.
fn independent_counts(xml: &str) -> (usize, usize, usize) {
    let mut reader = Reader::from_str(xml);
    let (mut periods, mut sets, mut reps) = (0, 0, 0);
    loop {
        match reader.read_event().expect("well-formed XML") {
            Event::Start(e) | Event::Empty(e) => match e.name().as_ref() {
                "Period" => periods += 1,
                "AdaptationSet" => sets += 1,
                "Representation" => reps += 1,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    (periods, sets, reps)
}

fn examples() -> Option<Vec<(String, String)>> {
    let dir = schema_dir()?;
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("read schema dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) == Some("mpd") {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            out.push((name, std::fs::read_to_string(&path).expect("read example")));
        }
    }
    out.sort();
    Some(out)
}

#[test]
fn every_supported_iso_example_parses_with_the_independent_element_counts() {
    let Some(all) = examples() else { return };
    assert!(
        all.len() >= 40,
        "expected the 41 ISO examples, found {}",
        all.len()
    );
    let mut checked = 0;
    for (name, xml) in &all {
        if UNSUPPORTED_SIXTH_EDITION.contains(&name.as_str()) {
            continue;
        }
        let mpd = Mpd::parse(xml).unwrap_or_else(|e| panic!("{name}: {e}"));
        let (periods, sets, reps) = independent_counts(xml);
        let got_sets: usize = mpd.periods.iter().map(|p| p.adaptation_sets.len()).sum();
        let got_reps: usize = mpd
            .periods
            .iter()
            .flat_map(|p| &p.adaptation_sets)
            .map(|a| a.representations.len())
            .sum();
        assert_eq!(mpd.periods.len(), periods, "{name}: Period count");
        assert_eq!(got_sets, sets, "{name}: AdaptationSet count");
        assert_eq!(got_reps, reps, "{name}: Representation count");
        checked += 1;
    }
    assert!(checked >= 39, "only {checked} examples were checked");
}

#[test]
fn the_sixth_edition_segment_timeline_pattern_example_is_rejected_not_guessed() {
    let Some(all) = examples() else { return };
    for name in UNSUPPORTED_SIXTH_EDITION {
        let (_, xml) = all
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("{name} missing from the corpus"));
        let err = Mpd::parse(xml).expect_err("pattern timelines are not implemented");
        assert!(
            err.to_string().contains("@d"),
            "{name}: expected the missing-@d rejection, got: {err}"
        );
    }
}
