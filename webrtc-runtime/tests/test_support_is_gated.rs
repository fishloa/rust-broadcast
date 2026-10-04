//! The fault-injection hooks live behind the non-default `test-support`
//! feature (and `#[doc(hidden)]`): a default build must not contain them.
//! A source scan, because the absence of a symbol cannot be compiled-against.

const HOOKS: &[&str] = &[
    "pending_timer_error",
    "stuck_timer",
    "fn with_certificate_for_test",
    "fn force_next_timer_error",
    "fn force_stuck_timer",
    "fn certificate_fingerprint",
    "use transport::certificate_fingerprint",
];
const GATE: &str = "#[cfg(feature = \"test-support\")]";

#[test]
fn every_hook_item_is_cfg_gated() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for file in ["src/media/transport.rs", "src/media/mod.rs"] {
        let src = std::fs::read_to_string(root.join(file)).unwrap();
        let lines: Vec<&str> = src.lines().collect();
        let mut hits = 0;
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim_start();
            // Bodies of a gated fn are covered by the fn's own gate.
            if t.starts_with("//") || t.starts_with("self.") {
                continue;
            }
            // Only declarations / statements that name a hook; walk back over
            // attribute lines to find the gate.
            if !HOOKS.iter().any(|h| t.contains(h)) {
                continue;
            }
            hits += 1;
            let mut j = i;
            let mut gated = false;
            while j > 0 {
                j -= 1;
                let p = lines[j].trim();
                if p == GATE {
                    gated = true;
                    break;
                }
                if !(p.starts_with("#[") || p.starts_with("///") || p.starts_with("//")) {
                    break;
                }
            }
            // `if self.pending_timer_error...` bodies sit under the gate on
            // the `if`; a line inside such a body is checked via its `if`.
            assert!(gated, "{file}:{}: `{}` is not behind {GATE}", i + 1, t);
        }
        if file.ends_with("transport.rs") {
            assert!(
                hits >= 8,
                "scan found only {hits} hook lines; scanner broken"
            );
        }
    }
}

#[test]
fn test_support_is_not_a_default_feature() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let toml = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let default = toml
        .lines()
        .find(|l| l.trim_start().starts_with("default"))
        .expect("default feature line");
    assert!(!default.contains("test-support"), "{default}");
}
