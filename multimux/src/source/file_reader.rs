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
//! * **TS, FLV, MP4, PS and Matroska stream.**
//!   [`media_plane::file_stream::StreamingFileSession`] drives an incremental
//!   demuxer; the reader answers its forward chunks (TS/FLV/PS) or absolute
//!   byte ranges (MP4/Matroska) until it asks for end of input, then rewinds to
//!   offset 0 for the next pass. Live memory is a function of the session's
//!   [`FileStreamLimits`] and never of the file's length.
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
    DemuxerKind, FILE_TIMED_RING, FileReaderError, PROBE_PREFIX_MAX, ProbeStep, probe_prefix,
};
use media_plane::file_stream::{
    FileDemand, FileInput, FileStreamLimits, StreamContainer, StreamingFileSession,
};
use media_plane::ingress::SessionEvent;
use media_plane::trunk::{Trunk, TrunkWriter};
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
}

impl FileReaderConfig {
    /// Construct with sensible runtime defaults: pacing on, 3 read retries at
    /// 500 ms and no loop cap. Use the `with_*` builders to tune the remaining
    /// knobs.
    pub fn new(path: PathBuf, loop_file: bool, trunk: Arc<Trunk>) -> Self {
        FileReaderConfig {
            path,
            loop_file,
            trunk,
            pace: true,
            max_retries: DEFAULT_MAX_RETRIES,
            retry_interval: DEFAULT_RETRY_INTERVAL,
            max_loops: None,
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

    /// One open/probe/play attempt: every supported container streams from
    /// disk.
    async fn run_once(&self) -> Result<(), FileReaderError> {
        let probed = open_probed(&self.config.path).await?;
        let demuxer = probed.demuxer;
        match stream_container(demuxer) {
            Some(container) => self.publish_streaming(probed, container).await,
            None => Err(FileReaderError::UnsupportedFormat {
                format: demuxer.name(),
            }),
        }
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
            matroska: matroska_config(probed.len),
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
                    if reads_by_range(container) {
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
}

/// One opened and identified playout file, positioned at offset 0.
struct OpenFile {
    file: tokio::fs::File,
    len: u64,
    demuxer: DemuxerKind,
}

/// Whether the session reaches this container's bytes by absolute range.
fn reads_by_range(container: StreamContainer) -> bool {
    matches!(container, StreamContainer::Mp4 | StreamContainer::Matroska)
}

/// The streaming container a probed demuxer selects.
fn stream_container(demuxer: DemuxerKind) -> Option<StreamContainer> {
    match demuxer {
        DemuxerKind::StreamingTs => Some(StreamContainer::MpegTs),
        DemuxerKind::StreamingFlv => Some(StreamContainer::Flv),
        DemuxerKind::Fmp4 | DemuxerKind::Progressive | DemuxerKind::Mp4 => {
            Some(StreamContainer::Mp4)
        }
        DemuxerKind::Ps => Some(StreamContainer::MpegPs),
        DemuxerKind::Webm => Some(StreamContainer::Matroska),
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

/// The Matroska stage policy for a file of `total_len` bytes.
fn matroska_config(total_len: u64) -> transmux::matroska_demux::MatroskaDemuxConfig {
    transmux::matroska_demux::MatroskaDemuxConfig::default().with_total_len(Some(total_len))
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
/// through a [`StreamingFileSession`]; WebM keeps the whole-file
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
    let probed = match open_probed(std::path::Path::new(path)).await {
        Ok(probed) => probed,
        Err(e) => return e.into(),
    };

    let Some(container) = stream_container(probed.demuxer) else {
        return FileReaderError::UnsupportedFormat {
            format: probed.demuxer.name(),
        }
        .into();
    };
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
            matroska: matroska_config(total_len),
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

    /// A minimal Matroska file (EBML header + Segment + Info + Tracks + one
    /// Cluster) with one VP8 track and one frame per `(relative ms, byte)`
    /// entry. Element ids and sizes follow RFC 8794 §4 / RFC 9559 §27.
    fn mkv_bytes(frames: &[(i16, u8)]) -> Vec<u8> {
        const EBML_HEADER: [u8; 4] = [0x1A, 0x45, 0xDF, 0xA3];
        const SEGMENT: [u8; 4] = [0x18, 0x53, 0x80, 0x67];
        const INFO: [u8; 4] = [0x15, 0x49, 0xA9, 0x66];
        const TIMESTAMP_SCALE: [u8; 3] = [0x2A, 0xD7, 0xB1];
        const TRACKS: [u8; 4] = [0x16, 0x54, 0xAE, 0x6B];
        const TRACK_ENTRY: [u8; 1] = [0xAE];
        const TRACK_NUMBER: [u8; 1] = [0xD7];
        const TRACK_TYPE: [u8; 1] = [0x83];
        const CODEC_ID: [u8; 1] = [0x86];
        const CLUSTER: [u8; 4] = [0x1F, 0x43, 0xB6, 0x75];
        const CLUSTER_TIMESTAMP: [u8; 1] = [0xE7];
        const SIMPLE_BLOCK: [u8; 1] = [0xA3];

        fn size(mut value: usize) -> Vec<u8> {
            let mut width = 1usize;
            while value >= (1usize << (7 * width)) - 1 {
                width += 1;
            }
            let mut out = vec![0u8; width];
            for slot in (0..width).rev() {
                out[slot] = (value & 0xFF) as u8;
                value >>= 8;
            }
            out[0] |= 1u8 << (8 - width);
            out
        }
        fn element(id: &[u8], body: &[u8]) -> Vec<u8> {
            let mut out = id.to_vec();
            out.extend(size(body.len()));
            out.extend_from_slice(body);
            out
        }
        let info = element(&INFO, &element(&TIMESTAMP_SCALE, &[0x0F, 0x42, 0x40]));
        let track_entry = {
            let mut body = element(&TRACK_NUMBER, &[1]);
            body.extend(element(&TRACK_TYPE, &[1]));
            body.extend(element(&CODEC_ID, b"V_VP8"));
            element(&TRACK_ENTRY, &body)
        };
        let tracks = element(&TRACKS, &track_entry);
        let mut cluster_body = element(&CLUSTER_TIMESTAMP, &[0]);
        for (relative, byte) in frames {
            let mut payload = size(1);
            payload.extend_from_slice(&relative.to_be_bytes());
            payload.push(0x80);
            // A complete VP8 key-frame header: key-frame tag, start code,
            // 320x240 dimensions (`byte` fills the last dimension octet).
            payload.extend_from_slice(&[0x10, 0, 0, 0x9D, 0x01, 0x2A, 0x40, 0x01, 0xF0, *byte]);
            cluster_body.extend(element(&SIMPLE_BLOCK, &payload));
        }
        let cluster = element(&CLUSTER, &cluster_body);
        let mut segment_body = info;
        segment_body.extend(tracks);
        segment_body.extend(cluster);
        let mut out = element(&EBML_HEADER, &[]);
        out.extend(element(&SEGMENT, &segment_body));
        out
    }

    /// A stream at a file's samples: feed its byte demands until `stop` says
    /// the released-sample count is enough.
    async fn drive_matroska(
        session: &mut StreamingFileSession,
        bytes: &[u8],
        mut stop: impl FnMut(usize) -> bool,
    ) -> Result<usize, FileReaderError> {
        let mut offset = 0usize;
        let mut released = 0usize;
        let mut eof = false;
        for _ in 0..(bytes.len() * 4 + 64) {
            match session.file_demand() {
                FileDemand::BytesAt { offset: at, len } => {
                    let start = usize::try_from(at).unwrap_or(usize::MAX).min(bytes.len());
                    let end = start.saturating_add(len).min(bytes.len());
                    session.feed(
                        FileInput::At {
                            offset: at,
                            bytes: bytes[start..end].to_vec().into(),
                        },
                        broadcast_common::Timestamp::ZERO,
                    )?;
                }
                FileDemand::Bytes if offset < bytes.len() => {
                    let end = (offset + (64 << 10)).min(bytes.len());
                    session.feed(
                        FileInput::Bytes(&bytes[offset..end]),
                        broadcast_common::Timestamp::ZERO,
                    )?;
                    offset = end;
                }
                FileDemand::Eof if !eof => {
                    session.feed(FileInput::Eof, broadcast_common::Timestamp::ZERO)?;
                    eof = true;
                }
                FileDemand::Done => return Ok(released),
                _ => {}
            }
            while let Some(event) = session.poll() {
                if matches!(event, SessionEvent::Sample { .. }) {
                    released += 1;
                }
            }
            if stop(released) || (eof && released > 0) {
                return Ok(released);
            }
        }
        Ok(released)
    }

    /// A file with no playable sample is refused, on the streaming path too:
    /// a Matroska track with no Block has no timed sample for the session to
    /// pace, loop or write.
    #[tokio::test]
    async fn a_streaming_matroska_file_with_no_playable_sample_is_rejected() {
        let bytes = mkv_bytes(&[]);
        let total = u64::try_from(bytes.len()).expect("length fits u64");
        let mut session = StreamingFileSession::new(
            StreamContainer::Matroska,
            STANDALONE_PROGRAM,
            true,
            None,
            false,
            FileStreamLimits {
                matroska: transmux::matroska_demux::MatroskaDemuxConfig::default()
                    .with_total_len(Some(total)),
                ..FileStreamLimits::default()
            },
        );
        let outcome = drive_matroska(&mut session, &bytes, |released| released > 0).await;
        let error = match outcome {
            Err(error) => error,
            Ok(_) => session.finish().expect_err("no timed sample can be played"),
        };
        assert!(
            matches!(error, FileReaderError::NoPlayableSamples),
            "a file with no playable sample must be refused, got {error:?}"
        );
    }

    /// Pacing sleeps by the sample's own delta from the file start: the second
    /// sample (100 ms after the first) is due ~100 ms after the first was
    /// released, and the third (200 ms after the second) ~200 ms later.
    #[tokio::test]
    async fn a_streaming_matroska_file_paces_by_the_sample_delta() {
        const SECOND_MS: u64 = 100;
        const THIRD_MS: u64 = 200;
        let bytes = mkv_bytes(&[(0, 1), (SECOND_MS as i16, 2), (300, 3)]);
        let total = u64::try_from(bytes.len()).expect("length fits u64");
        let mut session = StreamingFileSession::new(
            StreamContainer::Matroska,
            STANDALONE_PROGRAM,
            false,
            None,
            true,
            FileStreamLimits {
                matroska: transmux::matroska_demux::MatroskaDemuxConfig::default()
                    .with_total_len(Some(total)),
                ..FileStreamLimits::default()
            },
        );
        let released = drive_matroska(&mut session, &bytes, |_| false)
            .await
            .expect("first sample");
        assert_eq!(released, 1, "the first sample is due immediately");
        let due = session.next_due().expect("a paced session has a deadline");
        let wait = due.saturating_duration_since(std::time::Instant::now());
        assert!(
            wait >= std::time::Duration::from_millis(50)
                && wait <= std::time::Duration::from_millis(250),
            "the next sample must be paced ~{SECOND_MS} ms after the first, got {wait:?}"
        );
        tokio::time::sleep(wait).await;
        let released = drive_matroska(&mut session, &bytes, |_| false)
            .await
            .expect("second sample");
        assert_eq!(released, 1, "the second sample is due after its delta");
        let due = session.next_due().expect("a paced session has a deadline");
        let wait = due.saturating_duration_since(std::time::Instant::now());
        assert!(
            wait >= std::time::Duration::from_millis(120)
                && wait <= std::time::Duration::from_millis(400),
            "the third sample must follow the second by ~{THIRD_MS} ms, got {wait:?}"
        );
    }

    /// The ring depth does not depend on the file or the window.
    #[test]
    fn the_file_ring_is_independent_of_the_file_length() {
        assert_eq!(file_trunk_config(8).timed_capacity.get(), FILE_TIMED_RING);
        assert_eq!(file_trunk_config(500).timed_capacity.get(), FILE_TIMED_RING);
        assert_eq!(file_trunk_config(500).segment_capacity.get(), 500);
    }

    /// Pacing sleeps by the sample's own delta from the file's start, so a
    /// 0 s, 1 s and 3 s sample play at 0 s, 1 s and 3 s from the wall-clock
    /// start (the third sample's own two-second gap), not all at once and not
    /// at a duration derived from the absolute presentation time.
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
