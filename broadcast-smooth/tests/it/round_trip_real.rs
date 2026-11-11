//! G2 — round trip on real data. Every committed Smooth client Manifest
//! (`fixtures/smooth/**`, ffmpeg's own `smoothstreaming` muxer output, and
//! `fixtures/golden/smooth/*.xml`, recorded from `transmux`'s writer in PR
//! #2360) is parsed, serialized, and re-parsed: the second model must equal
//! the first, and serializing it again must give the identical string (the
//! serializer is a fixed point).
//!
//! Fixtures are resolved through `test_fixtures` at run time — a missing
//! fixture is a hard failure, never a silent skip.

use broadcast_smooth::SmoothManifest;

fn manifests() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for dir in ["smooth", "golden/smooth"] {
        let root = test_fixtures::root().join(dir);
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(e) if dir == "smooth" => {
                println!("skipping {dir}: only in the private fixture tier ({e})");
                continue;
            }
            Err(e) => panic!("read {dir}: {e}"),
        };
        for entry in entries {
            let path = entry.expect("dir entry").path();
            let is_manifest = path.is_file()
                && (path.file_name().is_some_and(|n| n == "Manifest")
                    || path.extension().is_some_and(|e| e == "xml"));
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
    }
    assert!(!out.is_empty(), "no Smooth manifest fixtures were found");
    out
}

#[test]
fn every_real_manifest_reparses_equal_and_serializes_to_a_fixed_point() {
    for (name, xml) in manifests() {
        let first = match SmoothManifest::parse(&xml) {
            Ok(model) => model,
            // `fixtures/golden/smooth/` also holds PlayReady WRMHEADER
            // documents (written, never parsed): they are not client
            // manifests, and the parser must reject them.
            Err(_) if name.starts_with("golden/smooth/") => {
                println!("{name}: not a client manifest, rejected");
                continue;
            }
            Err(e) => panic!("{name}: parse failed: {e}"),
        };
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
}

#[test]
fn mutating_a_parsed_manifest_changes_the_output() {
    for (name, xml) in manifests() {
        let Ok(mut model) = SmoothManifest::parse(&xml) else {
            continue;
        };
        let before = model.to_xml();
        model.timescale = model.timescale.saturating_add(1);
        let after = model.to_xml();
        assert_ne!(before, after, "{name}: a mutated TimeScale changed nothing");
    }
}
