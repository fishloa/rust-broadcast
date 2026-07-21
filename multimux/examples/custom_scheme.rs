//! Register a custom input scheme with **zero multimux edits** (issue #663
//! external scheme plugin registry).
//!
//! A third-party crate that wants to feed multimux from a transport this
//! crate doesn't know about (WebRTC, a proprietary camera SDK, ...) writes
//! its own [`multimux::SourceConnector`], registers a factory for it under a
//! `type_tag` in a [`multimux::SchemeRegistry`], and names that tag in a
//! config's `InputSpec::Custom` — no fork, no PR against this crate.
//!
//! This example is deliberately minimal and **never calls
//! [`multimux::serve`]/[`multimux::serve_with_registry`]** (which block
//! forever serving HTTP): it proves the two things a real integration relies
//! on — the factory is found by its registered tag, and a config naming that
//! tag parses/validates — without needing a tokio runtime at all (the
//! registered factory is never actually invoked here).
//!
//! `examples/custom-scheme.json` is the on-disk counterpart of the inline
//! JSON built below: the same single-route config (a `"silence"`-tagged
//! `InputSpec::Custom`) as a standalone file, for a `multimux-cli --config`
//! invocation naming this exact scheme.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example custom_scheme
//! ```

use std::sync::Arc;

use multimux::config::{Config, InputSpec};
use multimux::pipeline::SampleSource;
use multimux::{Backoff, InputCtx, SchemeRegistry, SourceConnector};
use transmux::pipeline::{Sample, TrackSpec};

/// A trivial [`SampleSource`] standing in for a real external ingest
/// transport: no tracks, ends immediately (`Ok(None)`) on its first poll. A
/// genuine external scheme (e.g. WebRTC) would instead depayload real media
/// here.
struct SilenceSource;

impl SampleSource for SilenceSource {
    fn track_specs(&self) -> Vec<TrackSpec> {
        Vec::new()
    }

    async fn next_samples(&mut self) -> multimux::Result<Option<Vec<(u32, Sample)>>> {
        Ok(None)
    }
}

/// The [`SourceConnector`] a `"silence"`-tagged `InputSpec::Custom` route
/// resolves to, once registered below.
struct SilenceConnector;

impl SourceConnector for SilenceConnector {
    type Source = SilenceSource;

    async fn connect(&self) -> multimux::Result<Self::Source> {
        Ok(SilenceSource)
    }
}

/// Builds a [`SchemeRegistry`] with one custom input scheme (`"silence"`)
/// registered — exactly what an external crate would do in its own code to
/// add a new ingest transport without editing multimux. The factory
/// constructs its own concrete `SilenceConnector` and spawns the
/// re-exported [`multimux::supervise`] itself (see [`multimux::registry`]'s
/// module docs for why: `SourceConnector` is not object-safe, so a factory
/// can't just return a connector — it has to erase the type at the closure
/// boundary by spawning the supervised task directly).
fn build_registry() -> SchemeRegistry {
    let mut registry = SchemeRegistry::new();
    registry.register_input(
        "silence",
        Arc::new(|ctx: InputCtx| {
            Ok(tokio::spawn(multimux::supervise(
                SilenceConnector,
                ctx.store,
                ctx.target_duration_secs,
                ctx.part_target_ms,
                Backoff::production_default(),
                ctx.name,
                ctx.shutdown_rx,
            )))
        }),
    );
    registry
}

fn main() {
    let registry = build_registry();

    // The registered tag resolves; anything else doesn't — the same lookup
    // `serve_with_registry` performs for an `InputSpec::Custom` route.
    assert!(registry.input("silence").is_some());
    assert!(registry.input("nope").is_none());

    // A config naming the registered scheme parses like any real config
    // would; `Config::validate` always accepts a `Custom` input structurally
    // (the registry — not `validate` — is what would reject a bad `params`,
    // at route-build time inside `serve_with_registry`).
    let json = r#"{
        "routes": [
            {
                "name": "cam1",
                "input": { "type": "custom", "type_tag": "silence", "params": {} }
            }
        ]
    }"#;
    let config: Config = serde_json::from_str(json).expect("valid JSON");
    config
        .validate()
        .expect("a Custom input is always structurally valid");
    match &config.routes[0].input {
        InputSpec::Custom { type_tag, .. } => assert_eq!(type_tag, "silence"),
        other => panic!("expected InputSpec::Custom, got {other:?}"),
    }

    println!(
        "custom_scheme: registered a \"silence\" input scheme with zero multimux edits; \
         a config naming it parsed and validated."
    );
}
