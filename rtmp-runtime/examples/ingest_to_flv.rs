//! Replay a captured RTMP publish through the sans-IO [`ServerSession`] and
//! emit the resulting FLV — no socket, no tokio, pure `rtmp_runtime` public
//! API (mirrors `transmux/tests/it/rtmp_ingest_fixture.rs`, but as a runnable example rather
//! than an assertion).
//!
//! Reads `../fixtures/rtmp/obs-publish.bin` (a real `ffmpeg -f flv` publish,
//! see `../fixtures/rtmp/PROVENANCE.md`) in bounded chunks, feeds each to
//! [`ServerSession::handle_data`], and writes each [`ServerEvent::Media`] tag
//! to stdout as it arrives, so the capture is never held whole.
//! A one-line summary per event (Connected/Publish/Media size/Eof) goes to
//! stderr so piping stdout to a file yields a valid FLV:
//!
//! ```text
//! cargo run -p rtmp-runtime --example ingest_to_flv > out.flv
//! ```

use rtmp_runtime::server::{ServerEvent, ServerSession};
use std::io::{self, BufReader, BufWriter, Read, Write};

const FIXTURE: &str = "rtmp/obs-publish.bin";

/// Read granularity for replaying the capture.
const READ_CHUNK_LEN: usize = 64 * 1024;

fn main() -> io::Result<()> {
    let file = std::fs::File::open(test_fixtures::path(FIXTURE))?;
    let mut reader = BufReader::with_capacity(READ_CHUNK_LEN, file);
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    let mut session = ServerSession::with_defaults();
    let mut chunk = vec![0u8; READ_CHUNK_LEN];
    let mut media_count = 0usize;
    let mut written = 0usize;
    loop {
        let read = read_chunk(&mut reader, &mut chunk)?;
        if read == 0 {
            break;
        }
        let (_out, events) = session
            .handle_data(&chunk[..read])
            .expect("replay of a committed fixture must not error");
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
                    out.write_all(tag)?;
                    written += tag.len();
                }
                ServerEvent::Eof => {
                    eprintln!("ingest_to_flv: Eof");
                }
                other => {
                    eprintln!("ingest_to_flv: unrecognised event: {other:?}");
                }
            }
        }
    }
    eprintln!("ingest_to_flv: {media_count} Media event(s), {written} total FLV bytes");
    out.flush()
}

/// One `read`, retried on `Interrupted`.
fn read_chunk(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        match reader.read(buf) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            other => return other,
        }
    }
}
