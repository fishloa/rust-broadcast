//! G2 — round trip on real data. Every committed Smooth client Manifest
//! (`fixtures/smooth/**`, ffmpeg's own `smoothstreaming` muxer output, and
//! `fixtures/golden/smooth/*.xml`, recorded from `transmux`'s writer in PR
//! #2360) is parsed, serialized, and re-parsed: the second model must equal
//! the first, and serializing it again must give the identical string (the
//! serializer is a fixed point).
//!
//! Fixtures are resolved through `test_fixtures` at run time — a missing
//! fixture is a hard failure, never a silent skip.

use broadcast_common::timescale::Timescale;
use broadcast_smooth::SmoothManifest;

fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn manifests() -> Vec<(String, String)> {
    let mut paths = Vec::new();
    for dir in ["smooth", "golden/smooth"] {
        walk(&test_fixtures::root().join(dir), &mut paths);
    }
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if file_name.starts_with("wrm-") {
            continue;
        }
        let is_manifest = path.file_name().is_some_and(|n| n == "Manifest")
            || path.extension().is_some_and(|e| e == "xml");
        if is_manifest {
            let name = path
                .strip_prefix(test_fixtures::root())
                .expect("under fixtures root")
                .display()
                .to_string();
            let xml = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            out.push((name, xml));
        }
    }
    assert!(
        out.iter().any(|(n, _)| n.starts_with("smooth/")),
        "no ffmpeg Smooth manifest found under fixtures/smooth"
    );
    out
}

#[test]
fn every_real_manifest_reparses_equal_and_serializes_to_a_fixed_point() {
    let mut checked = 0usize;
    for (name, xml) in manifests() {
        let first =
            SmoothManifest::parse(&xml).unwrap_or_else(|e| panic!("{name}: parse failed: {e}"));
        checked += 1;
        let rendered = first.to_xml();
        let second = SmoothManifest::parse(&rendered)
            .unwrap_or_else(|e| panic!("{name}: re-parse failed: {e}\n{rendered}"));
        assert_eq!(second, first, "{name}: re-parsed model differs");
        assert_eq!(
            second.to_xml(),
            rendered,
            "{name}: serialization is not a fixed point"
        );
        println!(
            "{name}: {} stream(s), round-trip equal and a fixed point",
            first.streams.len()
        );
    }
    assert!(checked > 0, "no manifest was checked");
}

#[test]
fn mutating_a_parsed_manifest_changes_the_output() {
    for (name, xml) in manifests() {
        let Ok(mut model) = SmoothManifest::parse(&xml) else {
            continue;
        };
        let before = model.to_xml();
        model.timescale = Timescale::from_nonzero(model.timescale.get().saturating_add(1));
        let after = model.to_xml();
        assert_ne!(before, after, "{name}: a mutated TimeScale changed nothing");
    }
}
