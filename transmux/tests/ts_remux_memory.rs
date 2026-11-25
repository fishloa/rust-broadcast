//! Streaming TS remux memory bound (Phase 4, #953/#1024): a generated 1 GiB
//! MPEG-2 TS demuxed and re-muxed sample by sample must stay under a fixed,
//! small live-allocation budget.
//!
//! The generator writes the file pass by pass ([`write_big_ts`]), and this
//! test reads it back in fixed chunks: `StreamingTsDemux` hands over one sample
//! at a time, the TS muxer stage emits whole TS packets (dropped here), and
//! nothing holds the input or the output. `test_alloc::ThreadCapped` makes that
//! a hard gate rather than a measurement: an allocation that would push this
//! thread's live bytes over [`BUDGET`] returns null, so a regression aborts
//! here instead of swapping the machine.
//!
//! The generator is soak-gated: writing and re-reading a gigabyte takes far
//! longer than a normal unit-test budget, so it runs under
//! `RUST_BROADCAST_SOAK=1` (the nightly workflow) and reports its skip loudly
//! otherwise. `RUST_BROADCAST_TS_REMUX_BYTES` overrides the size for a quick
//! local run.
#![cfg(feature = "std")]

use std::io::Read;

use broadcast_common::Stage;
use broadcast_common::stage::Timestamp;
use test_alloc::ThreadCapped;
use transmux::mux_stage::{MuxInput, MuxOutput};
use transmux::{DEFAULT_PSI_INTERVAL, DemuxEvent, StreamingTsDemux, TsMuxStage};

/// Live-allocation budget for one thread: the fixture pass the generator holds
/// (~0.9 MiB), one 1 MiB read buffer, the demuxer's per-PID carriers, one
/// sample in flight and the muxer's one-interleave-window of packets.
/// Measured while developing: 8 MiB passes, 4 MiB aborts (the 901 KiB pass
/// buffer alone). 16 MiB leaves headroom for allocator overhead while a
/// whole-input or whole-output regression — which needs a gigabyte — cannot
/// hide under it.
const BUDGET: usize = 16 << 20;

#[global_allocator]
static ALLOC: ThreadCapped<BUDGET> = ThreadCapped::new();

/// Default remux size: one gibibyte.
const DEFAULT_REMUX_BYTES: u64 = 1 << 30;

/// Read-buffer size: one mebibyte, small enough that the input is never held
/// and large enough that the loop is not syscall-bound.
const READ_CHUNK: usize = 1 << 20;

fn remux_bytes() -> u64 {
    std::env::var("RUST_BROADCAST_TS_REMUX_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_REMUX_BYTES)
}

#[test]
fn a_one_gib_ts_remux_holds_one_window_not_the_stream() {
    if std::env::var_os("RUST_BROADCAST_SOAK").is_none()
        && std::env::var_os("RUST_BROADCAST_TS_REMUX_BYTES").is_none()
    {
        eprintln!(
            "SKIP a_one_gib_ts_remux_holds_one_window_not_the_stream: set \
             RUST_BROADCAST_SOAK=1 (or RUST_BROADCAST_TS_REMUX_BYTES=<n>) to run"
        );
        return;
    }
    let dir = std::env::temp_dir().join(format!("transmux-ts-remux-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join("big.ts");
    let target = remux_bytes();
    let source = test_fixtures::path("ts/h264-aac.ts");
    let made = test_harness::big_media::write_big_ts(&path, &source, target);
    assert!(made.bytes >= target);

    let mut file = std::fs::File::open(&path).expect("open generated TS");
    let mut demux = StreamingTsDemux::new();
    let mut mux = TsMuxStage::new(Some(DEFAULT_PSI_INTERVAL));
    let mut buffer = vec![0u8; READ_CHUNK];
    let mut out_bytes = 0u64;
    let mut samples = 0u64;
    loop {
        let read = file.read(&mut buffer).expect("read chunk");
        if read == 0 {
            break;
        }
        demux.feed(&buffer[..read]).expect("feed demuxer");
        while let Some(event) = demux.poll_event() {
            match event {
                DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                    mux.feed(MuxInput::Track(Box::new(spec)), Timestamp::ZERO)
                        .expect("announce track");
                }
                DemuxEvent::Sample {
                    track_id, sample, ..
                } => {
                    samples += 1;
                    mux.feed(
                        MuxInput::Sample {
                            track: track_id,
                            sample,
                        },
                        Timestamp::ZERO,
                    )
                    .expect("mux sample");
                }
                _ => {}
            }
            while let Some(output) = Stage::poll(&mut mux) {
                if let MuxOutput::Bytes(chunk) = output {
                    out_bytes += chunk.len() as u64;
                }
            }
        }
    }
    demux.finish().expect("finish demuxer");
    Stage::finish(&mut mux).expect("finish muxer");
    while let Some(output) = Stage::poll(&mut mux) {
        if let MuxOutput::Bytes(chunk) = output {
            out_bytes += chunk.len() as u64;
        }
    }
    assert!(samples > 0, "the generated stream must carry samples");
    assert!(out_bytes > 0, "the remux must produce TS bytes");
    assert_eq!(out_bytes % 188, 0, "whole TS packets");
    let _ = std::fs::remove_dir_all(&dir);
}
