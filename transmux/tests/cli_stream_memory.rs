//! Memory bound for the streaming CLI (issue #953).
//!
//! Generates a 1 GiB MPEG-2 TS and runs the shipped binary's core over it,
//! writing TS and CMAF-HLS outputs, while [`ProcessPeak`] reports the live-byte
//! high-water mark. Soak-gated: the 1 GiB generation belongs in the nightly
//! soak, not every pull request.
//!
//! The budget is **64 MiB** of live bytes above the level at entry. CMAF's
//! single-fragment output is deliberately bounded by `isobmff::Limits`
//! (`DEFAULT_MAX_FRAGMENT_BYTES`, 64 MiB): a 1 GiB input is refused with
//! `FragmentTooLarge` rather than buffered, so the test asserts that bounded
//! refusal too and no case may approach the budget.
#![cfg(all(feature = "cli", feature = "std"))]

use clap::Parser;
use test_alloc::ProcessPeak;

#[global_allocator]
static PEAK: ProcessPeak = ProcessPeak::new();

/// Live bytes any one run may hold above the level at entry.
const BUDGET: usize = 64 * 1024 * 1024;
/// Generated input size, at least the issue's 1 GiB.
const INPUT_BYTES: u64 = 1 << 30;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target/cli-stream-memory")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn args(input: &std::path::Path, output: &std::path::Path, format: &str) -> transmux::cli::Args {
    transmux::cli::Args::try_parse_from([
        "transmux",
        input.to_str().expect("utf-8 input path"),
        "-o",
        output.to_str().expect("utf-8 output path"),
        "-f",
        format,
        "--segment-duration",
        "1",
    ])
    .expect("representative argv")
}

#[test]
fn a_big_ts_streams_under_budget() {
    if std::env::var("RUST_BROADCAST_SOAK").is_err() {
        eprintln!("SKIP cli_stream_memory: set RUST_BROADCAST_SOAK=1 to run the 1 GiB soak");
        return;
    }
    let dir = scratch("big");
    let input = dir.join("big.ts");
    let written = test_harness::big_media::write_big_ts(
        &input,
        &test_fixtures::path("ts/h264-aac.ts"),
        INPUT_BYTES,
    );
    assert!(
        written.bytes >= INPUT_BYTES,
        "generated {} bytes, wanted {INPUT_BYTES}",
        written.bytes
    );

    for (format, name) in [("ts", "out.ts"), ("hls", "out.m3u8")] {
        let output = dir.join(name);
        let parsed = args(&input, &output, format);
        let (peak, result) = PEAK.peak_during(|| transmux::cli::run(parsed));
        result.unwrap_or_else(|error| panic!("{format} run failed: {error}"));
        println!("cli_stream_memory: {format} peak {peak} bytes (budget {BUDGET})");
        assert!(
            peak < BUDGET,
            "{format} held {peak} bytes, over the {BUDGET}-byte budget"
        );
    }

    // CMAF is one fragment by contract; a 1 GiB input exceeds the fragment
    // bound, and the run must refuse rather than buffer.
    let output = dir.join("out.cmaf");
    let parsed = args(&input, &output, "cmaf");
    let (peak, result) = PEAK.peak_during(|| transmux::cli::run(parsed));
    println!("cli_stream_memory: cmaf peak {peak} bytes (budget {BUDGET})");
    assert!(
        peak < BUDGET,
        "cmaf held {peak} bytes, over the {BUDGET}-byte budget"
    );
    assert!(
        result.is_err(),
        "a 1 GiB single-fragment CMAF must be refused, not buffered"
    );
}
