# Streaming-first Phase 0 (stop the crash) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** multimux file playout of MPEG-TS and FLV, and DVR period recovery, run in memory that does not depend on file size.

**Architecture:** A new sans-IO `StreamingFileSession` in `media-plane` takes file bytes in chunks, demuxes them with the existing streaming demuxers, merges tracks through a bounded window and paces samples out. multimux's two file drivers (the standalone `FileReader` and the file route) feed it from disk in 64 KiB reads and rewind at end of file to loop. MP4, PS and WebM keep today's whole-file path behind the existing `max_file_bytes` cap until Phases 3 and 5. `dvr-runtime` recovery reads box headers from disk and seeks over payloads.

**Tech Stack:** Rust 1.95, `transmux::{StreamingTsDemux, StreamingFlvDemux, DemuxEvent}`, `container_probe`, `broadcast_common::Stage`, tokio (multimux only), `test-alloc`.

**Spec:** `docs/superpowers/specs/2026-11-24-streaming-first-containers-design.md` (§1, P1, P4, §5 Phase 0, §6).

**Tracking issue:** #4057.

## Global Constraints

- Memory is bounded by configuration, never by input size. No `fs::read`, `read_to_end` or whole-file buffer on the TS/FLV path.
- sans-IO in `media-plane` and `dvr-runtime::recover`: no file opens there; the caller reads.
- No new field on `FileReaderConfig`. No breaking change to multimux or media-plane public API. `dvr-runtime` takes the one breaking change Task 5 names. `max_file_bytes` stays and now applies to MP4, PS and WebM only.
- Every limit is a named field or constant with a doc comment giving its reason. No numeric literal other than 0 and 1 in non-test code.
- Zero warnings, no `#[allow]`/`#[expect]`, no comments that describe what the code does, no `.unwrap()`/`.expect()`/`panic!` in non-test code.
- Tests load fixtures only through `test_fixtures`. Big inputs are generated at test time into a temp dir, never committed.
- Typed `thiserror` errors; new enum variants only on `#[non_exhaustive]` enums.
- Commit after every green step. No Claude attribution in commits.
- Gate for every task: `cargo fmt --all --check`; `RUSTFLAGS="-D warnings" cargo clippy -p <crate> --all-features --all-targets --locked -- -D warnings`; `cargo test -p <crate> --all-features --locked`; `cargo build -p <crate> --no-default-features --locked`; `RUSTDOCFLAGS="-D warnings" cargo doc -p <crate> --all-features --no-deps --locked`; `cargo test -p workspace-guards --test guards --locked`.

## Review Focus

1. **Sparse track** (a subtitle or SCTE-35 data track with one sample a minute beside video): playout must not stall waiting for it, and its samples must still come out in presentation order. Test in Task 2.
2. **File with no playable samples** (PAT/PMT only, or zero bytes): a typed `NoPlayableSamples` / `FileTooShortToIdentify`, never a hang or an endless silent loop. Tests in Tasks 2 and 3.
3. **Truncated tail** (file ends mid TS packet or mid FLV tag): the complete samples before it play, the pass ends cleanly, and a loop continues the timeline. Test in Task 2.
4. **File replaced or shrunk between loop passes**: the next pass re-probes; a different container is a typed error, a different track set is re-announced. Test in Task 4.
5. **33-bit PTS wrap inside one pass** (a TS longer than 26.5 h, or one starting near the wrap): written timestamps keep increasing across the wrap and the pass span is the true duration. Test in Task 2.

---

### Task 1: Peak-memory harness

**Files:**
- Modify: `test-alloc/src/lib.rs`
- Create: `test-harness/src/big_media.rs`, export from `test-harness/src/lib.rs`
- Test: `test-alloc/tests/process_peak.rs`, `test-harness/tests/big_media.rs`

**Interfaces:**
- Produces, in `test_alloc`:
  - `pub struct ProcessPeak;` a `GlobalAlloc` delegating to `System` that tracks process-wide live bytes and their high-water mark in two `AtomicUsize`s.
  - `impl ProcessPeak { pub const fn new() -> Self; pub fn reset(&self); pub fn live(&self) -> usize; pub fn peak(&self) -> usize; pub fn peak_during<R>(&self, f: impl FnOnce() -> R) -> (usize, R) }`. `peak_during` returns the high-water mark *above the live bytes at entry*.
- Produces, in `test_harness::big_media`:
  - `pub fn write_big_ts(path: &Path, source_fixture: &Path, target_bytes: u64) -> BigMedia`
  - `pub fn write_big_flv(path: &Path, source_fixture: &Path, target_bytes: u64) -> BigMedia`
  - `pub struct BigMedia { pub bytes: u64, pub samples_per_track: Vec<(u32, u64)>, pub duration: Duration }`
  - Both demux the fixture once (small, in memory, test code), then write the output incrementally through `transmux::StreamingTsMux` / the FLV muxer, repeating the fixture's samples with timestamps advanced by one pass span each repeat, flushing to the file as they go. The writer itself must stay under 32 MiB live (asserted in its own test with `ProcessPeak`).

- [ ] **Step 1: Write the failing tests**

```rust
// test-alloc/tests/process_peak.rs
#[global_allocator]
static A: test_alloc::ProcessPeak = test_alloc::ProcessPeak::new();

#[test]
fn peak_during_reports_the_high_water_above_entry() {
    const HELD: usize = 8 << 20;
    let (peak, ()) = A.peak_during(|| drop(vec![0u8; HELD]));
    assert!(peak >= HELD && peak < HELD + (1 << 20), "peak {peak}");
    let (after, ()) = A.peak_during(|| ());
    assert!(after < 1 << 16, "nothing held: {after}");
}
```

```rust
// test-harness/tests/big_media.rs
#[global_allocator]
static A: test_alloc::ProcessPeak = test_alloc::ProcessPeak::new();
const TARGET: u64 = 256 << 20;
const WRITER_BUDGET: usize = 32 << 20;

#[test]
fn big_ts_is_written_incrementally_and_reaches_its_size() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("big.ts");
    let (peak, made) = A.peak_during(|| {
        test_harness::big_media::write_big_ts(&out, &test_fixtures::path("ts/m6-single.ts"), TARGET)
    });
    assert!(made.bytes >= TARGET);
    assert_eq!(std::fs::metadata(&out).unwrap().len(), made.bytes);
    assert_eq!(made.bytes % 188, 0);
    assert!(peak < WRITER_BUDGET, "writer held {peak}");
}
// the same test for write_big_flv with an FLV fixture found via test_fixtures::find
```

- [ ] **Step 2: Run them, expect FAIL** (`ProcessPeak` and `big_media` do not exist).
  Run: `cargo test -p test-alloc --test process_peak --locked` and `cargo test -p test-harness --test big_media --locked`

- [ ] **Step 3: Implement `ProcessPeak` and `big_media`.** `ProcessPeak` follows the existing `ProcessCounting` layout in the same file; all `unsafe` stays in the delegating calls with a `SAFETY:` line. Pick the FLV fixture with `test_fixtures::find`; if no public FLV fixture carries both audio and video, build the FLV source from the TS fixture's H.264/AAC samples.

- [ ] **Step 4: Run both tests, expect PASS.** Then the Task gate for `test-alloc` and `test-harness`.

- [ ] **Step 5: Commit** `test: process-wide peak allocator and incremental big-media writers`

---

### Task 2: `StreamingFileSession` in media-plane

**Files:**
- Create: `media-plane/src/file_stream.rs`; `pub mod file_stream;` in `media-plane/src/lib.rs` beside `file_source`
- Test: `media-plane/tests/file_stream.rs`, `media-plane/tests/file_stream_memory.rs`

**Interfaces:**
- Consumes: `transmux::{StreamingTsDemux, StreamingFlvDemux, DemuxEvent, TrackSpec, Sample, TrackId}`, `crate::file_source::{FileReaderError, PassRange, advance_offsets, secs_to_duration}`, `crate::ingress::SessionEvent`, `broadcast_common::{Stage, Timestamp}`; Task 1's harness in tests.
- Produces:

```rust
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamContainer { MpegTs, Flv }

#[derive(Debug, Clone)]
pub struct FileStreamLimits {
    /// Bytes of demuxed samples held ahead of the pacing clock. Default 8 MiB.
    pub max_queued_bytes: usize,
    /// Media time held ahead of the pacing clock. Default 2 s.
    pub read_ahead: Duration,
    /// How far one track may trail another in the file before the merge
    /// stops waiting for it. Default 1 s.
    pub interleave_skew: Duration,
    /// Media time the session waits at the start of a pass for every track
    /// to appear before announcing. Default 10 s.
    pub start_window: Duration,
    /// Bytes fed before the start window closes regardless. Default 16 MiB.
    pub start_window_bytes: usize,
    /// Distinct presentation times kept to measure the frame period.
    /// Default 64, above the deepest decoder reorder (16 frames).
    pub pts_window: usize,
}
impl Default for FileStreamLimits { /* the defaults above, each a named const */ }

pub enum FileInput<'a> { Bytes(&'a [u8]), Eof }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDemand {
    /// Feed the next chunk of the file.
    Bytes,
    /// Seek the file to offset 0, then feed.
    Rewind,
    /// Nothing wanted until `next_due()`.
    Wait,
    /// The session has finished; drop the file.
    Done,
}

pub struct StreamingFileSession { /* private */ }
impl StreamingFileSession {
    pub fn new(container: StreamContainer, program: ProgramId, loop_file: bool,
               max_loops: Option<u32>, pace: bool, limits: FileStreamLimits) -> Self;
    pub fn file_demand(&self) -> FileDemand;
    pub fn next_due(&self) -> Option<std::time::Instant>;
    pub fn drained(&self) -> bool;
}
impl Stage for StreamingFileSession {
    type In<'a> = FileInput<'a>;
    type Out = SessionEvent;
    type Error = FileReaderError;
}
```

**Behaviour the tests pin (decisions, not left to the implementer):**
- **Events:** `Established`, then `NewProgram` with every track, then `Sample`s. `NewProgram` is emitted when the demuxer reports `TracksResolved`, or when the start window closes (media time or bytes), whichever is first. A track that appears later, or a spec that differs on a later pass, causes a fresh `NewProgram` with the full current set.
- **Merge order:** per track, samples stay in demux (decode) order. Across tracks the next sample is the queue front with the smallest presentation time in seconds, ties broken by track index then decode time: the same key as `ParsedFile::build_playout`. A front is released when every known track has a queued sample, or when its presentation time is at least `interleave_skew` behind the latest presentation time seen on any track.
- **Bounded hold:** `file_demand()` is `Bytes` only while queued sample bytes are below `max_queued_bytes` and queued media time is below `read_ahead`; otherwise `Wait`. When not pacing, `Wait` clears as `poll` drains. A single fed chunk may overshoot the cap by its own demuxed content; nothing else may.
- **Pacing:** a sample is due at `pass_wall_start + (due_seconds - pass_first_due)`, where `pass_first_due` is the presentation time of the first sample released in the pass. `MAX_SAMPLES_PER_DRAIN` from `file_source` bounds one `feed`/`poll` burst as today.
- **Pass range, computed while streaming:** per track, `span_ticks = max_pts - min_pts` over the pass; `frame_ticks` is the smallest positive gap between neighbouring distinct presentation times, measured through a sorted window of `pts_window` values (the smallest value is compared with its successor when it leaves the window, and the remaining window is drained at end of pass). A presentation time below a value that has already left the window is ignored for the gap. No positive gap falls back to the first positive sample duration, then 1, as `build_playout` does.
- **Loop:** on `Eof` the session finishes the demuxer, drains its queues, then: if looping and `max_loops` is not reached, advances per-track offsets with `advance_offsets`, advances `pass_wall_start` by the longest `span + frame`, creates a fresh demuxer and demands `Rewind`; otherwise demands `Done` and `drained()` is true. A pass that released no sample is `FileReaderError::NoPlayableSamples`.
- **Timestamps:** each released sample's timing is shifted by its track's loop offset with checked arithmetic (`OffsetOverflow` on overflow). Presentation times inside a pass are unrolled across a 33-bit wrap with `broadcast_common::clock33` before any comparison, when the demuxer has not already done so.
- **Memory:** no collection grows with file length. No per-sample list is kept after a sample is released.

- [ ] **Step 1: Write the failing differential test**

```rust
// media-plane/tests/file_stream.rs
// Drives the session with CHUNK-byte feeds and no pacing, collecting
// (track_id, pts, dts, duration, sync, data bytes) per sample; fixtures are small.
const CHUNK: usize = 64 << 10;
const PASSES: u32 = 3;

fn old_path(bytes: &[u8]) -> Vec<Emitted>;   // probe_and_demux + FileIngestSession, 3 passes
fn new_path(bytes: &[u8], c: StreamContainer) -> Vec<Emitted>; // StreamingFileSession, max_loops Some(3)

#[test]
fn ts_streaming_session_matches_the_whole_file_session() {
    for name in ["ts/m6-single.ts" /* + every TS fixture multimux/tests/it/file_reader.rs uses */] {
        let bytes = test_fixtures::read(name);
        assert_eq!(new_path(&bytes, StreamContainer::MpegTs), old_path(&bytes), "{name}");
    }
}
#[test] fn flv_streaming_session_matches_the_whole_file_session() { /* same, FLV fixtures */ }
#[test] fn chunk_size_does_not_change_the_output() { /* CHUNK of 1, 187, 188, 4096, 1 << 20 give equal output */ }
```

- [ ] **Step 2: Run, expect FAIL** (`file_stream` does not exist). `cargo test -p media-plane --test file_stream --locked`

- [ ] **Step 3: Write the failing behaviour tests** in the same file:

```rust
#[test] fn events_start_established_then_new_program_then_samples();
#[test] fn sparse_track_does_not_stall_the_merge_and_stays_in_order();
//   two tracks: video at 25 fps, a data track with one sample per 60 s; with
//   interleave_skew 1 s every video sample is released without the data queue
//   filling; the data samples come out between the right video samples.
#[test] fn no_playable_samples_is_a_typed_error_after_the_first_pass();
//   a PAT/PMT-only TS then Eof -> Err(NoPlayableSamples); file_demand never Rewind.
#[test] fn truncated_tail_plays_whole_samples_and_loops_cleanly();
//   fixture cut at len-100: every sample fully present is emitted; pass 2's first
//   pts == pass 1's max pts + frame for that track.
#[test] fn pts_wrap_inside_a_pass_keeps_the_timeline_increasing();
//   TS fixture restamped (test_harness) to start 2 s before 2^33: per-track pts
//   strictly follows decode-order expectations and span == true duration.
#[test] fn demand_is_wait_once_the_read_ahead_is_full_and_bytes_after_draining();
#[test] fn a_track_set_change_between_passes_is_reannounced();
#[test] fn loop_offsets_overflow_is_a_typed_error();
#[test] fn paced_sample_is_not_released_before_it_is_due(); // tokio::time::pause not available here: use next_due() against Instant::now()
```

- [ ] **Step 4: Write the failing memory test**

```rust
// media-plane/tests/file_stream_memory.rs
#[global_allocator]
static A: test_alloc::ProcessPeak = test_alloc::ProcessPeak::new();
const FILE_BYTES: u64 = 256 << 20;
const BUDGET: usize = 32 << 20;
const CHUNK: usize = 64 << 10;

#[test]
fn ts_playout_memory_does_not_grow_with_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.ts");
    let made = test_harness::big_media::write_big_ts(&path, &test_fixtures::path("ts/m6-single.ts"), FILE_BYTES);
    let (peak, emitted) = A.peak_during(|| drive_from_disk(&path, StreamContainer::MpegTs, CHUNK, /*passes*/ 2));
    assert_eq!(emitted, 2 * made.samples_per_track.iter().map(|t| t.1).sum::<u64>());
    assert!(peak < BUDGET, "held {peak} for a {FILE_BYTES}-byte file");
}
#[test] fn flv_playout_memory_does_not_grow_with_the_file() { /* same with write_big_flv */ }
// drive_from_disk reads CHUNK bytes with std::fs::File per FileDemand::Bytes,
// seeks to 0 on Rewind, drops each emitted sample at once.
```
A 4 GiB variant of both runs only under `RUST_BROADCAST_SOAK=1`.

- [ ] **Step 5: Implement `file_stream.rs`** to the Interfaces block and the behaviour list. Keep `file_source.rs` untouched except for making `MAX_SAMPLES_PER_DRAIN` and any helper it shares `pub(crate)`.

- [ ] **Step 6: Run all three test files, expect PASS**, then the Task gate for `media-plane`. Confirm the memory tests bite: temporarily collect emitted samples into a `Vec` inside the session, see both fail, revert.

- [ ] **Step 7: Commit** `feat(media-plane): bounded-memory streaming file session for TS and FLV`

---

### Task 3: Probe a file from a bounded prefix

**Files:**
- Modify: `media-plane/src/file_source.rs`
- Test: `media-plane/tests/file_probe.rs`

**Interfaces:**
- Produces:

```rust
/// Largest prefix read to identify a file. 4 MiB.
pub const PROBE_PREFIX_MAX: usize = 4 << 20;

pub enum ProbeStep {
    /// Read the file's first `prefix_len` bytes and call again.
    NeedPrefix { prefix_len: usize },
    Identified(DemuxerKind),
}
/// `prefix` is the first bytes of the file; `file_len` its total length.
pub fn probe_prefix(prefix: &[u8], file_len: u64) -> Result<ProbeStep, FileReaderError>;
```
- Starts from `container_probe::DEFAULT_BUDGET`, follows `Insufficient { need_at_least }` up to `min(PROBE_PREFIX_MAX, file_len)`, then maps: `Ambiguous` to `AmbiguousProbe`, `Insufficient` at the limit to `FileTooShortToIdentify`, anything else to `UnknownProbe`, and an identified format through the existing `select_demuxer`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn every_playout_fixture_is_identified_from_a_prefix_as_the_whole_file_is();
//   for each fixture file_reader tests use: drive probe_prefix's loop; the DemuxerKind
//   equals the one probe_and_demux's path selects (via container_probe + select_demuxer on the whole file).
#[test] fn the_loop_never_asks_for_more_than_the_cap_or_the_file();
#[test] fn an_empty_file_is_too_short_to_identify();
#[test] fn a_file_of_zeros_is_unknown_not_a_hang(); // 8 MiB of zeros -> Err within the cap
```

- [ ] **Step 2: Run, expect FAIL.** `cargo test -p media-plane --test file_probe --locked`
- [ ] **Step 3: Implement `probe_prefix`.**
- [ ] **Step 4: Run, expect PASS**, then the Task gate.
- [ ] **Step 5: Commit** `feat(media-plane): identify a playout file from a bounded prefix`

---

### Task 4: multimux drives the streaming session

**Files:**
- Modify: `multimux/src/source/file_reader.rs` (both drivers: `FileReader::spawn`'s run and the file route built around `FileIngestSession` near line 555)
- Test: `multimux/tests/it/file_reader.rs`, `multimux/tests/it/file_route.rs`, new `multimux/tests/it/file_memory.rs` (add to `multimux/tests/it/main.rs`)

**Interfaces:**
- Consumes: Task 2's `StreamingFileSession`, `FileInput`, `FileDemand`, `StreamContainer`, `FileStreamLimits`; Task 3's `probe_prefix`; Task 1's harness in tests.
- Produces: no public API change. One private constant `FILE_READ_CHUNK: usize = 64 << 10`.

**Decisions:**
- Open, stat and reject non-regular files exactly as today (`Read`, `NotARegularFile`), with the same retry schedule for `Read` errors.
- Run `probe_prefix` over the open file. `StreamingTs` and `StreamingFlv` take the streaming path: `max_file_bytes` is not consulted. Every other kind takes today's path unchanged (cap check, `read_capped`, `probe_and_demux`, `FileIngestSession`).
- Streaming path loop: on `Bytes`, read up to `FILE_READ_CHUNK` with tokio and `feed(FileInput::Bytes)`, or `feed(FileInput::Eof)` at a zero-length read; on `Rewind`, re-stat the file, re-run `probe_prefix` (a different `DemuxerKind` is `FileReaderError::UnsupportedFormat`-class: add `FileChangedContainer { path }` to the `#[non_exhaustive]` `FileReaderError`), seek to 0; on `Wait`, sleep until `next_due()` (or yield when not pacing); on `Done`, finish. Samples and track announcements go to the trunk the way the current drivers send them. Cancellation behaves as today.
- Update `FileReaderConfig::max_file_bytes`, `with_max_file_bytes` and `DEFAULT_MAX_FILE_BYTES` docs: the cap applies to MP4, PS and WebM files only; TS and FLV stream and are not capped. Update the module doc's pipeline section to describe the two paths.

- [ ] **Step 1: Write the failing tests**

```rust
// multimux/tests/it/file_reader.rs (additions)
#[tokio::test] async fn a_ts_file_larger_than_the_cap_plays();
//   config.with_max_file_bytes(1 << 20) and a 4 MiB generated TS: samples reach the trunk, no FileTooLarge.
#[tokio::test] async fn an_flv_file_larger_than_the_cap_plays();
#[tokio::test] async fn an_mp4_file_larger_than_the_cap_is_still_refused(); // FileTooLarge, as today
#[tokio::test] async fn a_ts_file_replaced_by_an_mp4_between_passes_fails_with_file_changed_container();
#[tokio::test] async fn a_ts_file_that_grows_between_passes_plays_the_longer_file_on_the_next_pass();
```

```rust
// multimux/tests/it/file_memory.rs
#[global_allocator]
static A: test_alloc::ProcessPeak = test_alloc::ProcessPeak::new();
const FILE_BYTES: u64 = 256 << 20;
const BUDGET: usize = 64 << 20;

#[tokio::test(flavor = "current_thread")]
async fn ts_file_playout_holds_less_than_the_budget() {
    // write_big_ts(FILE_BYTES); FileReader with pace off, loop off, a trunk whose
    // ring is small and a subscriber that drains and drops; assert every sample
    // arrives (count == made.samples_per_track total) and A.peak() - baseline < BUDGET.
}
```
If `multimux/tests/it` already declares a global allocator, put this test in its own integration test binary `multimux/tests/file_memory.rs`.

- [ ] **Step 2: Run, expect FAIL** (`FileTooLarge` for the TS case; the memory test exceeds the budget). `cargo test -p multimux --test it file_ --all-features --locked`
- [ ] **Step 3: Implement the streaming path in both drivers.**
- [ ] **Step 4: Run the new tests and the whole existing `file_reader` and `file_route` suites, expect PASS.** Existing timeline, pacing and loop tests must pass without edits to their assertions; a test that pins an implementation detail of the old path (e.g. the recorded mutation note on `FileIngestSession::refill`) is re-pointed at the equivalent streaming behaviour and the note re-verified by making the mutation.
- [ ] **Step 5: Task gate for `multimux`** (`--all-features`), then commit `fix(multimux): stream TS and FLV file playout from disk in bounded memory`

---

### Task 5: DVR recovery scans from disk

**Files:**
- Modify: `dvr-runtime/src/recover.rs`, `dvr-runtime/src/fs.rs:788-794`
- Test: `dvr-runtime/tests/it/recover.rs`, new `dvr-runtime/tests/recover_memory.rs`

**Interfaces:**
- Produces (replacing `scan_fmp4_data(&[u8])`, which is deleted: a breaking change, so `dvr-runtime` goes 0.1 to 0.2 and its dependants bump under epoch purity):

```rust
/// Bytes of a box header: 32-bit size, type, 64-bit largesize.
pub const BOX_HEADER_MAX: usize = 16;

pub enum ScanStep {
    /// Supply `len` bytes at `offset` (always a box header) and call `supply`.
    Need { offset: u64, len: usize },
    Done(Vec<RecoveredBox>),
}
pub struct Fmp4Scan { /* private */ }
impl Fmp4Scan {
    pub fn new(file_len: u64) -> Self;
    pub fn step(&mut self) -> Result<ScanStep, RecoverError>;
    pub fn supply(&mut self, offset: u64, bytes: &[u8]) -> Result<(), RecoverError>;
}
```
- `RecoveredBox` and its `ByteSpan`s are unchanged; spans become `u64`-based if they are `usize` today only where a 32-bit target would overflow (keep the public type if it is already wide enough).
- The scan reads headers only. A box whose declared size runs past `file_len` ends the scan exactly where `scan_fmp4_data` stopped (a torn tail). A first box that is not a valid header is `RecoverError::NotIsoBmff`.
- `FsArchive::recover_spans` opens the period file, drives `Fmp4Scan` with `seek` + `read_exact` of each requested header, and returns the same `Vec<RecoveredBox>`.

- [ ] **Step 1: Port the existing tests.** In `dvr-runtime/tests/it/recover.rs`, replace each `scan_fmp4_data(&data)` call with a local `scan(&data)` helper that drives `Fmp4Scan` over the slice. Assertions stay as they are.
- [ ] **Step 2: Add the failing tests**

```rust
#[test] fn the_scan_only_ever_asks_for_box_headers();
//   record every Need: len <= BOX_HEADER_MAX, and total bytes supplied <= BOX_HEADER_MAX * (boxes + 1).
#[test] fn a_declared_size_past_the_file_end_stops_at_the_last_whole_box();
#[test] fn a_zero_size_box_and_a_size_smaller_than_its_header_end_the_scan_without_looping();
#[test] fn a_64_bit_largesize_mdat_is_skipped_by_its_size();
```
```rust
// dvr-runtime/tests/recover_memory.rs
#[global_allocator]
static A: test_alloc::ProcessPeak = test_alloc::ProcessPeak::new();
const PERIOD_BYTES: u64 = 512 << 20;
const BUDGET: usize = 4 << 20;
#[test]
fn recovering_a_large_period_reads_only_headers() {
    // build a period file of init + N fragments whose mdat bodies are written with
    // File::set_len / seek (sparse), reach PERIOD_BYTES; FsArchive::recover_spans;
    // assert spans equal the offsets written and peak < BUDGET.
}
```
- [ ] **Step 3: Run, expect FAIL.** `cargo test -p dvr-runtime --all-features --locked recover`
- [ ] **Step 4: Implement `Fmp4Scan`, delete `scan_fmp4_data`, switch `recover_spans`.** Box header decoding goes through `transmux`'s existing header parser, not a new one.
- [ ] **Step 5: Run, expect PASS**, then the Task gate for `dvr-runtime` (`--all-features` and `--no-default-features`).
- [ ] **Step 6: Commit** `fix(dvr-runtime): recover a period by scanning box headers from disk`

---

### Task 6: Docs and release notes

**Files:**
- Modify: `multimux/CHANGELOG.md`, `media-plane/CHANGELOG.md`, `dvr-runtime/CHANGELOG.md` (`[Unreleased]`), `multimux/README.md` (file input section), `docs/CRATES.md` via `python3 tools/gen-crate-index.py` if crate descriptions changed, root `CLAUDE.md` entries for `multimux` and `dvr-runtime`.

- [ ] **Step 1:** CHANGELOG entries. multimux `### Fixed`: TS and FLV file inputs stream from disk in bounded memory and are no longer subject to `max_file_bytes`; `### Changed`: the cap now applies to MP4, PS and WebM only. media-plane `### Added`: `file_stream::StreamingFileSession`, `file_source::probe_prefix`. dvr-runtime `### Breaking`: `recover::scan_fmp4_data` replaced by `recover::Fmp4Scan`; `### Fixed`: `recover_spans` no longer reads the period file into memory. Each names #4057.
- [ ] **Step 2:** README and CLAUDE.md: state which file containers stream and which are capped until the later phases.
- [ ] **Step 3:** Full workspace gate, exactly as CI: `cargo build --workspace --all-features --locked`; `cargo test --workspace --all-features --locked`; `cargo build --workspace --no-default-features --locked`; `cargo clippy --workspace --all-features --all-targets --locked -- -D warnings` with `RUSTFLAGS="-D warnings"`; `cargo fmt --all --check`; `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked`.
- [ ] **Step 4: Commit** `docs: bounded-memory file playout and DVR recovery (#4057)`

Version bumps and tags are the orchestrator's, after the audit, and need the owner's OK to publish.

---

## Lanes

Tasks 1 and 5 are independent of each other and of the rest. Tasks 2 and 3 need Task 1 merged. Task 4 needs 2 and 3. Task 6 is last.

| Lane | Tasks | Crates |
|---|---|---|
| A | 1 | test-alloc, test-harness |
| B | 5 | dvr-runtime (after A lands, for `ProcessPeak`) |
| C | 2, 3 | media-plane |
| D | 4, 6 | multimux, docs |

The orchestrator audits each lane on its own gate run before landing, and checks each memory test bites by making the mutation named in Task 2 Step 6.
