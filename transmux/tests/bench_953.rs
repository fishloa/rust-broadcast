//! #953 benchmark: a CENC fragmented MP4 to MPEG-2 TS under a 128 MiB budget.
//!
//! Issue #953 measured a 2 GiB CENC fragmented MP4 converted to TS peaking at
//! 6.66 GB, because the CLI held the input, a decrypted copy and the output at
//! once. This test drives the same CLI entry point the binary runs, over a
//! generated file of the same shape, under a per-thread live-byte cap that
//! aborts the allocation that would cross 128 MiB, and records the measured
//! peak so a regression names a number.
//!
//! The generated file is real CENC: `cenc`/AES-CTR with subsample encryption
//! for the video track, produced one sample at a time by the crypto and CMAF
//! muxer stages. The output is demuxed back to per-track samples so the decrypt
//! is checked to be real, not skipped.
//!
//! Full size is soak-gated (`RUST_BROADCAST_SOAK=1`); without it a quick size
//! still exercises several fragments (override either with
//! `RUST_BROADCAST_BENCH_953_BYTES`).
#![cfg(all(feature = "cli", feature = "std", feature = "cenc"))]

use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::Parser;
use test_alloc::ThreadCapped;
use test_harness::big_media::{BigFmp4Options, write_big_fmp4};
use transmux::cli::{Args, run};
use transmux::{DemuxEvent, StreamingTsDemux};

#[global_allocator]
static ALLOC: ThreadCapped<BUDGET> = ThreadCapped::new();

/// Bytes in one mebibyte.
const MIB: usize = 1024 * 1024;
/// Live bytes one thread may hold; the cap enforces the issue's target.
const BUDGET: usize = 128 * MIB;
/// Bytes in one gibibyte.
const GIB: u64 = 1024 * 1024 * 1024;
/// The issue's input size.
const FULL_INPUT_BYTES: u64 = 2 * GIB;
/// The quick size: several fragments and a real decrypt.
const QUICK_INPUT_BYTES: u64 = 32 * MIB as u64;
/// Bytes in one generated sample, kept small enough for the benchmark's own
/// memory budget and a bounded TS output.
const SAMPLE_BYTES: u32 = 3 * MIB as u32;
/// Bytes read from the TS output per feed.
const READ_CHUNK_BYTES: usize = 64 * 1024;
/// Environment variable that overrides the generated size.
const SIZE_ENV: &str = "RUST_BROADCAST_BENCH_953_BYTES";
/// Environment variable that opts into the full-size soak.
const SOAK_ENV: &str = "RUST_BROADCAST_SOAK";

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target/bench-953")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the benchmark scratch directory");
    dir
}

fn input_bytes() -> u64 {
    if let Ok(value) = std::env::var(SIZE_ENV)
        && let Ok(bytes) = value.trim().parse::<u64>()
        && bytes > 0
    {
        return bytes;
    }
    if std::env::var(SOAK_ENV).is_ok() {
        FULL_INPUT_BYTES
    } else {
        QUICK_INPUT_BYTES
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[test]
fn a_cenc_fragmented_mp4_decrypts_to_ts_under_the_budget() {
    let target_bytes = input_bytes();
    let dir = scratch("cenc-to-ts");
    let input = dir.join("big-cenc.mp4");
    let output = dir.join("out.ts");

    let written = write_big_fmp4(
        &input,
        &BigFmp4Options {
            target_bytes,
            fragment_count: 1,
            sample_bytes: SAMPLE_BYTES,
            with_mfra: false,
            protect: true,
        },
    );
    assert!(
        written.bytes >= target_bytes,
        "generated {} bytes, wanted {target_bytes}",
        written.bytes
    );
    let cenc = written.cenc.as_ref().expect("CENC report");
    let key_argument = format!("{}:{}", hex(&cenc.key_id), hex(&cenc.content_key));
    let parsed = Args::try_parse_from([
        "transmux",
        input.to_str().expect("utf-8 input path"),
        "-o",
        output.to_str().expect("utf-8 output path"),
        "-f",
        "ts",
        "--decrypt",
        "--key",
        &key_argument,
    ])
    .expect("representative benchmark argv");

    ThreadCapped::<BUDGET>::reset_peak();
    let started = Instant::now();
    let result = run(parsed);
    let elapsed = started.elapsed();
    let peak = ThreadCapped::<BUDGET>::peak();
    result.unwrap_or_else(|error| panic!("CENC fMP4 to TS run failed: {error}"));

    println!(
        "bench_953: input {} bytes, wall {:?}, peak {peak} bytes (budget {BUDGET})",
        written.bytes, elapsed
    );
    assert!(
        peak < BUDGET,
        "the CENC fMP4 to TS run held {peak} live bytes, over the {BUDGET}-byte budget"
    );

    let (counts, head, tail) = scan_ts(&output, cenc.hashes);
    let expected: Vec<u64> = written
        .samples_per_track
        .iter()
        .map(|(_, count)| *count)
        .collect();
    assert_eq!(
        counts.values().copied().collect::<Vec<u64>>(),
        expected,
        "the output TS must carry every generated sample"
    );
    assert_eq!(head, cenc.head_hashes, "the first samples must decrypt");
    assert_eq!(tail, cenc.tail_hashes, "the last samples must decrypt");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Demux the output TS, returning per-track sample counts and the CRC-32 of
/// each track's first and last `edges` sample bytes.
fn scan_ts(path: &Path, edges: usize) -> (BTreeMap<u32, u64>, Vec<u32>, Vec<u32>) {
    let mut file = File::open(path).expect("open the output TS");
    let mut buf = vec![0u8; READ_CHUNK_BYTES];
    let mut demux = StreamingTsDemux::new();
    let mut counts = BTreeMap::new();
    let mut head = Vec::new();
    let mut tail: VecDeque<u32> = VecDeque::new();
    let record = |event: DemuxEvent,
                  counts: &mut BTreeMap<u32, u64>,
                  head: &mut Vec<u32>,
                  tail: &mut VecDeque<u32>| {
        if let DemuxEvent::Sample {
            track_id, sample, ..
        } = event
        {
            let count = counts.entry(track_id.value()).or_insert(0);
            *count = count.saturating_add(1);
            let hash = broadcast_common::crc32_mpeg2::compute(&sample.data);
            if head.len() < edges {
                head.push(hash);
            } else {
                if tail.len() == edges {
                    tail.pop_front();
                }
                tail.push_back(hash);
            }
        }
    };
    loop {
        let read = file.read(&mut buf).expect("read the output TS");
        if read == 0 {
            break;
        }
        demux.feed(&buf[..read]).expect("feed TS bytes");
        while let Some(event) = demux.poll_event() {
            record(event, &mut counts, &mut head, &mut tail);
        }
    }
    demux.finish().expect("finish the TS demux");
    while let Some(event) = demux.poll_event() {
        record(event, &mut counts, &mut head, &mut tail);
    }
    (counts, head, tail.into_iter().collect())
}
