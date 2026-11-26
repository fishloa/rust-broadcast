//! The file-playout memory bound (issue #4057): a TS file far larger than the
//! memory budget must play from disk through the tokio reader without holding
//! memory proportional to its length.
//!
//! The capture is generated incrementally at test time (a quarter-gigabyte
//! fixture cannot be committed); the process-wide peak allocator reports the
//! live-byte high-water mark of the whole run.

#![cfg(target_pointer_width = "64")]

use std::sync::Arc;

use media_plane::trunk::{SampleCursorItem, Trunk, TrunkConfig};
use multimux::source::file_reader::{FileReader, FileReaderConfig};

#[global_allocator]
static A: test_alloc::ProcessPeak = test_alloc::ProcessPeak::new();

/// The generated capture's size: big enough that a whole-file read would blow
/// the budget several times over.
const FILE_BYTES: u64 = 256 << 20;
/// Live bytes the reader may hold while playing it: the session's own hold
/// bound plus the trunk ring, never the file.
const BUDGET: usize = 64 << 20;
/// Timed samples the trunk retains: a few drains' worth, independent of the
/// file's length.
const RING: usize = 2048;

fn trunk() -> Arc<Trunk> {
    let nz = std::num::NonZeroUsize::new(RING).unwrap_or(std::num::NonZeroUsize::MIN);
    Trunk::new(
        TrunkConfig::default()
            .with_timed_capacity(nz)
            .with_sparse_capacity(std::num::NonZeroUsize::MIN)
            .with_segment_capacity(std::num::NonZeroUsize::MIN)
            .with_event_capacity(std::num::NonZeroUsize::MIN)
            .with_part_capacity(std::num::NonZeroUsize::MIN),
    )
}

#[tokio::test(flavor = "current_thread")]
async fn ts_file_playout_holds_less_than_the_budget() {
    let dir = std::env::temp_dir().join(format!("multimux-file-memory-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("big.ts");
    let made = test_harness::big_media::write_big_ts(
        &path,
        &test_fixtures::path("ts/h264-aac.ts"),
        FILE_BYTES,
    );
    let expected: u64 = made.samples_per_track.iter().map(|(_, n)| *n).sum();

    let trunk = trunk();
    let mut cursor = trunk.subscribe_from_backlog();
    let config = FileReaderConfig::new(path, false, trunk.clone())
        .with_pace(false)
        .with_max_retries(0);
    A.reset();
    let baseline = A.live();
    let handle = tokio::spawn(FileReader::new(config).run());

    let mut published = 0u64;
    let mut missed = 0u64;
    let mut finished = false;
    loop {
        while let Some(item) = cursor.poll() {
            match item {
                SampleCursorItem::Timed { .. } => published += 1,
                // A live trunk never backpressures its writer, so an unpaced
                // reader may lawfully outrun this cursor: the ring tells the
                // cursor what it evicted, and the count below accounts for it.
                SampleCursorItem::Lagged { skipped } => missed += skipped,
                SampleCursorItem::TracksChanged { .. } => {}
                _ => panic!("unexpected sample-cursor item from the file reader"),
            }
        }
        if finished {
            break;
        }
        finished = handle.is_finished();
        if !finished {
            tokio::task::yield_now().await;
        }
    }
    handle
        .await
        .expect("the reader task must not panic")
        .expect("a TS file must stream cleanly");

    let held = A.peak().saturating_sub(baseline);
    eprintln!(
        "peak {held} bytes above baseline for a {FILE_BYTES}-byte TS capture, {published} samples"
    );
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        published + missed,
        expected,
        "every sample of the capture must reach the trunk ({missed} evicted before the cursor)"
    );
    assert!(
        held < BUDGET,
        "playing a {FILE_BYTES}-byte file held {held} live bytes, over the {BUDGET} budget"
    );
}
