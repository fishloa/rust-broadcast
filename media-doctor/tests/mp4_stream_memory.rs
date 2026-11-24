//! The streamed MP4 container check holds memory bounded by the stage's read
//! window, never the file's length (streaming-first Phase 3, issue #4057).
//!
//! One `#[test]` per binary because `test_alloc::ProcessPeak` is process-wide.

#![cfg(all(target_pointer_width = "64", feature = "std"))]

use std::fs::File;

use media_doctor::{Report, check_mp4_file};
use test_harness::big_media::{BigMp4Options, write_big_mp4};

#[global_allocator]
static A: test_alloc::ProcessPeak = test_alloc::ProcessPeak::new();

/// Generated file size: far above the memory budget.
const FILE_BYTES: u64 = 8 << 30;
/// Live bytes the check may hold while reading it.
const BUDGET: usize = 64 << 20;
/// Samples per track in the generated file.
const SAMPLE_COUNT: u32 = 20_000;

#[test]
fn an_eight_gib_mp4_is_checked_in_bounded_memory() {
    let path = std::env::temp_dir().join(format!(
        "media-doctor-mp4-memory-{}.mp4",
        std::process::id()
    ));
    let made = write_big_mp4(
        &path,
        &BigMp4Options {
            target_bytes: FILE_BYTES,
            sample_count: SAMPLE_COUNT,
            ..BigMp4Options::default()
        },
    );
    assert!(made.bytes >= FILE_BYTES, "the generated file is 8 GiB");

    let mut file = File::open(&path).expect("open generated file");
    let mut report = Report::new();
    A.reset();
    let baseline = A.live();
    check_mp4_file(&mut file, made.bytes, &mut report).expect("streamed container check");
    let held = A.peak().saturating_sub(baseline);
    eprintln!("media-doctor streamed an 8 GiB MP4 holding {held} bytes above baseline");
    assert!(
        held < BUDGET,
        "checking an 8 GiB MP4 held {held} live bytes, over the {BUDGET} budget"
    );
    assert!(
        report.is_empty(),
        "the generated clean MP4 must report no findings, got {:?}",
        report.findings()
    );
    let _ = std::fs::remove_file(&path);
}
