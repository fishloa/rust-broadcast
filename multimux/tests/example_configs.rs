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

#[test]
fn broadcast_origin_is_valid_and_exercises_the_full_surface() {
    let cfg = load("broadcast-origin.json");
    assert_eq!(cfg.routes.len(), 8, "eight sources");

    // The point of this example is breadth: if a variant is dropped from the
    // config it stops being the "complex origin" example and nobody notices.
    let mut inputs: Vec<_> = cfg
        .routes
        .iter()
        .map(|r| format!("{:?}", std::mem::discriminant(&r.input)))
        .collect();
    inputs.sort();
    inputs.dedup();
    assert_eq!(
        inputs.len(),
        7,
        "seven distinct ingest transports (rtsp x2 share one variant)"
    );

    let mut outputs: Vec<_> = cfg
        .routes
        .iter()
        .flat_map(|r| r.outputs.iter())
        .map(|o| format!("{:?}", std::mem::discriminant(o)))
        .collect();
    outputs.sort();
    outputs.dedup();
    assert_eq!(outputs.len(), 3, "llhls + dash + ll_dash");

    assert!(
        cfg.output_auth.is_some(),
        "signed-URL egress auth is part of what this example demonstrates"
    );

    // The admin API must bind loopback here. It can add and remove routes, so
    // an example showing it on 0.0.0.0 would be teaching the wrong thing.
    let admin = cfg.admin.as_ref().expect("admin API configured");
    assert!(
        admin.bind.starts_with("127.0.0.1:") || admin.bind.starts_with("localhost:"),
        "admin listener must be loopback in the example, got {}",
        admin.bind
    );
    assert_ne!(admin.bind, cfg.bind, "admin must not share the media port");

    cfg.validate().expect("broadcast-origin.json must validate");
}
