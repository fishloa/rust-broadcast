//! Soak: `media-doctor check`'s TS pass over a generated multi-gigabyte
//! capture holds one packet plus each diagnostic's per-PID state, never the
//! file (streaming-first Phase 6, slice 4).
//!
//! Always runs at a small size; `RUST_BROADCAST_SOAK=1` raises it to the
//! 4 GiB the gate names. `RUST_BROADCAST_SOAK_BYTES` overrides either.

use std::fs::File;

use media_doctor::{
    CcAnomalyCheck, CodecSignallingCheck, Diagnostic, DiagnosticRunner, FpsCadenceCheck,
    InterlaceCheck, ParamSetsCheck, PatPmtVersionCheck, PcrCheck, PmtStreamCollector, PtsCheck,
    Report, Scte35Check, SyncByteCheck,
};
use mpeg_ts::reader::PacketReader;
use test_alloc::ThreadCapped;

/// Live-byte budget for the whole test: generous enough for generating the
/// file, small enough that a whole-capture read could never fit.
const BUDGET: usize = 64 * 1024 * 1024;

/// Extra live bytes the check pass may hold above its level at entry. The
/// streaming demuxer's pre-registration replay window is 4 MiB, so the bound
/// clears that and leaves room for the per-PID state.
const STREAM_BOUND: usize = 32 * 1024 * 1024;

const ALWAYS_ON_BYTES: u64 = 64 * 1024 * 1024;
const SOAK_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[global_allocator]
static ALLOC: ThreadCapped<BUDGET> = ThreadCapped::new();

fn target_bytes() -> u64 {
    if let Some(bytes) = std::env::var_os("RUST_BROADCAST_SOAK_BYTES")
        .and_then(|value| value.to_str().and_then(|text| text.parse::<u64>().ok()))
    {
        return bytes;
    }
    if std::env::var_os("RUST_BROADCAST_SOAK").as_deref() == Some(std::ffi::OsStr::new("1")) {
        SOAK_BYTES
    } else {
        ALWAYS_ON_BYTES
    }
}

#[test]
fn a_big_ts_checks_under_a_fixed_budget() {
    let target = target_bytes();
    let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let _ = std::fs::create_dir_all(&dir);
    let input = dir.join("soak-media-doctor.ts");
    let written = test_harness::big_media::write_big_ts(
        &input,
        &test_fixtures::path("ts/h264-aac.ts"),
        target,
    );
    assert!(
        written.bytes >= target,
        "generated {} bytes, wanted {target}",
        written.bytes
    );

    let baseline = ThreadCapped::<BUDGET>::live();
    ThreadCapped::<BUDGET>::reset_peak();

    // The two passes `media-doctor check` runs for a TS input: the PMT-declared
    // stream set, then every diagnostic fed one packet at a time.
    let mut collector = PmtStreamCollector::new();
    for packet in PacketReader::new(File::open(&input).expect("generated file opens")) {
        collector.feed(&packet.expect("a generated stream reads cleanly"));
    }
    let pmt = collector.finish();

    let diagnostics: &[&dyn Diagnostic] = &[
        &SyncByteCheck,
        &PatPmtVersionCheck,
        &CcAnomalyCheck,
        &PcrCheck,
        &PtsCheck,
        &Scte35Check,
        &CodecSignallingCheck,
        &FpsCadenceCheck,
        &ParamSetsCheck,
        &InterlaceCheck,
    ];
    let mut runner = DiagnosticRunner::new(diagnostics, &pmt);
    let mut report = Report::new();
    for packet in PacketReader::new(File::open(&input).expect("generated file opens")) {
        runner.feed(&packet.expect("a generated stream reads cleanly"));
    }
    runner.finish(&mut report);

    let peak = ThreadCapped::<BUDGET>::peak();
    let extra = peak.saturating_sub(baseline);
    println!(
        "soak_streaming (media-doctor check): {target} bytes in, {} finding(s), \
         peak {peak} bytes ({extra} above entry), bound {STREAM_BOUND}",
        report.len()
    );
    assert!(
        extra <= STREAM_BOUND,
        "the check held {extra} live bytes above entry, over the {STREAM_BOUND}-byte bound"
    );
    let _ = std::fs::remove_file(&input);
}
