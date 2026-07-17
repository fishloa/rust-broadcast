//! ACAP entrypoint (device-gated). This is task 4 and requires the `device`
//! feature; it only builds inside the Axis ACAP Native SDK sysroot.
//!
//! This is deliberately a MINIMAL app: it proves the full on-device
//! toolchain end-to-end (cross-compiled Rust, `vdo`/vdo-sys linking,
//! `acap-logging`, `axum`, `cargo-acap-build` .eap packaging) *before* the
//! real VDO-capture -> multimux -> LL-HLS pipeline is written on top of it.
//!
//! It references the `vdo` crate just enough to force the `vdo-sys` native
//! library to link, without calling `.build()` on the stream -- so it never
//! touches real camera hardware -- and serves a trivial HTTP health route
//! via `axum` so a reverse-proxied request from the camera's web UI
//! (`manifest.json`'s `reverseProxy` entry) has something to hit.

use log::info;

#[tokio::main]
async fn main() {
    acap_logging::init_logger();
    info!("acap-multimux: starting minimal device proof-of-pipeline app");

    // Reference the `vdo` crate enough to force `vdo-sys` to link. This is
    // intentionally NOT `.build()`-ed, so it never opens a real camera
    // channel -- it only proves the cross-compiled binary resolves the
    // vdo-sys native symbols inside the ACAP SDK sysroot.
    let _stream_builder = vdo::StreamBuilder::new().format(vdo::VdoFormat::VDO_FORMAT_H264);
    info!("acap-multimux: vdo StreamBuilder(VDO_FORMAT_H264) constructed, vdo-sys linked OK");

    let app =
        axum::Router::new().route("/", axum::routing::get(|| async { "acap-multimux alive" }));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:2999")
        .await
        .expect("failed to bind 127.0.0.1:2999");
    info!("acap-multimux: listening on 127.0.0.1:2999");

    axum::serve(listener, app).await.expect("axum server error");
}
