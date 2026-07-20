//! Serve a single live RTSP source as LL-HLS and/or DASH.
//!
//! Builds a single-route [`multimux::config::Config`] from an RTSP URL given
//! on the command line and hands it to [`multimux::origin::serve`] — the
//! same entrypoint the `multimux` CLI binary drives, minus the argument
//! parsing.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example serve_rtsp -- rtsp://cam.local/stream
//! # then, in another terminal:
//! curl http://0.0.0.0:8080/cam/master.m3u8
//!
//! # LL-HLS + DASH from the same ingest (issue #663 P4) via --outputs (or
//! # the --dash shorthand for "both"):
//! cargo run --example serve_rtsp -- rtsp://cam.local/stream --outputs llhls,dash
//! cargo run --example serve_rtsp -- rtsp://cam.local/stream --dash
//! curl http://0.0.0.0:8080/cam/manifest.mpd
//! ```

use multimux::config::{Config, InputSpec, Route};
use multimux::output::OutputKind;

/// Served stream name for the single route this example configures.
const STREAM_NAME: &str = "cam";

/// Parse `--outputs llhls,dash` / the `--dash` shorthand (equivalent to
/// `--outputs llhls,dash`) out of the trailing args; defaults to LL-HLS only
/// (matching [`multimux::config::Route::outputs`]'s own default) when
/// neither flag is given.
fn parse_outputs(args: &[String]) -> Vec<OutputKind> {
    if args.iter().any(|a| a == "--dash") {
        return vec![OutputKind::LlHls, OutputKind::Dash];
    }
    if let Some(pos) = args.iter().position(|a| a == "--outputs") {
        let list = args
            .get(pos + 1)
            .unwrap_or_else(|| panic!("--outputs requires a value, e.g. --outputs llhls,dash"));
        return list
            .split(',')
            .map(|tok| match tok.trim() {
                "llhls" => OutputKind::LlHls,
                "dash" => OutputKind::Dash,
                other => panic!("unknown output kind {other:?} (expected llhls or dash)"),
            })
            .collect();
    }
    vec![OutputKind::LlHls]
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rtsp_url = args
        .first()
        .cloned()
        .expect("usage: serve_rtsp <rtsp://host/stream> [--dash | --outputs llhls,dash]");
    let outputs = parse_outputs(&args[1..]);

    let config = Config {
        routes: vec![Route {
            name: STREAM_NAME.to_string(),
            input: InputSpec::Rtsp {
                url: rtsp_url,
                auth: None,
            },
            outputs,
        }],
        ..Config::default()
    };
    config.validate().expect("valid config");

    println!(
        "multimux serve_rtsp: http://{}/{STREAM_NAME}/master.m3u8",
        config.bind
    );
    if config.routes[0].outputs.contains(&OutputKind::Dash) {
        println!(
            "multimux serve_rtsp: http://{}/{STREAM_NAME}/manifest.mpd",
            config.bind
        );
    }

    multimux::origin::serve(config)
        .await
        .expect("origin server");
}
