//! The linear-playout **FileReader**: play a media file on disk into a
//! [`media_plane::trunk::Trunk`] as same-IR samples every other ingest path
//! produces.
//!
//! This is a **plain tokio task**, not a
//! [`Dialer`](media_plane::ingress::Dialer)/[`IngestSession`](media_plane::ingress::IngestSession):
//! there is no connection to dial and no reconnect semantics to honour. The
//! design it implements is `docs/superpowers/specs/2026-08-11-linear-playout-design.md`
//! §"`multimux/src/source/file_reader.rs`", issue #748.
//!
//! # Pipeline
//!
//! 1. **Read** the whole file into memory.
//! 2. **Identify** the container with [`container_probe::probe_with_budget`]
//!    over the **entire** file (not a prefix — the default 64 KiB budget
//!    identifies the format but does not always settle the ISOBMFF layout,
//!    and a file reader holds the whole file anyway). The verdict maps to a
//!    [`transmux`] demuxer exactly per the design spec's table; a
//!    [`container_probe::Probe`] that is `Ambiguous`/`Insufficient`/`Unknown`,
//!    or a format [`transmux`] cannot demux as a playout source, **fails the
//!    source** — a file is never fed to a guessed demuxer, and the ISOBMFF
//!    layout is never re-derived here (the probe already did it; two
//!    implementations of one rule that can disagree is the bug class this
//!    workspace keeps finding).
//! 3. **Demux** the file into the neutral `Media`/`Track`/`Sample` IR. The
//!    whole-buffer demuxers ([`transmux::PsDemux`], [`transmux::Fmp4Demux`],
//!    [`transmux::ProgressiveDemux`], [`transmux::WebmDemux`]) run once over
//!    the buffer; the streaming demuxers ([`transmux::StreamingTsDemux`],
//!    [`transmux::StreamingFlvDemux`]) are fed and their [`transmux::DemuxEvent`]s
//!    drained into the same IR.
//! 4. **Announce the tracks** on the Trunk the same way every other ingest
//!    does ([`TrunkWriter::set_tracks`]), **before** the first sample — and
//!    again on any loop where the codec configuration differs.
//! 5. **Pace and write** each sample at its natural PTS cadence relative to a
//!    wall-clock start instant — without pacing the whole file lands in the
//!    ring instantly and overflows it. Pacing is off the sample PTS delta,
//!    never a fixed per-sample sleep.
//!
//! # The loop
//!
//! At EOF with `loop_file: true`, the reader restarts from the file's
//! beginning and advances a **per-track PTS offset** so the looped content
//! continues on the same monotonic timeline: the first sample of loop N+1
//! carries the last sample of loop N's PTS plus one frame duration (in that
//! track's media timescale). The output PTS is therefore **strictly
//! monotonic across the loop point** — no reset to zero, no backwards step —
//! and the wall-clock cadence continues seamlessly because pacing is relative
//! to the same continuous `(pts + offset)` timeline. This is the property the
//! loop tests assert (and a mutation hides it; see
//! `multimux/tests/it/file_reader.rs`'s `loop_preserves_dts_monotonicity_across_boundary`).
//!
//! With `loop_file: false`, the reader stops after the first pass and reports
//! no further samples.
//!
//! # Supervision
//!
//! A read failure restarts the pass from the beginning of the file after a
//! fixed retry interval — deliberately simpler than the origin supervisor's
//! exponential backoff, which models a remote server that may recover. A local
//! file that fails to read fails identically on retry, so retries are **capped**
//! ([`FileReaderConfig::max_retries`]) and the final failure is surfaced rather
//! than spinning forever at growing intervals.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use media_plane::file_source::{
    FILE_TIMED_RING, FileIngestSession, FileReaderError, ParsedFile, PublishItem, advance_offsets,
    probe_and_demux, secs_to_duration,
};
use media_plane::trunk::{RetentionClass, Trunk, TrunkWriter};
use tokio::task::JoinHandle;
use transmux::TrackSpec;

/// Configuration for one [`FileReader`] run.
///
/// `#[non_exhaustive]`: adding a field (e.g. the size cap, which shipped as a
/// later hardening) must not be a breaking change — construct with
/// [`FileReaderConfig::new`] and tune with the builder methods.
#[non_exhaustive]
pub struct FileReaderConfig {
    /// Path to the media file to play.
    pub path: PathBuf,
    /// Restart from the beginning at EOF (`loop_file: true`) or stop after
    /// one pass (`false`).
    pub loop_file: bool,
    /// The [`Trunk`] this source announces tracks into and writes samples
    /// into.
    pub trunk: Arc<Trunk>,
    /// Whether to pace samples to a wall-clock start instant at their natural
    /// PTS cadence (`true`), or write them as fast as possible (`false`).
    /// The playout runtime paces; tests and the timeline invariant gate run
    /// slowly-but-deterministically-paced or un-paced.
    pub pace: bool,
    /// Cap on consecutive read/format failures before the reader gives up and
    /// surfaces the error. A local file that fails to read fails identically
    /// on retry, so retries are bounded, never exponential.
    pub max_retries: u32,
    /// Fixed interval between read-failure retries, in the spirit of the
    /// design spec's "fixed retry interval rather than exponential backoff".
    pub retry_interval: Duration,
    /// Cap on replay passes before the reader stops cleanly (`Some(n)`) — a
    /// test / operational control that keeps an otherwise-infinite `loop_file`
    /// reader bounded without changing its semantics. `None` (the runtime
    /// default) loops forever.
    pub max_loops: Option<u32>,
    /// Cap on the file's size in bytes, enforced **before** the read — an
    /// operator-supplied path (e.g. a character device or FIFO) is never read
    /// unbounded. Defaults to [`DEFAULT_MAX_FILE_BYTES`].
    pub max_file_bytes: usize,
}

/// The default [`FileReaderConfig::max_file_bytes`]: 1 GiB. Generous enough for
/// any real slate/filler/loop asset, small enough that a misconfigured path
/// fails cleanly ([`FileReaderError::FileTooLarge`]) long before an unbounded
/// read could exhaust process memory.
pub const DEFAULT_MAX_FILE_BYTES: usize = 1 << 30;

impl FileReaderConfig {
    /// Construct with sensible runtime defaults: pacing on, 3 read retries at
    /// 500 ms, no loop cap, and the [`DEFAULT_MAX_FILE_BYTES`] size cap. Use
    /// the `with_*` builders to tune the remaining knobs.
    pub fn new(path: PathBuf, loop_file: bool, trunk: Arc<Trunk>) -> Self {
        FileReaderConfig {
            path,
            loop_file,
            trunk,
            pace: true,
            max_retries: 3,
            retry_interval: Duration::from_millis(500),
            max_loops: None,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        }
    }

    /// Set [`Self::pace`].
    pub fn with_pace(mut self, pace: bool) -> Self {
        self.pace = pace;
        self
    }

    /// Set [`Self::max_retries`].
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Set [`Self::retry_interval`].
    pub fn with_retry_interval(mut self, retry_interval: Duration) -> Self {
        self.retry_interval = retry_interval;
        self
    }

    /// Set [`Self::max_loops`].
    pub fn with_max_loops(mut self, max_loops: Option<u32>) -> Self {
        self.max_loops = max_loops;
        self
    }

    /// Set [`Self::max_file_bytes`].
    pub fn with_max_file_bytes(mut self, max_file_bytes: usize) -> Self {
        self.max_file_bytes = max_file_bytes;
        self
    }
}

/// A [`FileReader`] that has been spawned onto a tokio task; the run's
/// terminal [`Result`] arrives on the wrapped [`JoinHandle`].
///
/// Owns a `tokio_util::sync::CancellationToken` (SP1.4): dropping this value CANCELS the run
/// rather than detaching it, so a reader whose owner goes away stops instead
/// of playing a file forever in the background. Awaiting [`Self::handle`]
/// still yields the outcome; a cancelled run resolves to
/// [`FileReaderError::Cancelled`].
#[non_exhaustive]
pub struct SpawnedReader {
    /// The tokio task's join handle — awaiting it yields the run's outcome.
    pub handle: JoinHandle<Result<(), FileReaderError>>,
    /// Cancels the run when this `SpawnedReader` is dropped.
    cancel: tokio_util::sync::CancellationToken,
}

impl SpawnedReader {
    /// Cancel the run now (dropping this value would do the same). Useful when
    /// the caller wants to stop the reader but still await
    /// [`Self::handle`] for its terminal outcome.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

impl Drop for SpawnedReader {
    fn drop(&mut self) {
        // Cancel, don't detach: the task must stop when its owner goes away.
        self.cancel.cancel();
    }
}

/// A standalone tokio task playing a media file on disk into a [`Trunk`] — see
/// the [module docs](self).
pub struct FileReader {
    config: FileReaderConfig,
}

impl FileReader {
    /// Construct a reader for `config`. Performs no I/O until [`Self::run`]/
    /// [`Self::spawn`] is called.
    pub fn new(config: FileReaderConfig) -> Self {
        FileReader { config }
    }

    /// Run the reader to completion on the current task: read+probe+demux the
    /// file (retrying read failures up to `max_retries`), then announce, pace,
    /// and write the samples, looping per `loop_file`/`max_loops`. Returns
    /// success when the pass(es) ended cleanly, or the first unrecoverable
    /// `FileReaderError`.
    pub async fn run(self) -> Result<(), FileReaderError> {
        // Read + probe + demux once (retriable: a local read failure could
        // plausibly clear), then publish (not retriable mid-pass).
        let demuxed = self.read_probe_demux().await?;
        self.publish_looping(demuxed).await
    }

    /// Spawn the reader as an independent tokio task; returns a
    /// [`SpawnedReader`] whose handle the caller may await for the outcome, and
    /// which cancels the run when dropped (SP1.4: cancel, not detach).
    pub fn spawn(self) -> SpawnedReader {
        let cancel = tokio_util::sync::CancellationToken::new();
        let run_cancel = cancel.clone();
        let handle = tokio::spawn(async move {
            tokio::select! {
                () = run_cancel.cancelled() => Err(FileReaderError::Cancelled),
                outcome = self.run() => outcome,
            }
        });
        SpawnedReader { handle, cancel }
    }

    /// One read+probe+demux attempt, retried on a [`FileReaderError::Read`]
    /// up to `max_retries` times at `retry_interval`.
    async fn read_probe_demux(&self) -> Result<ParsedFile, FileReaderError> {
        // A constant retry interval (factor 1.0), as before: the schedule is
        // the workspace's one backoff implementation, so even the flat case
        // goes through it (SP1.5).
        let schedule = broadcast_common::backoff::RetrySchedule::new(
            self.config.retry_interval,
            self.config.retry_interval,
            1.0,
            crate::source::seed_entropy(),
        );
        for attempt in 0..self.config.max_retries {
            match self.read_probe_demux_once().await {
                Ok(parsed) => return Ok(parsed),
                Err(FileReaderError::Read { .. }) => {
                    tokio::time::sleep(schedule.delay_for_attempt(attempt)).await;
                }
                Err(e) => return Err(e),
            }
        }
        // The final attempt's own error propagates (there is no attempt left
        // to sleep before).
        self.read_probe_demux_once().await
    }

    /// Read the whole file, probe it, and demux it into the playable tracks —
    /// no pacing, no publishing.
    async fn read_probe_demux_once(&self) -> Result<ParsedFile, FileReaderError> {
        // Metadata first — reject a non-regular file (character device /
        // FIFO / socket / directory) and an over-cap file *before* the read,
        // so an operator-supplied path can never trigger an unbounded read
        // that grows until the process dies.
        let metadata = tokio::fs::metadata(&self.config.path)
            .await
            .map_err(|source| FileReaderError::Read {
                path: self.config.path.clone(),
                source,
            })?;
        let kind = file_type_label(metadata.file_type());
        if !metadata.is_file() {
            return Err(FileReaderError::NotARegularFile {
                path: self.config.path.clone(),
                kind,
            });
        }
        let cap = self.config.max_file_bytes;
        if metadata.len() > cap as u64 {
            return Err(FileReaderError::FileTooLarge {
                path: self.config.path.clone(),
                size: metadata.len(),
                max: cap,
            });
        }

        let bytes =
            tokio::fs::read(&self.config.path)
                .await
                .map_err(|source| FileReaderError::Read {
                    path: self.config.path.clone(),
                    source,
                })?;

        // Identify + demux — whole file, never a prefix. Both are CPU-heavy
        // (a `container-probe` lattice search over the whole buffer, then a
        // full container demux) and synchronous, so they run on the blocking
        // pool (issue #1083, W3) rather than pinning a runtime worker on every
        // supervisor retry.
        tokio::task::spawn_blocking(move || probe_and_demux(&bytes))
            .await
            .map_err(|e| FileReaderError::Read {
                path: self.config.path.clone(),
                source: std::io::Error::other(format!("probe/demux task failed: {e}")),
            })?
    }

    /// Publish the parsed file into the trunk: announce tracks, then
    /// pace/write samples, looping per `loop_file`/`max_loops`.
    async fn publish_looping(&self, parsed: ParsedFile) -> Result<(), FileReaderError> {
        // `Trunk::writer` returns `None` on every call after the first — a
        // second reader over the same `Arc<Trunk>`, or a caller that already
        // took the writer, must fail as a structured error, never panic.
        let writer = self
            .config
            .trunk
            .writer()
            .ok_or(FileReaderError::WriterUnavailable)?;

        let (track_refs, order, pass_ranges) = parsed.build_playout();

        // A file with zero timed (PTS-carrying) samples cannot be paced,
        // looped, or written — fail rather than spin a no-op publish loop.
        if order.is_empty() {
            return Err(FileReaderError::NoPlayableSamples);
        }

        // The file's earliest presentation time (across all tracks) — the
        // pacing anchor. Each sample is due at the *advanced* start instant
        // plus `(due_seconds - file_start_secs)`, so the file begins playing
        // at the wall-clock start instant and continues at its natural cadence.
        let file_start_secs = order
            .iter()
            .map(|i| i.due_seconds)
            .fold(f64::INFINITY, f64::min);

        // One full pass's worth of wall time, in seconds: the largest span +
        // one frame across tracks (the same `PassRange` data the PTS offsets
        // advance by per track — one mechanism, not two). This is what makes
        // the pacing baseline *grow* each pass, so a looped sample's due
        // instant is never in the past the way an unadvanced start would make
        // it.
        let pass_duration_secs = pass_ranges
            .iter()
            .map(|r| r.span_secs + r.frame_secs)
            .fold(0.0f64, f64::max);

        announce_tracks(&writer, &track_refs);

        let mut start = if self.config.pace {
            Some(std::time::Instant::now())
        } else {
            None
        };

        let mut loop_index: u32 = 0;
        // Per-track accumulated PTS offset in the track's own media timescale.
        let mut offsets: Vec<i64> = vec![0i64; track_refs.len()];

        loop {
            publish_one_pass(
                &writer,
                &track_refs,
                &order,
                &offsets,
                start,
                file_start_secs,
            )
            .await?;

            loop_index += 1;
            let done =
                !self.config.loop_file || self.config.max_loops.is_some_and(|m| loop_index >= m);
            // Advance the per-track PTS offsets (checked). Then advance the
            // pacing baseline by one pass's wall span, mirroring the PTS
            // offset advance — pass N+1's due instants sit exactly one content
            // duration after pass N's, so pacing never dumps the whole file
            // and never spins with every target already in the past.
            advance_offsets(&mut offsets, &pass_ranges)?;
            if let Some(start) = &mut start {
                *start = start
                    .checked_add(secs_to_duration(pass_duration_secs)?)
                    .ok_or(FileReaderError::PacingOverflow {
                        value: pass_duration_secs,
                    })?;
            }
            if done {
                break;
            }
        }
        Ok(())
    }
}

/// Stable label for a [`std::fs::FileType`], for the [`FileReaderError`]
/// diagnostics — every non-file kind gets a distinct, named label (never a
/// catch-all string).
fn file_type_label(ft: std::fs::FileType) -> &'static str {
    if ft.is_dir() {
        "directory"
    } else if ft.is_symlink() {
        "symlink"
    } else if ft.is_file() {
        "file"
    } else {
        "non-regular (device/FIFO/socket/other)"
    }
}

/// Announce the track set on the trunk, seeding it before the first sample —
/// the same `set_tracks` call [`media_plane::ingress::IngestDriver`] makes for
/// `SessionEvent::NewProgram`.
fn announce_tracks(writer: &TrunkWriter, specs: &[TrackSpec]) {
    writer.set_tracks(specs.to_vec());
}

/// Write one pass (one replay of the whole file): for each sample in
/// presentation order, sleep until its PTS cadence (if pacing) and publish it
/// with the current per-track loop offset applied.
async fn publish_one_pass(
    writer: &TrunkWriter,
    track_refs: &[TrackSpec],
    order: &[PublishItem],
    offsets: &[i64],
    start: Option<std::time::Instant>,
    file_start_secs: f64,
) -> Result<(), FileReaderError> {
    for item in order {
        let offset = offsets[item.track_ref];
        if let Some(start) = start {
            // Pace off the sample's own presentation delta: `due = start +
            // (due_seconds - file_start_secs)`. Because the loop offset
            // advances absolute PTS, this delta grows continuously across
            // the loop point — the looped content plays at its natural cadence
            // rather than dumping the whole file at once.
            let secs = (item.due_seconds - file_start_secs).max(0.0);
            let delta = secs_to_duration(secs)?;
            let due = start
                .checked_add(delta)
                .ok_or(FileReaderError::PacingOverflow {
                    value: item.due_seconds,
                })?;
            let now = std::time::Instant::now();
            if due > now {
                tokio::time::sleep(due - now).await;
            }
        }
        let mut sample = item.sample.clone();
        if let Some(pts) = sample.timing.map(|t| t.pts) {
            if let Some(t) = sample.timing.as_mut() {
                t.pts = pts
                    .checked_add(offset)
                    .ok_or(FileReaderError::OffsetOverflow)?;
            }
        }
        if let Some(dts) = sample.timing.map(|t| t.dts) {
            if let Some(t) = sample.timing.as_mut() {
                t.dts = dts
                    .checked_add(offset)
                    .ok_or(FileReaderError::OffsetOverflow)?;
            }
        }
        writer.publish(
            track_refs[item.track_ref].track_id,
            RetentionClass::Timed,
            sample,
        );
    }
    // Yield once per pass so a `pace: false` + `loop_file: true` reader (the
    // public `with_pace(false)` builder with the default `max_loops: None`)
    // does not busy-spin its worker forever: with pacing off there is no
    // sleep inside the loop, so without this `.await` the pass-boundary is a
    // zero-cost hot loop. A single yield keeps the task fair while preserving
    // the un-paced "write as fast as possible" contract.
    tokio::task::yield_now().await;
    Ok(())
}

/// The [`FileReaderConfig`] builder / constructor, kept small now that WP2
/// wires reader construction; later WPs (controller) reuse it.
impl FileReader {
    /// Convenience constructor with pacing and retries on, `loop_file` as
    /// given, and no loop cap.
    pub fn standard(path: PathBuf, loop_file: bool, trunk: Arc<Trunk>) -> Self {
        FileReader::new(FileReaderConfig::new(path, loop_file, trunk))
    }
}

// ---------------------------------------------------------------------------
// Route wiring (issue #748 WP5) — the file route served through the shared
// driver/segmenter machinery.
//
// The standalone [`FileReader`] above is a plain task that writes a Trunk
// directly; a *route* needs its output turned into servable segments, which is
// `crate::source::advance_route`'s job — and that facade enumerates programs
// from an [`media_plane::ingress::IngestDriver`], not a bare Trunk. So the
// route mints a single program through a `FileIngestSession` (the one thin
// adapter the driver understands), whose demux is the very same
// [`FileReader::read_probe_demux`] path — one demux, two entry points.
// ---------------------------------------------------------------------------

use media_plane::ingress::IngestDriver;

/// The `Trunk` configuration of a file route.
fn file_trunk_config(window_segments: usize) -> media_plane::trunk::TrunkConfig {
    let nz =
        |n: usize| std::num::NonZeroUsize::new(n.max(1)).unwrap_or(std::num::NonZeroUsize::MIN);
    let live = crate::source::driver_trunk_config(window_segments);
    media_plane::trunk::TrunkConfig::default()
        .with_timed_capacity(nz(FILE_TIMED_RING))
        .with_sparse_capacity(live.sparse_capacity)
        .with_segment_capacity(live.segment_capacity)
        .with_event_capacity(live.event_capacity)
        .with_part_capacity(live.part_capacity)
}

/// Run a `InputSpec::File` route to completion on the current task: demux the
/// file, replay it through a single-program [`IngestDriver`] over a
/// [`FileIngestSession`], and pump `crate::source::advance_route` once per
/// iteration so samples become servable segments/parts. Mirrors
/// [`crate::source::ts_udp::run_ts_udp`]'s shape; returns `Err` only on a
/// failure (a bad/unreadable file), which the route's supervisor retries.
///
/// The drive loop is **paced**: it sleeps until the next sample is due on the
/// wall clock (`FileIngestSession::next_due`), never a fixed 10 ms spin — the
/// earlier fixed-tick loop dumped the whole file every tick, ~300× realtime.
/// A finite `loop_file: false` file that has fully drained parks (zero CPU)
/// holding its trunk, so its served segments stay servable until shutdown
/// rather than spinning or triggering a supervisor re-serve.
pub(crate) async fn run_file_source(
    path: &str,
    loop_file: bool,
    window_segments: usize,
    handshake: media_plane::ingress::HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
) -> crate::MultimuxError {
    // Demux the file eagerly (the same probe->demux path the standalone
    // FileReader uses); a failure surfaces through the supervisor's retry.
    // `FileReaderConfig` wants a `Trunk` to publish into; `read_probe_demux`
    // never publishes, so the smallest possible one is all this needs.
    let one = std::num::NonZeroUsize::MIN;
    let scratch = media_plane::trunk::Trunk::new(
        media_plane::trunk::TrunkConfig::default()
            .with_timed_capacity(one)
            .with_sparse_capacity(one)
            .with_segment_capacity(one)
            .with_event_capacity(one)
            .with_part_capacity(one),
    );
    let reader = FileReader::new(
        FileReaderConfig::new(std::path::PathBuf::from(path), loop_file, scratch)
            .with_pace(false)
            .with_max_retries(0)
            .with_retry_interval(std::time::Duration::ZERO),
    );
    let parsed = match reader.read_probe_demux().await {
        Ok(p) => p,
        Err(e) => return e.into(),
    };

    let trunk_config = file_trunk_config(window_segments);

    let session =
        match FileIngestSession::new(parsed, crate::route::SPTS_PROGRAM_ID, loop_file, true) {
            Ok(s) => s,
            Err(e) => return e.into(),
        };
    let mut driver = IngestDriver::new(
        session,
        media_plane::IngestConfig::new(handshake)
            .with_trunk(trunk_config)
            .with_max_programs(
                std::num::NonZeroUsize::new(1).unwrap_or(std::num::NonZeroUsize::MIN),
            ),
    );
    let start = std::time::Instant::now();
    let mut progress = crate::source::DriverProgress::new();
    loop {
        let now = broadcast_common::Timestamp::from_instant(start, std::time::Instant::now());
        // Feed the session: on the first feed it emits Established + NewProgram
        // (minting the program's Trunk), on later feeds it emits the samples —
        // so `advance_route` below creates the segmenter *between* those two
        // phases, before any sample lands.
        driver.feed((), now);
        crate::source::advance_route(&driver, route_handle, &mut progress).await;

        if driver.session().drained() {
            // Finite, non-looping file has fully run. Signal end-of-input so
            // the segmenter FLUSHES its tail — the segmenter's `flush()` is
            // gated on `!driver.health().is_running()`, which only becomes
            // true once the driver is `Ended`, so without this a parked-but
            // healthy driver would silently drop up to one `target_duration`
            // of media. `driver.finish()` sets `Ended`, then one more
            // `advance_route` drains the final segment before we park (zero
            // CPU) holding the trunk so the served segments stay servable.
            driver.finish();
            crate::source::advance_route(&driver, route_handle, &mut progress).await;
            std::future::pending::<()>().await;
        }

        // Pace: sleep until the next sample is due, rather than polling at 100 Hz.
        match driver.session().next_due() {
            Some(due) => {
                let now = std::time::Instant::now();
                if due > now {
                    tokio::time::sleep(due - now).await;
                } else {
                    // Already due (a burst is being worked off in bounded
                    // drains): let other tasks run between drains.
                    tokio::task::yield_now().await;
                }
            }
            // Loop refill happens on the *next* `feed` (top of this loop), so
            // this arm is hit once per pass, not per iteration. Yield rather
            // than sleep a fixed 1 ms: `feed` refills immediately, and a
            // non-looping drained file is already parked by `drained()` above.
            // (Parking on the refill notification is NOT safe here: the feed
            // that would fire it only runs after this arm, so it would never
            // be woken in the loop-refill case.)
            None => tokio::task::yield_now().await,
        }
    }
}

/// Unit tests for the playout invariants that need synthetic tracks (negative
/// PTS, zero/absent duration, absurdly large deltas, no timed samples) — none
/// of which a real fixture produces, so each is built from the public
/// [`transmux`] IR and driven through the private `build_playout`/
/// `advance_offsets`/`FileIngestSession::new` paths directly.
#[cfg(test)]
mod playout_tests {
    use super::*;
    use transmux::{Sample, Track};

    /// I8/SP1.4: dropping a [`SpawnedReader`] CANCELS the run rather than
    /// detaching it — the task must not keep playing a file forever after its
    /// owner is gone. PRE-FIX: `SpawnedReader { handle }` had no cancel token,
    /// so dropping it detached the task and this would time out (the run keeps
    /// going), not resolve to `Cancelled`.
    #[tokio::test]
    async fn dropping_a_spawned_reader_cancels_the_run() {
        let fixture = test_fixtures::path("ts/h264-aac.ts");
        let trunk = media_plane::trunk::Trunk::new(crate::source::driver_trunk_config(8));
        let reader = FileReader::new(
            FileReaderConfig::new(fixture, true, trunk)
                .with_pace(true)
                .with_max_retries(0),
        );
        let mut spawned = reader.spawn();
        // Cancel (as a drop would): the run must stop, and awaiting the handle
        // must yield `Cancelled` rather than the run completing.
        spawned.cancel();
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), &mut spawned.handle)
            .await
            .expect("a cancelled reader must not run forever")
            .expect("the task must not panic");
        assert!(
            matches!(outcome, Err(FileReaderError::Cancelled)),
            "cancelling the SpawnedReader must cancel the run, got {outcome:?}"
        );
        drop(spawned);
    }

    /// I8/SP1.4 (m1): dropping a [`SpawnedReader`] WITHOUT calling
    /// [`SpawnedReader::cancel`] explicitly must still cancel the run, purely
    /// via `impl Drop`. The task deliberately loops the fixture
    /// (`loop_file = true`), so a merely-detached task would run forever; the
    /// test drops the value and waits (bounded) for the task to finish,
    /// observed through an abort handle taken before the drop. Deleting
    /// `impl Drop for SpawnedReader` makes the task run forever and times out.
    #[tokio::test]
    async fn dropping_a_spawned_reader_without_cancel_stops_the_run() {
        let fixture = test_fixtures::path("ts/h264-aac.ts");
        let trunk = media_plane::trunk::Trunk::new(crate::source::driver_trunk_config(8));
        let reader = FileReader::new(
            // `loop_file = true`: a detached task would never end on its own.
            FileReaderConfig::new(fixture, true, trunk)
                .with_pace(true)
                .with_max_retries(0),
        );
        let spawned = reader.spawn();
        // Observe the task's completion without owning the handle after drop.
        let abort = spawned.handle.abort_handle();
        // Drop the SpawnedReader itself (no explicit `cancel()`); `impl Drop`
        // must cancel the run.
        drop(spawned);

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        while !abort.is_finished() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "dropping the SpawnedReader must cancel the looping run, not detach it"
            );
            tokio::task::yield_now().await;
        }
    }

    /// Finding 8 (standalone reader half): `publish_looping` rejects a file
    /// with no timed samples before spinning.
    #[tokio::test]
    async fn publish_looping_rejects_no_playable_samples() {
        let cfg = crate::source::driver_trunk_config(8);
        let trunk = media_plane::trunk::Trunk::new(cfg);
        let reader = FileReader::new(
            FileReaderConfig::new(std::path::PathBuf::from("unused"), true, trunk).with_pace(false),
        );
        let samples = vec![Sample::new(vec![0], None, None, None, false)];
        let parsed = ParsedFile {
            tracks: vec![Track::new(
                media_plane::ts_session::test_support::track_spec(transmux::TrackId::new(1)),
                samples,
            )],
        };
        let err = reader.publish_looping(parsed).await;
        assert!(
            matches!(err, Err(FileReaderError::NoPlayableSamples)),
            "publish_looping must fail with NoPlayableSamples"
        );
    }

    /// The ring depth does not depend on the file or the window.
    #[test]
    fn the_file_ring_is_independent_of_the_file_length() {
        assert_eq!(file_trunk_config(8).timed_capacity.get(), FILE_TIMED_RING);
        assert_eq!(file_trunk_config(500).timed_capacity.get(), FILE_TIMED_RING);
        assert_eq!(file_trunk_config(500).segment_capacity.get(), 500);
    }
}
