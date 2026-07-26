//! Replay a captured RTMP publish through the sans-IO [`ServerSession`] and
//! emit the resulting FLV — no socket, no tokio, pure `rtmp_runtime` public
//! API (mirrors `tests/ingest_fixture.rs`, but as a runnable example rather
//! than an assertion).
//!
//! Reads `tests/fixtures/obs-publish.bin` (a real `ffmpeg -f flv` publish,
//! see `tests/fixtures/PROVENANCE.md`) via `std::fs`, feeds every byte to
//! [`ServerSession::handle_data`], concatenates each [`ServerEvent::Media`]
//! tag in arrival order, and writes the resulting FLV byte stream to stdout.
//! A one-line summary per event (Connected/Publish/Media size/Eof) goes to
//! stderr so piping stdout to a file yields a valid FLV:
//!
//! ```text
//! cargo run -p rtmp-runtime --example ingest_to_flv > out.flv
//! ```

use rtmp_runtime::server::{ServerEvent, ServerSession};
use std::io::Write;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/obs-publish.bin"
);

fn main() -> std::io::Result<()> {
    let input = std::fs::read(FIXTURE)?;

    let mut session = ServerSession::with_defaults();
    let (_out, events) = session
        .handle_data(&input)
        .expect("replay of a committed fixture must not error");

    let mut flv = Vec::new();
    let mut media_count = 0usize;
    for event in &events {
        match event {
            ServerEvent::Connected { app } => {
                eprintln!("ingest_to_flv: Connected {{ app: {app:?} }}");
            }
            ServerEvent::Publish {
                app,
                stream_key,
                stream_id,
            } => {
                eprintln!(
                    "ingest_to_flv: Publish {{ app: {app:?}, stream_key: {stream_key:?}, stream_id: {stream_id} }}"
                );
            }
            ServerEvent::Media { flv: tag } => {
                media_count += 1;
                flv.extend_from_slice(tag);
            }
            ServerEvent::Eof => {
                eprintln!("ingest_to_flv: Eof");
            }
            other => {
                eprintln!("ingest_to_flv: unrecognised event: {other:?}");
            }
        }
    }
    eprintln!(
        "ingest_to_flv: {media_count} Media event(s), {} total FLV bytes",
        flv.len()
    );

    std::io::stdout().write_all(&flv)
}
