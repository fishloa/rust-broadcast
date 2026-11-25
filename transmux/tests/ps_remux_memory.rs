//! Streaming MPEG-2 Program Stream remux memory bound (Phase 5 slice 1): a
//! generated 4 GiB Program Stream demuxed by `StreamingPsDemux` and re-muxed
//! sample by sample to TS must stay under a fixed, small live-allocation
//! budget.
//!
//! The generator ([`write_big_ps`]) repeats a committed PS fixture's packs,
//! advancing every SCR and PES timestamp, writing pack by pack; this test reads
//! the result back in one-mebibyte chunks, feeds the demuxer and the TS mux
//! stage, and drops every byte it produces. `test_alloc::ThreadCapped` makes
//! that a hard gate: an allocation that would push this thread's live bytes
//! over [`BUDGET`] returns null, aborting here rather than swapping the machine.
//!
//! The generator is soak-gated: writing and re-reading four gigabytes takes far
//! longer than a normal unit-test budget, so it runs under
//! `RUST_BROADCAST_SOAK=1` (the nightly workflow) and reports its skip loudly
//! otherwise. `RUST_BROADCAST_PS_REMUX_BYTES` overrides the size for a local run.
#![cfg(feature = "std")]

use std::io::Read;

use broadcast_common::Stage;
use broadcast_common::stage::Timestamp;
use test_alloc::ThreadCapped;
use transmux::mux_stage::{MuxInput, MuxOutput};
use transmux::{DEFAULT_PSI_INTERVAL, DemuxEvent, StreamingPsDemux, TsMuxStage};

/// Live-allocation budget for one thread: the fixture's pack list the generator
/// holds (~0.2 MiB), one 1 MiB read buffer, the scanner's one PES (~21 KiB),
/// the demuxed samples held before the track announcement, and the TS muxer's
/// one-interleave-window of packets. 16 MiB leaves headroom for allocator
/// overhead while a whole-input or whole-output regression (which needs
/// gigabytes) cannot hide under it.
const BUDGET: usize = 16 << 20;

#[global_allocator]
static ALLOC: ThreadCapped<BUDGET> = ThreadCapped::new();

/// Default remux size: four gibibytes.
const DEFAULT_REMUX_BYTES: u64 = 4 << 30;

/// Read-buffer size: one mebibyte, small enough that the input is never held
/// and large enough that the loop is not syscall-bound.
const READ_CHUNK: usize = 1 << 20;

fn remux_bytes() -> u64 {
    std::env::var("RUST_BROADCAST_PS_REMUX_BYTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_REMUX_BYTES)
}

#[test]
fn a_four_gib_ps_remux_holds_one_pes_not_the_stream() {
    if std::env::var_os("RUST_BROADCAST_SOAK").is_none()
        && std::env::var_os("RUST_BROADCAST_PS_REMUX_BYTES").is_none()
    {
        eprintln!(
            "SKIP a_four_gib_ps_remux_holds_one_pes_not_the_stream: set \
             RUST_BROADCAST_SOAK=1 (or RUST_BROADCAST_PS_REMUX_BYTES=<n>) to run"
        );
        return;
    }
    let dir = std::env::temp_dir().join(format!("transmux-ps-remux-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let path = dir.join("big.ps");
    let target = remux_bytes();
    let source = test_fixtures::path("ps/h264-ac3.ps");
    let made = test_harness::big_media::write_big_ps(&path, &source, target);
    assert!(made.bytes >= target);

    let mut file = std::fs::File::open(&path).expect("open generated PS");
    let mut demux = StreamingPsDemux::new();
    let mut mux = TsMuxStage::new(Some(DEFAULT_PSI_INTERVAL));
    let mut buffer = vec![0u8; READ_CHUNK];
    let mut out_bytes = 0u64;
    let mut samples = 0u64;
    ThreadCapped::<BUDGET>::reset_peak();
    loop {
        let read = file.read(&mut buffer).expect("read chunk");
        if read == 0 {
            break;
        }
        demux
            .feed(&buffer[..read], Timestamp::ZERO)
            .expect("feed demuxer");
        while let Some(event) = demux.poll() {
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
    eprintln!(
        "ps remux: {} bytes in, {} samples, {} TS bytes, live peak <= {} bytes",
        made.bytes,
        samples,
        out_bytes,
        ThreadCapped::<BUDGET>::peak()
    );
    let _ = std::fs::remove_dir_all(&dir);
}
