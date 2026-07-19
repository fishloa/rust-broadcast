//! Serve a single live RTSP source as LL-HLS.
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
//! ```

use multimux::config::{Config, InputSpec, Route};

/// Served stream name for the single route this example configures.
const STREAM_NAME: &str = "cam";

#[tokio::main]
async fn main() {
    let rtsp_url = std::env::args()
        .nth(1)
        .expect("usage: serve_rtsp <rtsp://host/stream>");

    let config = Config {
        routes: vec![Route {
            name: STREAM_NAME.to_string(),
            input: InputSpec::Rtsp { url: rtsp_url },
        }],
        ..Config::default()
    };
    config.validate().expect("valid config");

    println!(
        "multimux serve_rtsp: http://{}/{STREAM_NAME}/master.m3u8",
        config.bind
    );

    multimux::origin::serve(config)
        .await
        .expect("origin server");
}
