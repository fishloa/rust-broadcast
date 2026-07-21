//! Guards `examples/*.json` (the multimux hub config examples, issue #663)
//! against config-schema drift: every example must deserialize as a
//! [`multimux::config::Config`] and pass [`multimux::config::Config::validate`],
//! the same two steps `multimux-cli --config <file>` performs at startup
//! ([`multimux::config::Config::from_json_file`]).
//!
//! Fixtures are read via `std::fs` + `CARGO_MANIFEST_DIR` (not
//! `include_str!` of a moving path), matching the workspace's fixture-access
//! convention.

use multimux::config::Config;

fn load(name: &str) -> Config {
    let path = format!("{}/examples/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path}: {e}"))
}

#[test]
fn webcam_fleet_40_is_valid_and_has_forty_routes() {
    let cfg = load("webcam-fleet-40.json");
    assert_eq!(
        cfg.routes.len(),
        40,
        "the 40-webcam scenario names 40 routes"
    );
    cfg.validate().expect("webcam-fleet-40.json must validate");
}

#[test]
fn reverse_proxy_is_valid() {
    let cfg = load("reverse-proxy.json");
    assert_eq!(cfg.routes.len(), 3);
    cfg.validate().expect("reverse-proxy.json must validate");
}

#[test]
fn multi_output_is_valid() {
    let cfg = load("multi-output.json");
    assert_eq!(cfg.routes.len(), 1);
    cfg.validate().expect("multi-output.json must validate");
}

#[test]
fn custom_scheme_is_valid() {
    let cfg = load("custom-scheme.json");
    assert_eq!(cfg.routes.len(), 1);
    cfg.validate().expect("custom-scheme.json must validate");
}
