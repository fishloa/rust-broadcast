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
//! The file is identified from a bounded leading prefix, then played down one
//! of two paths by container:
//!
//! * **TS, FLV and MP4 stream.** [`media_plane::file_stream::StreamingFileSession`]
//!   drives an incremental demuxer; the reader answers its forward chunks
//!   (TS/FLV) or absolute byte ranges (MP4) until it asks for end of input,
//!   then rewinds to offset 0 for the next pass. Live memory is a function of
//!   the session's [`FileStreamLimits`] and never of the file's length, so
//!   [`FileReaderConfig::max_file_bytes`] does not apply.
//! * **PS and WebM keep today's whole-file path.** The whole file is read
//!   under the [`FileReaderConfig::max_file_bytes`] cap, the whole-buffer
//!   demuxers ([`transmux::PsDemux`], [`transmux::WebmDemux`]) run once over
//!   the buffer, and the parsed tracks are replayed.
//!
//! 1. **Identify** the container from a bounded prefix with
//!    [`media_plane::file_source::probe_prefix`]. The verdict maps to a
//!    [`transmux`] demuxer exactly per the design spec's table; an
//!    `Ambiguous`/`Insufficient`/`Unknown` verdict, or a format [`transmux`]
//!    cannot demux as a playout source, **fails the source** — a file is never
//!    fed to a guessed demuxer, and the ISOBMFF layout is never re-derived
//!    here (the probe already did it; two implementations of one rule that can
//!    disagree is the bug class this workspace keeps finding).
//! 2. **Demux and play** down the path the container selects (above).
//! 3. **Announce the tracks** on the Trunk the same way every other ingest
//!    does ([`TrunkWriter::set_tracks`]), **before** the first sample — and
//!    again on any loop where the codec configuration differs.
//! 4. **Pace and write** each sample at its natural PTS cadence relative to a
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
//! The streaming path re-opens and re-probes the file for each pass: a file
//! replaced by a different container is
//! [`FileReaderError::FileChangedContainer`], while a longer file simply plays
//! longer.
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

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Default attempts a missing file is retried.
const DEFAULT_MAX_RETRIES: u32 = 3;

/// Default interval between those attempts.
const DEFAULT_RETRY_INTERVAL: Duration = Duration::from_millis(500);

/// Bytes read from the file per streaming feed: 64 KiB. Large enough that the
/// demuxer always has whole TS packets or FLV tags to work on, small enough
/// that the read buffer never dominates a bounded-memory playout.
const FILE_READ_CHUNK: usize = 64 << 10;

/// The program a standalone [`FileReader`] names in the session events it
/// consumes itself — the caller-supplied [`Trunk`] carries the tracks and
/// samples, so the id never reaches an output.
const STANDALONE_PROGRAM: media_plane::ingress::ProgramId = media_plane::ingress::ProgramId(0);

use broadcast_common::Stage as _;
use container_probe::DEFAULT_BUDGET;
use media_plane::file_source::{
    DemuxerKind, FILE_TIMED_RING, FileIngestSession, FileReaderError, PROBE_PREFIX_MAX, ParsedFile,
    ProbeStep, PublishItem, advance_offsets, probe_and_demux, probe_prefix, secs_to_duration,
};
use media_plane::file_stream::{
    FileDemand, FileInput, FileStreamLimits, StreamContainer, StreamingFileSession,
};
use media_plane::ingress::SessionEvent;
use media_plane::trunk::{RetentionClass, Trunk, TrunkWriter};
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _};
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
    /// Cap on the file's size in bytes, enforced **before** a whole-file read
    /// — an operator-supplied path (e.g. a character device or FIFO) is never
    /// read unbounded. Applies to PS and WebM files only: TS, FLV and MP4
    /// stream from disk under bounded buffers, so their size is never capped.
    /// Defaults to [`DEFAULT_MAX_FILE_BYTES`].
    pub max_file_bytes: usize,
}

/// The default [`FileReaderConfig::max_file_bytes`]: 1 GiB. Generous enough for
/// any real slate/filler/loop asset, small enough that a misconfigured path
/// fails cleanly ([`FileReaderError::FileTooLarge`]) long before an unbounded
/// read could exhaust process memory. TS, FLV and MP4 stream from disk and are
/// not subject to it.
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
            max_retries: DEFAULT_MAX_RETRIES,
            retry_interval: DEFAULT_RETRY_INTERVAL,
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

    /// Set [`Self::max_file_bytes`], the whole-file cap that applies to PS and
    /// WebM only.
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

    /// Run the reader to completion on the current task, retrying a read
    /// failure up to `max_retries` at `retry_interval`: the pass restarts from
    /// the beginning of the file, then announces, paces, and writes the
    /// samples, looping per `loop_file`/`max_loops`. Returns success when the
    /// pass(es) ended cleanly, or the first unrecoverable `FileReaderError`.
    pub async fn run(self) -> Result<(), FileReaderError> {
        let schedule = broadcast_common::backoff::RetrySchedule::new(
            self.config.retry_interval,
            self.config.retry_interval,
            1.0,
            crate::source::seed_entropy(),
        )
        .ok();
        for attempt in 0..self.config.max_retries {
            match self.run_once().await {
                Ok(()) => return Ok(()),
                Err(FileReaderError::Read { .. }) => {
                    let interval = schedule
                        .as_ref()
                        .map_or(self.config.retry_interval, |s| s.delay_for_attempt(attempt));
                    tokio::time::sleep(interval).await;
                }
                Err(e) => return Err(e),
            }
        }
        self.run_once().await
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

    /// One open/probe/play attempt: TS, FLV and MP4 stream from disk, PS and
    /// WebM take the whole-file path.
    async fn run_once(&self) -> Result<(), FileReaderError> {
        let probed = open_probed(&self.config.path).await?;
        match stream_container(probed.demuxer) {
            Some(container) => self.publish_streaming(probed, container).await,
            None => {
                let parsed = self.read_capped_whole(probed).await?;
                self.publish_looping(parsed).await
            }
        }
    }

    /// Read the whole file, probe it, and demux it into the playable tracks —
    /// the capped path for PS and WebM.
    async fn read_capped_whole(&self, probed: OpenFile) -> Result<ParsedFile, FileReaderError> {
        let cap = self.config.max_file_bytes;
        if probed.len > cap as u64 {
            return Err(FileReaderError::FileTooLarge {
                path: self.config.path.clone(),
                size: probed.len,
                max: cap,
            });
        }

        let path = self.config.path.clone();
        let bytes = read_capped(&path, probed.file, cap).await?;

        // Both are CPU-heavy (a `container-probe` lattice search over the whole
        // buffer, then a full container demux) and synchronous, so they run on
        // the blocking pool (issue #1083, W3) rather than pinning a runtime
        // worker on every supervisor retry.
        tokio::task::spawn_blocking(move || probe_and_demux(&bytes))
            .await
            .map_err(|e| FileReaderError::Read {
                path: self.config.path.clone(),
                source: std::io::Error::other(format!("probe/demux task failed: {e}")),
            })?
    }

    /// Play a streaming file by feeding [`StreamingFileSession`] the bytes it
    /// asks for, one bounded read at a time — the file is never held whole.
    async fn publish_streaming(
        &self,
        probed: OpenFile,
        container: StreamContainer,
    ) -> Result<(), FileReaderError> {
        let writer = self
            .config
            .trunk
            .writer()
            .ok_or(FileReaderError::WriterUnavailable)?;
        let limits = FileStreamLimits {
            mp4: mp4_config(probed.len),
            ..FileStreamLimits::default()
        };
        let mut session = StreamingFileSession::new(
            container,
            STANDALONE_PROGRAM,
            self.config.loop_file,
            self.config.max_loops,
            self.config.pace,
            limits,
        );
        let mut file = probed.file;
        let mut buffer = vec![0u8; FILE_READ_CHUNK];

        while !session.drained() {
            match session.file_demand() {
                FileDemand::Bytes => {
                    let read = read_chunk(&self.config.path, &mut file, &mut buffer).await?;
                    feed_read(&mut session, &buffer[..read])?;
                }
                FileDemand::BytesAt { offset, len } => {
                    let bytes = read_at(&self.config.path, &mut file, offset, len).await?;
                    session.feed(
                        FileInput::At {
                            offset,
                            bytes: bytes.into(),
                        },
                        broadcast_common::Timestamp::ZERO,
                    )?;
                }
                FileDemand::Eof => {
                    session.feed(FileInput::Eof, broadcast_common::Timestamp::ZERO)?;
                }
                FileDemand::Rewind => {
                    let next = reprobe_stream_file(&self.config.path, container).await?;
                    file = next.file;
                    if container == StreamContainer::Mp4 {
                        session.rewound();
                    } else {
                        let read = read_chunk(&self.config.path, &mut file, &mut buffer).await?;
                        feed_read(&mut session, &buffer[..read])?;
                    }
                }
                FileDemand::Wait => {
                    if let Some(due) = session.next_due() {
                        let now = std::time::Instant::now();
                        if due > now {
                            tokio::time::sleep(due - now).await;
                        } else {
                            tokio::task::yield_now().await;
                        }
                    } else {
                        tokio::task::yield_now().await;
                    }
                }
                _ => break,
            }
            while let Some(event) = session.poll() {
                publish_standalone(&writer, event);
            }
        }
        Ok(())
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
            .map(|r| (r.span + r.frame).as_secs_f64())
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

/// One opened and identified playout file, positioned at offset 0.
struct OpenFile {
    file: tokio::fs::File,
    len: u64,
    demuxer: DemuxerKind,
}

/// The streaming container a probed demuxer selects, or `None` for the
/// containers that still take the whole-file path.
fn stream_container(demuxer: DemuxerKind) -> Option<StreamContainer> {
    match demuxer {
        DemuxerKind::StreamingTs => Some(StreamContainer::MpegTs),
        DemuxerKind::StreamingFlv => Some(StreamContainer::Flv),
        DemuxerKind::Fmp4 | DemuxerKind::Progressive | DemuxerKind::Mp4 => {
            Some(StreamContainer::Mp4)
        }
        _ => None,
    }
}

/// Open `path`, reject anything that is not a regular file, and identify its
/// container from a bounded leading prefix.
async fn open_probed(path: &Path) -> Result<OpenFile, FileReaderError> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|source| read_error(path, source))?;
    let metadata = file
        .metadata()
        .await
        .map_err(|source| read_error(path, source))?;
    if !metadata.is_file() {
        return Err(FileReaderError::NotARegularFile {
            path: path.to_path_buf(),
            kind: file_type_label(metadata.file_type()),
        });
    }
    let len = metadata.len();
    let demuxer = probe_from_prefix(path, &mut file, len).await?;
    file.seek(std::io::SeekFrom::Start(0))
        .await
        .map_err(|source| read_error(path, source))?;
    Ok(OpenFile { file, len, demuxer })
}

/// Re-open and re-identify `path` for the next loop pass, failing when the
/// file has become a different container.
async fn reprobe_stream_file(
    path: &Path,
    expected: StreamContainer,
) -> Result<OpenFile, FileReaderError> {
    let open = open_probed(path).await?;
    match stream_container(open.demuxer) {
        Some(container) if container == expected => Ok(open),
        _ => Err(FileReaderError::FileChangedContainer {
            path: path.to_path_buf(),
        }),
    }
}

/// Drive [`probe_prefix`] over the open file, growing the prefix from
/// [`DEFAULT_BUDGET`] until the container is identified or the probe cap is
/// reached.
async fn probe_from_prefix(
    path: &Path,
    file: &mut tokio::fs::File,
    len: u64,
) -> Result<DemuxerKind, FileReaderError> {
    let limit = usize::try_from(len).unwrap_or(usize::MAX);
    let mut prefix = Vec::new();
    let mut want = limit.min(DEFAULT_BUDGET);
    loop {
        fill_prefix(path, file, &mut prefix, want).await?;
        match probe_prefix(&prefix, len)? {
            ProbeStep::Identified(demuxer) => return Ok(demuxer),
            ProbeStep::NeedPrefix { prefix_len } => {
                want = prefix_len.min(PROBE_PREFIX_MAX);
            }
            _ => return Err(FileReaderError::UnknownProbe),
        }
    }
}

/// Read into `prefix` until it holds `want` bytes or the file ends.
async fn fill_prefix(
    path: &Path,
    file: &mut tokio::fs::File,
    prefix: &mut Vec<u8>,
    want: usize,
) -> Result<(), FileReaderError> {
    while prefix.len() < want {
        let start = prefix.len();
        prefix.resize(want, 0);
        let read = file
            .read(&mut prefix[start..])
            .await
            .map_err(|source| read_error(path, source))?;
        prefix.truncate(start + read);
        if read == 0 {
            break;
        }
    }
    Ok(())
}

/// Read up to one [`FILE_READ_CHUNK`]; a zero-length read is end of file.
async fn read_chunk(
    path: &Path,
    file: &mut tokio::fs::File,
    buffer: &mut [u8],
) -> Result<usize, FileReaderError> {
    file.read(buffer)
        .await
        .map_err(|source| read_error(path, source))
}

/// The MP4 stage policy for a file of `total_len` bytes: a caller-chosen
/// total length so a `moov`-after-`mdat` input can be read and end of input
/// recognised. The read window stays unset (one sample per request).
fn mp4_config(total_len: u64) -> transmux::mp4_demux::Mp4DemuxConfig {
    transmux::mp4_demux::Mp4DemuxConfig::default().with_total_len(Some(total_len))
}

/// Read exactly `len` bytes at absolute `offset`, or fewer only at end of
/// file: one seek and one bounded buffer, never a read to the end.
async fn read_at(
    path: &Path,
    file: &mut tokio::fs::File,
    offset: u64,
    len: usize,
) -> Result<Vec<u8>, FileReaderError> {
    file.seek(std::io::SeekFrom::Start(offset))
        .await
        .map_err(|source| read_error(path, source))?;
    let mut buffer = vec![0u8; len];
    let mut filled = 0usize;
    while filled < len {
        let read = file
            .read(&mut buffer[filled..])
            .await
            .map_err(|source| read_error(path, source))?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    buffer.truncate(filled);
    Ok(buffer)
}

/// Feed a read: the bytes just read, or end of input for a zero-length read.
fn feed_read(session: &mut StreamingFileSession, bytes: &[u8]) -> Result<(), FileReaderError> {
    if bytes.is_empty() {
        session.feed(FileInput::Eof, broadcast_common::Timestamp::ZERO)
    } else {
        session.feed(FileInput::Bytes(bytes), broadcast_common::Timestamp::ZERO)
    }
}

/// Publish one session event into the standalone reader's caller-supplied
/// trunk: a track announcement, or a sample.
fn publish_standalone(writer: &TrunkWriter, event: SessionEvent) {
    match event {
        SessionEvent::NewProgram { tracks, .. } => announce_tracks(writer, &tracks),
        SessionEvent::Sample {
            track_id,
            retention,
            sample,
            ..
        } => writer.publish(track_id, retention, sample),
        _ => {}
    }
}

fn read_error(path: &Path, source: std::io::Error) -> FileReaderError {
    FileReaderError::Read {
        path: path.to_path_buf(),
        source,
    }
}

/// Read an already-opened file through a `cap + 1` window, so a file that
/// grew past the cap after the metadata check is still refused instead of
/// held in memory unbounded.
async fn read_capped(
    path: &std::path::Path,
    file: tokio::fs::File,
    cap: usize,
) -> Result<Vec<u8>, FileReaderError> {
    use tokio::io::AsyncReadExt as _;

    let mut bytes = Vec::new();
    file.take((cap as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .await
        .map_err(|source| FileReaderError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() > cap {
        return Err(FileReaderError::FileTooLarge {
            path: path.to_path_buf(),
            size: bytes.len() as u64,
            max: cap,
        });
    }
    Ok(bytes)
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
        if let Some(t) = sample.timing.as_mut() {
            let current = t.dts().unwrap_or(0);
            let shifted = current
                .checked_add(offset)
                .ok_or(FileReaderError::OffsetOverflow)?;
            t.shift(shifted - current);
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

/// Run a `InputSpec::File` route to completion on the current task: identify
/// the file from a bounded prefix, then replay it through a single-program
/// [`IngestDriver`] and pump `crate::source::advance_route` once per iteration
/// so samples become servable segments/parts. TS, FLV and MP4 stream from disk
/// through a [`StreamingFileSession`]; PS and WebM keep the whole-file
/// [`FileIngestSession`] path under the configured size cap. Mirrors
/// [`crate::source::ts_udp::run_ts_udp`]'s shape; returns `Err` only on a
/// failure (a bad/unreadable file), which the route's supervisor retries.
///
/// Both drive loops are **paced**: they sleep until the next sample is due on
/// the wall clock, never a fixed 10 ms spin — the earlier fixed-tick loop
/// dumped the whole file every tick, ~300× realtime. A finite `loop_file:
/// false` file that has fully drained parks (zero CPU) holding its trunk, so
/// its served segments stay servable until shutdown rather than spinning or
/// triggering a supervisor re-serve.
pub(crate) async fn run_file_source(
    path: &str,
    loop_file: bool,
    window_segments: usize,
    handshake: media_plane::ingress::HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
) -> crate::MultimuxError {
    // `FileReaderConfig` wants a `Trunk` to publish into; the identification
    // helpers never publish, so the smallest possible one is all this needs.
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
    let probed = match open_probed(std::path::Path::new(path)).await {
        Ok(probed) => probed,
        Err(e) => return e.into(),
    };

    match stream_container(probed.demuxer) {
        Some(container) => {
            run_streaming_file_route(
                std::path::Path::new(path),
                probed,
                container,
                loop_file,
                window_segments,
                handshake,
                route_handle,
            )
            .await
        }
        None => {
            let parsed = match reader.read_capped_whole(probed).await {
                Ok(parsed) => parsed,
                Err(e) => return e.into(),
            };
            run_whole_file_route(parsed, loop_file, window_segments, handshake, route_handle).await
        }
    }
}

/// The whole-file route: replay an already-demuxed PS/WebM file through a
/// [`FileIngestSession`].
async fn run_whole_file_route(
    parsed: ParsedFile,
    loop_file: bool,
    window_segments: usize,
    handshake: media_plane::ingress::HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
) -> crate::MultimuxError {
    let session =
        match FileIngestSession::new(parsed, crate::route::SPTS_PROGRAM_ID, loop_file, true) {
            Ok(s) => s,
            Err(e) => return e.into(),
        };
    let mut driver = IngestDriver::new(
        session,
        media_plane::IngestConfig::new(handshake)
            .with_trunk(file_trunk_config(window_segments))
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

/// The streaming route: read a TS/FLV/MP4 file from disk in bounded chunks or
/// byte ranges and drive the session through the route's [`IngestDriver`].
async fn run_streaming_file_route(
    path: &std::path::Path,
    probed: OpenFile,
    container: StreamContainer,
    loop_file: bool,
    window_segments: usize,
    handshake: media_plane::ingress::HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
) -> crate::MultimuxError {
    let mut driver = IngestDriver::new(
        StreamingFileIngest::new(container, loop_file, probed.len),
        media_plane::IngestConfig::new(handshake)
            .with_trunk(file_trunk_config(window_segments))
            .with_max_programs(
                std::num::NonZeroUsize::new(1).unwrap_or(std::num::NonZeroUsize::MIN),
            ),
    );
    let start = std::time::Instant::now();
    let mut progress = crate::source::DriverProgress::new();
    let mut file = probed.file;
    let mut buffer = vec![0u8; FILE_READ_CHUNK];

    loop {
        let now = broadcast_common::Timestamp::from_instant(start, std::time::Instant::now());
        match driver.session().file_demand() {
            FileDemand::Bytes => {
                let read = match read_chunk(path, &mut file, &mut buffer).await {
                    Ok(read) => read,
                    Err(error) => return error.into(),
                };
                feed_route(&mut driver, &buffer[..read], now);
            }
            FileDemand::BytesAt { offset, len } => {
                let bytes = match read_at(path, &mut file, offset, len).await {
                    Ok(bytes) => bytes,
                    Err(error) => return error.into(),
                };
                driver.feed(
                    FileInput::At {
                        offset,
                        bytes: bytes.into(),
                    },
                    now,
                );
            }
            FileDemand::Eof => driver.feed(FileInput::Eof, now),
            FileDemand::Rewind => {
                let next = match reprobe_stream_file(path, container).await {
                    Ok(next) => next,
                    Err(error) => return error.into(),
                };
                file = next.file;
                if container == StreamContainer::Mp4 {
                    driver.session_mut().rewound();
                } else {
                    let read = match read_chunk(path, &mut file, &mut buffer).await {
                        Ok(read) => read,
                        Err(error) => return error.into(),
                    };
                    feed_route(&mut driver, &buffer[..read], now);
                }
            }
            FileDemand::Wait => {
                match driver.session().next_due() {
                    Some(due) => {
                        let now = std::time::Instant::now();
                        if due > now {
                            tokio::time::sleep(due - now).await;
                        }
                        // Even an already-due front needs this drain every
                        // iteration: the driver only polls its session on a
                        // feed, a deadline or a finish.
                        driver.on_deadline(now_timestamp(start));
                    }
                    None => {
                        driver.on_deadline(now_timestamp(start));
                        tokio::task::yield_now().await;
                    }
                }
            }
            _ => {}
        }
        crate::source::advance_route(&driver, route_handle, &mut progress).await;

        if driver.session().drained() {
            // The finite, non-looping file has fully run: flush the
            // segmenter's tail and park (zero CPU) holding the trunk, so the
            // served segments stay servable until shutdown.
            driver.finish();
            crate::source::advance_route(&driver, route_handle, &mut progress).await;
            std::future::pending::<()>().await;
        }

        if let media_plane::ingress::HealthState::Failed(_) = driver.health() {
            return terminal_health(driver);
        }
    }
}

/// Feed one read into the route's session: the bytes, or end of input for a
/// zero-length read.
fn feed_route(
    driver: &mut IngestDriver<StreamingFileIngest>,
    bytes: &[u8],
    now: broadcast_common::Timestamp,
) {
    let input = if bytes.is_empty() {
        FileInput::Eof
    } else {
        FileInput::Bytes(bytes)
    };
    driver.feed(input, now);
}

fn now_timestamp(start: std::time::Instant) -> broadcast_common::Timestamp {
    broadcast_common::Timestamp::from_instant(start, std::time::Instant::now())
}

/// Surface a driver's terminal failure as the route's typed error.
fn terminal_health(driver: IngestDriver<StreamingFileIngest>) -> crate::MultimuxError {
    match driver.into_health() {
        media_plane::ingress::HealthState::Failed(error) => error.into(),
        other => crate::MultimuxError::Connect {
            reason: format!("file session ended: {:?}", std::mem::discriminant(&other)),
        },
    }
}

/// A [`StreamingFileSession`] as the [`IngestSession`] a file route's
/// [`IngestDriver`] pumps: the session's own demand and next-due instant are
/// the file-I/O schedule the route loop follows.
struct StreamingFileIngest {
    session: StreamingFileSession,
}

impl StreamingFileIngest {
    fn new(container: StreamContainer, loop_file: bool, total_len: u64) -> Self {
        let limits = FileStreamLimits {
            mp4: mp4_config(total_len),
            ..FileStreamLimits::default()
        };
        let session = StreamingFileSession::new(
            container,
            crate::route::SPTS_PROGRAM_ID,
            loop_file,
            None,
            true,
            limits,
        );
        Self { session }
    }

    fn rewound(&mut self) {
        self.session.rewound();
    }

    fn file_demand(&self) -> FileDemand {
        self.session.file_demand()
    }

    fn next_due(&self) -> Option<std::time::Instant> {
        self.session.next_due()
    }

    fn drained(&self) -> bool {
        self.session.drained()
    }
}

impl broadcast_common::Stage for StreamingFileIngest {
    type In<'a> = FileInput<'a>;
    type Out = SessionEvent;
    type Error = FileReaderError;

    fn feed(
        &mut self,
        input: Self::In<'_>,
        now: broadcast_common::Timestamp,
    ) -> Result<(), Self::Error> {
        self.session.feed(input, now)
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.session.poll()
    }

    fn finish(&mut self) -> Result<(), Self::Error> {
        self.session.finish()
    }

    fn demand(&self) -> broadcast_common::Demand {
        self.session.demand()
    }
}

impl media_plane::ingress::IngestSession for StreamingFileIngest {
    type Request = ();
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
        let samples = vec![Sample::new(vec![0], None, None, None, false).expect("valid timing")];
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

    /// The cap is tied to the opened handle, not a path lookup: a file that
    /// grows past the cap after the open is refused rather than read whole.
    #[tokio::test]
    async fn a_file_grown_past_the_cap_after_the_open_is_refused() {
        let dir = std::env::temp_dir().join(format!("multimux-capped-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("grow.bin");
        std::fs::write(&path, vec![0u8; 8]).unwrap();
        let file = tokio::fs::File::open(&path).await.unwrap();
        std::fs::write(&path, vec![0u8; 9]).unwrap();
        let err = read_capped(&path, file, 8).await;
        assert!(matches!(
            err,
            Err(FileReaderError::FileTooLarge {
                size: 9,
                max: 8,
                ..
            })
        ));
        let file = tokio::fs::File::open(&path).await.unwrap();
        assert_eq!(read_capped(&path, file, 9).await.unwrap().len(), 9);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    /// The cap is inclusive: a file exactly `max_file_bytes` long is a valid
    /// playout asset, and one byte over is refused with its real size.
    #[tokio::test]
    async fn a_file_exactly_at_the_size_cap_is_accepted_and_one_byte_over_is_refused() {
        // A capped whole-file container: TS, FLV and MP4 stream from disk
        // instead and are deliberately not subject to the cap.
        let path = test_fixtures::path("ps/h264-ac3.ps");
        let len = std::fs::metadata(&path).expect("fixture metadata").len() as usize;
        assert!(len > 0);

        let trunk = media_plane::trunk::Trunk::new(crate::source::driver_trunk_config(8));
        let accepted = FileReader::new(
            FileReaderConfig::new(path.clone(), false, trunk)
                .with_pace(false)
                .with_max_retries(0)
                .with_max_file_bytes(len),
        )
        .run()
        .await;
        assert!(
            accepted.is_ok(),
            "a file exactly at the cap must load, got {accepted:?}"
        );

        let trunk = media_plane::trunk::Trunk::new(crate::source::driver_trunk_config(8));
        let refused = FileReader::new(
            FileReaderConfig::new(path, false, trunk)
                .with_pace(false)
                .with_max_retries(0)
                .with_max_file_bytes(len - 1),
        )
        .run()
        .await;
        match refused {
            Err(FileReaderError::FileTooLarge { size, max, .. }) => {
                assert_eq!(size as usize, len, "reports the file's real size");
                assert_eq!(max, len - 1, "reports the configured cap");
            }
            other => panic!("one byte over the cap must be FileTooLarge, got {other:?}"),
        }
    }

    /// Pacing sleeps by the sample's own delta from the file's start, so a
    /// 0 s, 1 s and 3 s sample play at 0 s, 1 s and 3 s from the wall-clock
    /// start (the third sample's own two-second gap), not all at once and not
    /// at a duration derived from the absolute presentation time.
    #[tokio::test(start_paused = true)]
    async fn pacing_sleeps_by_the_sample_delta_from_the_file_start() {
        let item = |due_seconds: f64| PublishItem {
            track_ref: 0,
            sample: Sample::new(vec![0xAB], None, None, None, false).expect("valid sample"),
            due_seconds,
            decode_seconds: due_seconds,
        };
        let order = vec![item(10.0), item(11.0), item(13.0)];
        let specs = vec![media_plane::ts_session::test_support::track_spec(
            transmux::TrackId::new(1),
        )];
        let trunk = media_plane::trunk::Trunk::new(crate::source::driver_trunk_config(8));
        let writer = trunk.writer().expect("a fresh trunk has a writer");
        let begin = tokio::time::Instant::now();
        let start: std::time::Instant = begin.into();
        publish_one_pass(&writer, &specs, &order, &[0], Some(start), 10.0)
            .await
            .expect("pass");
        let elapsed = tokio::time::Instant::now() - begin;
        assert!(
            elapsed >= std::time::Duration::from_millis(3_900)
                && elapsed <= std::time::Duration::from_millis(4_100),
            "the 0/1/3 s samples must sleep 0 + 1 + 3 s, got {elapsed:?}"
        );
    }

    /// The route drive loop sleeps until `next_due` rather than polling: a
    /// looping paced file never idles the runtime, so a bounded timeout still
    /// fires only if every future due instant is slept on.
    #[tokio::test(start_paused = true)]
    async fn the_file_route_sleeps_until_the_next_sample_is_due() {
        let path = test_fixtures::path("ts/h264-aac.ts");
        let route = std::sync::Arc::new(crate::route::RouteHandle::new(
            crate::route::RouteConfig::default()
                .with_target_duration(std::time::Duration::from_secs(1))
                .with_part_target(std::time::Duration::from_millis(250))
                .with_window_segments(8),
        ));
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            run_file_source(
                path.to_str().expect("utf8 fixture path"),
                true,
                8,
                media_plane::ts_session::test_support::handshake(),
                &route,
            ),
        )
        .await;
        assert!(
            outcome.is_err(),
            "a looping paced route sleeps between samples instead of spinning"
        );
    }
}
