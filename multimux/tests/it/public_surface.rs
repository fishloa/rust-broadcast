//! Audit 18, finding 12 (#2696): multimux's test-only seams are a feature-gated
//! hub. `src/lib.rs` declares `pub mod test_seam` under `test-seams`/
//! `test-hooks`, and every other module's test seams are reachable only by
//! that hub re-exporting them — a `pub` seam elsewhere would be surface no
//! consumer asked for.

use std::fs;
use std::path::Path;

fn read(name: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(name))
        .expect("read source file")
}

/// The hub is one module, gated on the test features, and the retired
/// `webrtc_ice` seam module is gone.
#[test]
fn the_test_seams_hang_off_one_feature_gated_hub() {
    let lib = read("lib.rs");
    assert!(
        lib.contains(
            "#[cfg(any(feature = \"test-seams\", feature = \"test-hooks\"))]\n\
             #[doc(hidden)]\n\
             pub mod test_seam {"
        ),
        "the test-seam hub must be gated on test-seams/test-hooks"
    );
    assert!(
        !lib.contains("pub mod webrtc_ice"),
        "the old `webrtc_ice` seam module is folded into `test_seam`"
    );
}

/// Every seam the integration tests reach is re-exported from the hub, so no
/// test has to name a production module path to find one.
#[test]
fn the_hub_re_exports_every_seam_the_tests_use() {
    let lib = read("lib.rs");
    let hub = &lib[lib.find("pub mod test_seam {").expect("hub module")..];
    let hub = &hub[..hub.find("\n}\n").expect("hub module end")];
    for seam in [
        "serve_for_test",
        "serve_whip_for_test_with_read_load",
        "serve_rtmp_for_test_with_read_load",
        "serve_for_test_with_read_timeout",
        "serve_for_test_with_read_load_and_timeout",
        "serve_for_test_with_read_load_timeout_and_policy",
        "whip_router_for_test",
        "parse_offer_for_test",
        "render_answer_for_test",
        "restart_fragment_for_test",
        "parse_srt_url_for_test",
        "tc_url_for_test",
        "control_url_for_test",
        "push_sdp_for_test",
        "push_media_for_test",
    ] {
        assert!(
            hub.contains(seam),
            "`{seam}` is not re-exported from the `test_seam` hub: {hub}"
        );
    }
    for seam in ["whep_router_for_test", "render_whep_answer_for_test"] {
        let whep = read("output/whep.rs");
        assert!(whep.contains(seam), "`{seam}` is missing from `output/whep.rs`");
    }
}

/// A seam function's own definition is cfg-gated on a test feature (or
/// `cfg(test)`), so a normal build carries none of them.
#[test]
fn every_seam_definition_is_gated_on_a_test_feature() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    source_files(&src, &mut files);
    let mut gated = 0;
    let mut hits = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path).expect("read source file");
        let mut gate = String::new();
        for (n, line) in text.lines().enumerate() {
            let t = line.trim();
            if t.starts_with("#[cfg(") {
                gate = t.to_string();
            } else if t.contains("pub fn ") && t.contains("_for_test") {
                if gate.contains("test-seams")
                    || gate.contains("test-hooks")
                    || gate == "#[cfg(test)]"
                {
                    gated += 1;
                } else {
                    hits.push(format!("{}:{}: {}", path.display(), n + 1, t));
                }
            } else if !t.starts_with("#[") && !t.starts_with("///") && !t.is_empty() {
                gate.clear();
            }
        }
    }
    assert!(gated > 0, "the scan found no seam definitions to check");
    assert!(
        hits.is_empty(),
        "a test seam is not gated on `test-seams`/`test-hooks`:\n{}",
        hits.join("\n")
    );
}

fn source_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).expect("read src dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            source_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}
