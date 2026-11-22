//! The blocking `std::fs` adapter: [`FsRecorder`] executes the writer's plans
//! synchronously, and [`FsArchive`] reads archived periods with its own
//! bounded, per-archive parsed-playlist cache.
//!
//! Everything here blocks. A host on an async runtime must run it on a
//! blocking pool, as `multimux` does; the crate core never does I/O.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use broadcast_common::Timestamp;
use broadcast_common::range::ByteSpan;
use bytes::Bytes;
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{SegmentCursor, SegmentCursorItem, SegmentEntry, Trunk};

use crate::layout::{self, ArchiveFile, SeedFile};
use crate::read::{
    ArchiveScan, ArchivedSegment, PeriodPlaylist, PlaylistError, ReadError, SeqConflict,
    checked_range, parse_period_playlist,
};
use crate::recover::{RecoveredBox, scan_fmp4_data};
use crate::writer::{
    ArchiveWriter, InitGeneration, Plan, PlanOutcome, Step, StoreFailure, StoreOp,
};
use crate::{ArchiveConfig, ArchiveContainer, ArchiveError, ArchiveName, PeriodNum, WriterEvent};

/// Most parsed period playlists one [`FsArchive`] caches.
pub const INDEX_CACHE_MAX_ENTRIES: usize = 4096;

/// Most segments every cached playlist in one [`FsArchive`] holds in total.
pub const INDEX_CACHE_MAX_SEGMENTS: usize = 65_536;

/// An `fs`-adapter failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FsError {
    /// The writer rejected the input.
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    /// A storage operation failed.
    #[error("{op} {}: {source}", path.display())]
    Io {
        /// The operation that failed.
        op: crate::writer::StoreOpKind,
        /// The file or directory it was working on.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// Another recorder holds the archive directory's single-writer lock.
    #[error("archive directory {} is locked by another recorder", path.display())]
    Locked {
        /// The lock file that is held.
        path: PathBuf,
    },
}

fn path_of(dir: &Path, op: &StoreOp, container: ArchiveContainer) -> PathBuf {
    match op {
        StoreOp::CreateDir => dir.to_path_buf(),
        StoreOp::CreateData { period }
        | StoreOp::AppendData { period, .. }
        | StoreOp::SyncData { period } => dir.join(layout::media_file_name(*period, container)),
        StoreOp::CreatePlaylist { period, .. } | StoreOp::AppendPlaylist { period, .. } => {
            dir.join(layout::playlist_file_name(*period))
        }
        StoreOp::Remove { file } => dir.join(layout::file_name(*file, container)),
    }
}

fn io_error(path: PathBuf, op: crate::writer::StoreOpKind, source: std::io::Error) -> FsError {
    FsError::Io { op, path, source }
}

/// The blocking archive recorder: one [`ArchiveWriter`], one pinning
/// [`SegmentCursor`] on the Trunk, and the archive's directory.
///
/// [`Self::poll`] is the only driver. `FsRecorder` does not implement
/// `media_plane::egress::SegmentEgress`: that trait hands over one cursor
/// item with no monotonic time, no wall time and no fMP4 init generations,
/// so a period could never roll on `period_duration` (and periods-only
/// retention would never evict) and an fMP4 archive would drop every segment
/// as arriving before its init.
///
/// ```compile_fail
/// fn egress<E: media_plane::egress::SegmentEgress>() {}
/// egress::<dvr_runtime::fs::FsRecorder>();
/// ```
pub struct FsRecorder {
    writer: ArchiveWriter,
    cursor: SegmentCursor,
    trunk: Arc<Trunk>,
    dir: PathBuf,
    _lock: File,
    tracker: Option<dvb_si::epg::PresentEventTracker>,
    container: ArchiveContainer,
    retry: Option<(SegmentCursorItem, u8)>,
    programme_retry: Option<dvb_si::epg::PresentEventChange>,
}

/// How many times one cursor item is retried after a store failure before it
/// becomes a counted gap.
const MAX_RETRIES: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Execution {
    /// The input is durable (or produced no plan); never retry it.
    Consumed,
    /// The plan failed before its record landed; the input must be retried.
    Retry,
}

impl FsRecorder {
    /// Create a recorder for the archive `name` under `root`, adopt the
    /// periods already on disk, and pin a segment cursor on `trunk` with the
    /// configured overrun policy.
    ///
    /// The directory is created if absent and guarded by an exclusive advisory
    /// lock on [`layout::LOCK_FILE_NAME`] (`File::try_lock`), held until the
    /// recorder drops. The kernel releases it when the process exits or
    /// crashes, so there is no stale-lock state to reason about. The lock
    /// file is never an archive file: it is not listed, scanned or evicted.
    ///
    /// `config.seq_offset` must put this run's archive sequence numbers
    /// (Trunk sequence plus offset) above [`FsArchive::seq_floor`] of the
    /// archive being resumed. A run that restarts at lower numbers collides
    /// with archived sequences: [`FsArchive::scan`] keeps the earlier period's
    /// record and reports each later claim in `ArchiveScan::conflicts`, so the
    /// colliding segments are hidden from readers (never silently lost on
    /// disk).
    ///
    /// Stale `.pN.m3u8.tmp` files from a crash between the temp write and the
    /// rename are removed and reported as
    /// [`WriterEvent::StaleTempPlaylistRemoved`].
    ///
    /// # Errors
    ///
    /// [`FsError::Archive`] for an invalid configuration or exhausted period
    /// numbers; [`FsError::Locked`] when another recorder holds the
    /// directory; [`FsError::Io`] when the directory cannot be created,
    /// locked or listed (a permission error is surfaced, never read as an
    /// empty archive).
    pub fn new(
        root: &Path,
        name: ArchiveName,
        config: ArchiveConfig,
        trunk: &Arc<Trunk>,
    ) -> Result<Self, FsError> {
        let overrun = config.overrun;
        let dir = layout::route_dir(root, &name);
        let lock = Self::lock(&dir)?;
        Self::open(dir, lock, config, trunk, || trunk.pin_segments(overrun))
    }

    /// As [`Self::new`], but recording from `cursor`, a pinning cursor the
    /// host took on `trunk` earlier (with the configured overrun policy), so
    /// no segment closed since then is missed. The cursor is taken out of
    /// `cursor` only when the recorder is created; on an error it stays, so
    /// the host can retry, for example once the recorder holding the
    /// directory's lock has been dropped ([`FsError::Locked`]). With `None`
    /// a fresh cursor is pinned, as [`Self::new`] does.
    ///
    /// # Errors
    ///
    /// As [`Self::new`].
    pub fn with_cursor(
        root: &Path,
        name: ArchiveName,
        config: ArchiveConfig,
        trunk: &Arc<Trunk>,
        cursor: &mut Option<SegmentCursor>,
    ) -> Result<Self, FsError> {
        let overrun = config.overrun;
        let dir = layout::route_dir(root, &name);
        let lock = Self::lock(&dir)?;
        Self::open(dir, lock, config, trunk, || {
            cursor.take().unwrap_or_else(|| trunk.pin_segments(overrun))
        })
    }

    /// The recorder of a new run on this recorder's archive (a host replacing
    /// its source, for example after a reconnect), recording from `cursor`
    /// on `trunk` as [`Self::with_cursor`] does. It takes over this
    /// recorder's directory lock, which is never released in between: a
    /// release followed by a fresh [`Self::with_cursor`] can be refused
    /// while a process the host spawns at that moment briefly shares the
    /// lock's file description. The archive is re-seeded from disk, so the
    /// new run opens a new period, as after [`Self::new`].
    ///
    /// # Errors
    ///
    /// As [`Self::new`], except [`FsError::Locked`]; on an error this
    /// recorder and its lock are dropped.
    pub fn succeed(
        self,
        config: ArchiveConfig,
        trunk: &Arc<Trunk>,
        cursor: &mut Option<SegmentCursor>,
    ) -> Result<Self, FsError> {
        let overrun = config.overrun;
        let Self { dir, _lock, .. } = self;
        Self::open(dir, _lock, config, trunk, || {
            cursor.take().unwrap_or_else(|| trunk.pin_segments(overrun))
        })
    }

    fn lock(dir: &Path) -> Result<File, FsError> {
        fs::create_dir_all(dir)
            .map_err(|e| io_error(dir.to_path_buf(), crate::writer::StoreOpKind::CreateDir, e))?;
        let lock_path = dir.join(layout::LOCK_FILE_NAME);
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|e| io_error(lock_path.clone(), crate::writer::StoreOpKind::List, e))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => {
                return Err(FsError::Locked { path: lock_path });
            }
            Err(fs::TryLockError::Error(e)) => {
                return Err(io_error(lock_path, crate::writer::StoreOpKind::List, e));
            }
        }
        Ok(lock)
    }

    fn open(
        dir: PathBuf,
        lock: File,
        config: ArchiveConfig,
        trunk: &Arc<Trunk>,
        cursor: impl FnOnce() -> SegmentCursor,
    ) -> Result<Self, FsError> {
        let container = config.container;
        let mut existing = Vec::new();
        let mut stale_temps = Vec::new();
        match fs::read_dir(&dir) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry
                        .map_err(|e| io_error(dir.clone(), crate::writer::StoreOpKind::List, e))?;
                    let file_name = entry.file_name();
                    let Some(file_name) = file_name.to_str() else {
                        continue;
                    };
                    if let Some(period) = layout::classify_playlist_tmp(file_name) {
                        let path = entry.path();
                        fs::remove_file(&path)
                            .map_err(|e| io_error(path, crate::writer::StoreOpKind::Remove, e))?;
                        stale_temps.push(period);
                    } else if let Some(file) = layout::classify(file_name, container) {
                        let bytes = entry
                            .metadata()
                            .map_err(|e| {
                                io_error(dir.clone(), crate::writer::StoreOpKind::List, e)
                            })?
                            .len();
                        existing.push(SeedFile::new(file, bytes));
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(io_error(dir.clone(), crate::writer::StoreOpKind::List, e));
            }
        }
        let mut writer = ArchiveWriter::new(config.clone())?;
        writer.seed(&existing)?;
        writer.note_stale_temp_playlists(&stale_temps)?;
        let cursor = cursor();
        let tracker = config
            .programme_service
            .map(dvb_si::epg::PresentEventTracker::new);
        Ok(Self {
            writer,
            cursor,
            trunk: Arc::clone(trunk),
            dir,
            _lock: lock,
            tracker,
            container,
            retry: None,
            programme_retry: None,
        })
    }

    /// A cheap "is there anything to do" check; see
    /// [`ArchiveWriter::needs_poll`].
    #[must_use]
    pub fn needs_poll(
        &self,
        latest_closed: Option<SegmentSeq>,
        latest_init: Option<&[u8]>,
    ) -> bool {
        self.writer.needs_poll(latest_closed, latest_init)
    }

    /// Drain the cursor and execute every plan the writer produces.
    ///
    /// `wall` supplies a segment's wall-clock time (an
    /// `EXT-X-PROGRAM-DATE-TIME`), and `inits` the fMP4 init generations
    /// closed since the last poll. Blocking.
    ///
    /// A store failure that leaves its record uncommitted keeps that item and
    /// retries it on the next poll (`MAX_RETRIES` times); after the budget it
    /// is dropped as a counted gap and a [`WriterEvent::Gap`] is emitted. An
    /// I/O error is still returned once so the caller can log it; the item
    /// stays queued. A failure after the record landed consumes the input and
    /// is not retried.
    ///
    /// # Errors
    ///
    /// [`FsError::Archive`] when the writer rejects an input; [`FsError::Io`]
    /// for a store failure (the input is retained for the next poll).
    pub fn poll(
        &mut self,
        now: Timestamp,
        wall: impl Fn(&SegmentEntry) -> Option<jiff::Timestamp>,
        inits: &[InitGeneration],
    ) -> Result<(), FsError> {
        self.writer
            .set_latest_closed(self.trunk.last_closed_segment());
        self.writer.on_inits(inits)?;
        let tick = self.writer.on_tick(now)?;
        let _ = self.handle_step(tick)?;
        if let Some((item, attempts)) = self.retry.take() {
            match self.process(&item, now, &wall) {
                Ok(Execution::Consumed) => {}
                Ok(Execution::Retry) => {
                    self.schedule_retry(item, attempts.saturating_add(1), now);
                    return Ok(());
                }
                Err(error) => {
                    self.schedule_retry(item, attempts.saturating_add(1), now);
                    return Err(error);
                }
            }
        }
        loop {
            let Some(item) = self.cursor.poll() else {
                break;
            };
            match self.process(&item, now, &wall) {
                Ok(Execution::Consumed) => {}
                Ok(Execution::Retry) => {
                    self.schedule_retry(item, 1, now);
                    break;
                }
                Err(error) => {
                    self.schedule_retry(item, 1, now);
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// Feed raw MPEG-2 TS bytes for EIT present/following tracking. A no-op
    /// unless the configuration names a programme service. Blocking (a
    /// transition rolls the period).
    ///
    /// A transition whose close plan fails is kept and retried by the next
    /// call rather than discarded.
    ///
    /// # Errors
    ///
    /// As [`Self::poll`].
    pub fn feed_si(&mut self, ts: &[u8]) -> Result<(), FsError> {
        let Some(mut tracker) = self.tracker.take() else {
            return Ok(());
        };
        tracker.feed_ts(ts);
        let changes: Vec<dvb_si::epg::PresentEventChange> =
            std::iter::from_fn(|| tracker.poll_change()).collect();
        self.tracker = Some(tracker);
        self.apply_changes(changes)
    }

    /// Apply a programme change detected outside this recorder, for example
    /// by a host that tracked the EIT before the recorder could be created.
    /// It is applied, kept and retried exactly like a change [`Self::feed_si`]
    /// detects; the same event again changes nothing.
    ///
    /// # Errors
    ///
    /// As [`Self::poll`].
    pub fn on_programme(&mut self, change: dvb_si::epg::PresentEventChange) -> Result<(), FsError> {
        self.apply_changes(vec![change])
    }

    fn apply_changes(
        &mut self,
        changes: Vec<dvb_si::epg::PresentEventChange>,
    ) -> Result<(), FsError> {
        let mut queue = Vec::new();
        if let Some(change) = self.programme_retry.take() {
            queue.push(change);
        }
        queue.extend(changes);
        for change in queue {
            let retry = change.clone();
            match self.apply_programme(change) {
                Ok(Execution::Consumed) => {}
                Ok(Execution::Retry) => {
                    self.programme_retry = Some(retry);
                    break;
                }
                Err(error) => {
                    self.programme_retry = Some(retry);
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// The next queued writer event, if any.
    pub fn poll_event(&mut self) -> Option<WriterEvent> {
        self.writer.poll_event()
    }

    /// The underlying writer.
    #[must_use]
    pub fn writer(&self) -> &ArchiveWriter {
        &self.writer
    }

    fn process(
        &mut self,
        item: &SegmentCursorItem,
        now: Timestamp,
        wall: &impl Fn(&SegmentEntry) -> Option<jiff::Timestamp>,
    ) -> Result<Execution, FsError> {
        let wall_time = match item {
            SegmentCursorItem::Segment(entry) => wall(entry),
            _ => None,
        };
        let step = self.writer.on_item(now, wall_time, item)?;
        self.handle_step(step)
    }

    fn apply_programme(
        &mut self,
        change: dvb_si::epg::PresentEventChange,
    ) -> Result<Execution, FsError> {
        let step = self.writer.on_programme(change)?;
        self.handle_step(step)
    }

    fn schedule_retry(&mut self, item: SegmentCursorItem, attempts: u8, now: Timestamp) {
        if attempts > MAX_RETRIES {
            let _ = self
                .writer
                .on_item(now, None, &SegmentCursorItem::Gap { skipped: 1 });
        } else {
            self.retry = Some((item, attempts));
        }
    }

    fn handle_step(&mut self, step: Step) -> Result<Execution, FsError> {
        match step {
            Step::Idle | Step::Stopped => Ok(Execution::Consumed),
            Step::Rearm { .. } => {
                self.cursor = self.trunk.pin_segments(self.writer.overrun());
                Ok(Execution::Consumed)
            }
            Step::Execute(plan) => self.execute(plan),
        }
    }

    fn execute(&mut self, plan: Plan) -> Result<Execution, FsError> {
        let mut outcome = PlanOutcome::Done;
        let mut error = None;
        for (at_op, op) in plan.ops.iter().enumerate() {
            if let Err(failed) = store_op(&self.dir, self.container, op) {
                if let Some(source) = failed.source {
                    error = Some(io_error(
                        path_of(&self.dir, op, self.container),
                        op.kind(),
                        source,
                    ));
                }
                outcome = PlanOutcome::Failed {
                    at_op,
                    failure: failed.failure,
                };
                break;
            }
        }
        self.writer.complete(plan.id, outcome)?;
        if self.writer.last_input_consumed() {
            Ok(Execution::Consumed)
        } else {
            match error {
                Some(error) => Err(error),
                None => Ok(Execution::Retry),
            }
        }
    }
}

/// A failed store operation: the typed failure the writer records, plus the
/// OS error it came from (which `StoreFailure` keeps only as a kind).
#[derive(Debug)]
struct StoreOpFailure {
    failure: StoreFailure,
    source: Option<std::io::Error>,
}

impl StoreOpFailure {
    fn io(error: std::io::Error) -> Self {
        Self {
            failure: StoreFailure::Io(error.kind()),
            source: Some(error),
        }
    }

    fn not_durable(error: std::io::Error) -> Self {
        Self {
            failure: StoreFailure::NotDurable(error.kind()),
            source: Some(error),
        }
    }

    fn bare(failure: StoreFailure) -> Self {
        Self {
            failure,
            source: None,
        }
    }
}

fn store_op(dir: &Path, container: ArchiveContainer, op: &StoreOp) -> Result<(), StoreOpFailure> {
    match op {
        StoreOp::CreateDir => fs::create_dir_all(dir).map_err(StoreOpFailure::io),
        StoreOp::CreateData { period } => {
            let path = dir.join(layout::media_file_name(*period, container));
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map(|_| ())
                .map_err(StoreOpFailure::io)
        }
        StoreOp::AppendData {
            period,
            expect_len,
            bytes,
        } => {
            let path = dir.join(layout::media_file_name(*period, container));
            append_at(&path, *expect_len, bytes)
        }
        StoreOp::SyncData { period } => {
            let media = dir.join(layout::media_file_name(*period, container));
            sync_file(&media)?;
            let playlist = dir.join(layout::playlist_file_name(*period));
            match sync_file(&playlist) {
                Ok(()) => Ok(()),
                Err(failed) if failed.failure == StoreFailure::Io(std::io::ErrorKind::NotFound) => {
                    Ok(())
                }
                Err(failed) => Err(failed),
            }
        }
        StoreOp::CreatePlaylist { period, text } => {
            let tmp = dir.join(layout::playlist_tmp_name(*period));
            let path = dir.join(layout::playlist_file_name(*period));
            let write = || -> std::io::Result<()> {
                if path.exists() {
                    return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
                }
                let mut file = File::create(&tmp)?;
                file.write_all(text.as_bytes())?;
                file.sync_all()?;
                fs::rename(&tmp, &path)
            };
            write().map_err(StoreOpFailure::io)?;
            File::open(dir)
                .and_then(|handle| handle.sync_all())
                .map_err(StoreOpFailure::not_durable)
        }
        StoreOp::AppendPlaylist {
            period,
            expect_len,
            text,
        } => {
            let path = dir.join(layout::playlist_file_name(*period));
            append_at(&path, *expect_len, text.as_bytes())
        }
        StoreOp::Remove { file } => {
            let path = dir.join(layout::file_name(*file, container));
            match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(StoreOpFailure::io(e)),
            }
        }
    }
}

fn sync_file(path: &Path) -> Result<(), StoreOpFailure> {
    OpenOptions::new()
        .append(true)
        .open(path)
        .and_then(|file| file.sync_data())
        .map_err(StoreOpFailure::io)
}

fn append_at(path: &Path, expect_len: u64, bytes: &[u8]) -> Result<(), StoreOpFailure> {
    let mut file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(StoreOpFailure::io)?;
    let actual = file.metadata().map_err(StoreOpFailure::io)?.len();
    if actual != expect_len {
        let settled = if actual > expect_len {
            file.set_len(expect_len).map_err(StoreOpFailure::io)?;
            expect_len
        } else {
            actual
        };
        return Err(StoreOpFailure::bare(StoreFailure::LengthMismatch {
            actual: settled,
        }));
    }
    let result = file
        .seek(SeekFrom::End(0))
        .and_then(|_| file.write_all(bytes));
    if let Err(e) = result {
        let _ = file.set_len(expect_len);
        return Err(StoreOpFailure::io(e));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    mtime: Option<SystemTime>,
    len: u64,
    inode: u64,
}

fn inode_of(metadata: &fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        0
    }
}

struct CacheEntry {
    stamp: Stamp,
    playlist: Arc<PeriodPlaylist>,
    last_used: u64,
}

/// The blocking read half: every readable segment of one archive, plus its
/// own bounded parsed-playlist cache.
pub struct FsArchive {
    dir: PathBuf,
    container: ArchiveContainer,
    cache: BTreeMap<PeriodNum, CacheEntry>,
    tick: u64,
}

impl FsArchive {
    /// Open the archive `name` under `root`. No I/O.
    #[must_use]
    pub fn open(root: &Path, name: &ArchiveName, container: ArchiveContainer) -> Self {
        Self {
            dir: layout::route_dir(root, name),
            container,
            cache: BTreeMap::new(),
            tick: 0,
        }
    }

    /// Every readable segment across every period, plus the periods that
    /// could not be read. A missing archive directory is an empty archive.
    ///
    /// # Errors
    ///
    /// [`ReadError::Io`] when the archive directory exists but cannot be
    /// listed: an unlistable archive is never reported as an empty one.
    pub fn scan(&mut self) -> Result<ArchiveScan, ReadError> {
        let mut scan = ArchiveScan::default();
        let mut owner: BTreeMap<SegmentSeq, PeriodNum> = BTreeMap::new();
        for period in self.periods()? {
            match self.playlist(period) {
                Ok(playlist) => {
                    for segment in &playlist.segments {
                        match owner.get(&segment.seq) {
                            Some(kept) => {
                                scan.conflicts
                                    .push(SeqConflict::new(segment.seq, *kept, period))
                            }
                            None => {
                                owner.insert(segment.seq, period);
                                scan.segments.push(*segment);
                            }
                        }
                    }
                }
                Err(error) => scan.unreadable.push((period, error)),
            }
        }
        scan.segments.sort_by_key(|segment| segment.seq);
        self.prune();
        Ok(scan)
    }

    /// The archived segment numbered `seq`, if any.
    ///
    /// # Errors
    ///
    /// As [`Self::scan`].
    pub fn find(&mut self, seq: SegmentSeq) -> Result<Option<ArchivedSegment>, ReadError> {
        for period in self.periods()? {
            if let Ok(playlist) = self.playlist(period)
                && let Some(segment) = playlist.segments.iter().find(|s| s.seq == seq)
            {
                return Ok(Some(*segment));
            }
        }
        Ok(None)
    }

    /// Read one archived segment's exact bytes.
    ///
    /// # Errors
    ///
    /// [`ReadError::Gone`] when the period was evicted, and
    /// [`ReadError::RangeBeyondFile`] when the recorded range is not backed
    /// by the file.
    pub fn read(&self, segment: &ArchivedSegment) -> Result<Bytes, ReadError> {
        self.read_span(segment.period, segment.span)
    }

    /// Read a period's init bytes.
    ///
    /// # Errors
    ///
    /// As [`Self::read`].
    pub fn read_init(&self, period: PeriodNum, span: ByteSpan) -> Result<Bytes, ReadError> {
        self.read_span(period, span)
    }

    /// The highest archived sequence number, or `None` for an archive that
    /// holds no segment, for a restarted route to number above.
    ///
    /// # Errors
    ///
    /// As [`Self::scan`], and [`ReadError::Playlist`] with
    /// [`PlaylistError::Io`] when a period playlist exists but cannot be
    /// read: its sequence numbers are unknown, so any floor could be too low.
    pub fn seq_floor(&mut self) -> Result<Option<SegmentSeq>, ReadError> {
        let scan = self.scan()?;
        if let Some((_, error)) = scan
            .unreadable
            .into_iter()
            .find(|(_, error)| matches!(error, PlaylistError::Io { .. }))
        {
            return Err(error.into());
        }
        Ok(scan.segments.iter().map(|s| s.seq).max())
    }

    /// Recover a period's init and fragment spans from its media file.
    ///
    /// # Errors
    ///
    /// [`ReadError::Gone`] when the period was evicted,
    /// [`ReadError::Recover`] when the file is not a recoverable fMP4
    /// period.
    pub fn recover_spans(&self, period: PeriodNum) -> Result<Vec<RecoveredBox>, ReadError> {
        let path = self
            .dir
            .join(layout::media_file_name(period, self.container));
        let data = fs::read(path).map_err(|e| io_read_error(period, e))?;
        Ok(scan_fmp4_data(&data)?)
    }

    /// How many period playlists the cache holds.
    #[must_use]
    pub fn cached_periods(&self) -> usize {
        self.cache.len()
    }

    fn periods(&self) -> Result<Vec<PeriodNum>, ReadError> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(ReadError::Io { kind: e.kind() }),
        };
        let mut periods = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| ReadError::Io { kind: e.kind() })?;
            let name = entry.file_name();
            if let Some(ArchiveFile::Playlist(period)) = name
                .to_str()
                .and_then(|name| layout::classify(name, self.container))
            {
                periods.push(period);
            }
        }
        periods.sort_unstable();
        Ok(periods)
    }

    fn playlist(&mut self, period: PeriodNum) -> Result<Arc<PeriodPlaylist>, PlaylistError> {
        let path = self.dir.join(layout::playlist_file_name(period));
        let metadata = fs::metadata(&path).map_err(io_playlist_error)?;
        let stamp = Stamp {
            mtime: metadata.modified().ok(),
            len: metadata.len(),
            inode: inode_of(&metadata),
        };
        if let Some(entry) = self.cache.get(&period)
            && entry.stamp == stamp
        {
            let playlist = Arc::clone(&entry.playlist);
            let tick = self.tick;
            if let Some(entry) = self.cache.get_mut(&period) {
                entry.last_used = tick;
            }
            self.tick = self.tick.wrapping_add(1);
            return Ok(playlist);
        }
        let data = read_bounded_playlist(&path).map_err(io_playlist_error)?;
        let parsed = Arc::new(parse_period_playlist(period, self.container, &data)?);
        let tick = self.tick;
        self.tick = self.tick.wrapping_add(1);
        self.cache.insert(
            period,
            CacheEntry {
                stamp,
                playlist: Arc::clone(&parsed),
                last_used: tick,
            },
        );
        self.enforce_bounds();
        Ok(parsed)
    }

    fn enforce_bounds(&mut self) {
        let mut segments: usize = self.cache.values().map(|e| e.playlist.segments.len()).sum();
        while self.cache.len() > INDEX_CACHE_MAX_ENTRIES || segments > INDEX_CACHE_MAX_SEGMENTS {
            let Some(oldest) = self
                .cache
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(period, _)| *period)
            else {
                break;
            };
            if let Some(removed) = self.cache.remove(&oldest) {
                segments = segments.saturating_sub(removed.playlist.segments.len());
            }
        }
    }

    fn prune(&mut self) {
        let dir = self.dir.clone();
        self.cache
            .retain(|period, _| dir.join(layout::playlist_file_name(*period)).exists());
    }

    fn read_span(&self, period: PeriodNum, span: ByteSpan) -> Result<Bytes, ReadError> {
        let path = self
            .dir
            .join(layout::media_file_name(period, self.container));
        let mut file = File::open(&path).map_err(|e| io_read_error(period, e))?;
        let file_len = file
            .metadata()
            .map_err(|e| ReadError::Io { kind: e.kind() })?
            .len();
        let span = checked_range(span, file_len)?;
        file.seek(SeekFrom::Start(span.offset))
            .map_err(|e| ReadError::Io { kind: e.kind() })?;
        let len = usize::try_from(span.len.get())
            .map_err(|_| ReadError::RangeBeyondFile { span, file_len })?;
        let mut buf = vec![0u8; len];
        file.read_exact(&mut buf)
            .map_err(|e| ReadError::Io { kind: e.kind() })?;
        Ok(Bytes::from(buf))
    }
}

fn read_bounded_playlist(path: &Path) -> std::io::Result<Vec<u8>> {
    let limit = u64::try_from(broadcast_hls::limits::MAX_PLAYLIST_BYTES)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut data = Vec::new();
    File::open(path)?.take(limit).read_to_end(&mut data)?;
    Ok(data)
}

fn io_playlist_error(error: std::io::Error) -> PlaylistError {
    if error.kind() == std::io::ErrorKind::NotFound {
        PlaylistError::NotFound
    } else {
        PlaylistError::Io { kind: error.kind() }
    }
}

fn io_read_error(period: PeriodNum, error: std::io::Error) -> ReadError {
    if error.kind() == std::io::ErrorKind::NotFound {
        ReadError::Gone { period }
    } else {
        ReadError::Io { kind: error.kind() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read::ArchivedSegment;

    fn playlist(segments: usize) -> Arc<PeriodPlaylist> {
        let one = ArchivedSegment::new(
            SegmentSeq::new(0),
            Timestamp::ZERO,
            std::time::Duration::from_secs(2),
            PeriodNum::new(0),
            ByteSpan::new(0, std::num::NonZeroU64::new(1).expect("nonzero")),
        );
        Arc::new(PeriodPlaylist {
            segments: vec![one; segments],
            programmes: Vec::new(),
            closed: true,
            torn_bytes: 0,
            torn_records: 0,
        })
    }

    fn entry(segments: usize, last_used: u64) -> CacheEntry {
        CacheEntry {
            stamp: Stamp {
                mtime: None,
                len: 0,
                inode: 0,
            },
            playlist: playlist(segments),
            last_used,
        }
    }

    #[test]
    fn a_failed_media_sync_is_reported_not_swallowed() {
        let dir = std::env::temp_dir().join(format!("dvr-sync-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let media = dir.join(layout::media_file_name(
            PeriodNum::new(0),
            ArchiveContainer::Ts,
        ));
        fs::write(&media, b"bytes").expect("media");
        let mut permissions = fs::metadata(&media).expect("meta").permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&media, permissions).expect("readonly");
        let result = store_op(
            &dir,
            ArchiveContainer::Ts,
            &StoreOp::SyncData {
                period: PeriodNum::new(0),
            },
        );
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(
            result.map_err(|failed| failed.failure),
            Err(StoreFailure::Io(std::io::ErrorKind::PermissionDenied))
        );
    }

    #[test]
    fn a_missing_playlist_is_not_a_sync_failure() {
        let dir = std::env::temp_dir().join(format!("dvr-sync-ok-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let media = dir.join(layout::media_file_name(
            PeriodNum::new(0),
            ArchiveContainer::Ts,
        ));
        fs::write(&media, b"bytes").expect("media");
        let result = store_op(
            &dir,
            ArchiveContainer::Ts,
            &StoreOp::SyncData {
                period: PeriodNum::new(0),
            },
        );
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(result.map_err(|failed| failed.failure), Ok(()));
    }

    fn archive(tag: &str) -> FsArchive {
        FsArchive::open(
            Path::new(tag),
            &ArchiveName::new("a").expect("name"),
            ArchiveContainer::Fmp4,
        )
    }

    #[test]
    fn a_vanished_playlist_is_not_found_and_an_unreadable_one_is_io() {
        let root = PathBuf::from("target/dvr-runtime-tests/typed-errors");
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("a");
        std::fs::create_dir_all(&dir).expect("dir");
        let name = ArchiveName::new("a").expect("name");
        let mut archive = FsArchive::open(&root, &name, ArchiveContainer::Ts);
        let playlist = dir.join("p0.m3u8");
        std::fs::write(&playlist, b"#EXTM3U\n").expect("write");
        assert!(matches!(
            archive.playlist(PeriodNum::new(0)),
            Err(PlaylistError::Hls(_))
        ));
        std::fs::remove_file(&playlist).expect("remove");
        assert!(matches!(
            archive.playlist(PeriodNum::new(0)),
            Err(PlaylistError::NotFound)
        ));
        std::fs::write(&playlist, b"#EXTM3U\n").expect("write");
        let mut perms = std::fs::metadata(&playlist).expect("meta").permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            perms.set_mode(0o000);
            std::fs::set_permissions(&playlist, perms).expect("chmod");
            let result = archive.playlist(PeriodNum::new(0));
            let mut restore = std::fs::metadata(&playlist).expect("meta").permissions();
            restore.set_mode(0o644);
            std::fs::set_permissions(&playlist, restore).expect("restore");
            if !running_as_root() {
                assert!(
                    matches!(result, Err(PlaylistError::Io { .. })),
                    "expected an I/O error, got {result:?}"
                );
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    fn running_as_root() -> bool {
        std::process::Command::new("id")
            .arg("-u")
            .output()
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .is_some_and(|uid| uid.trim() == "0")
    }

    #[test]
    fn the_cache_evicts_only_the_least_recently_used_entry_at_the_exact_bound() {
        let mut archive = archive("first");
        for tick in 0..INDEX_CACHE_MAX_ENTRIES as u64 {
            archive.cache.insert(
                PeriodNum::new(u32::try_from(tick).expect("tick")),
                entry(1, tick),
            );
        }
        archive.enforce_bounds();
        assert_eq!(archive.cached_periods(), INDEX_CACHE_MAX_ENTRIES);
        archive.cache.insert(
            PeriodNum::new(u32::try_from(INDEX_CACHE_MAX_ENTRIES).expect("period")),
            entry(1, INDEX_CACHE_MAX_ENTRIES as u64),
        );
        archive.enforce_bounds();
        assert_eq!(archive.cached_periods(), INDEX_CACHE_MAX_ENTRIES);
        assert!(!archive.cache.contains_key(&PeriodNum::new(0)));
        assert!(
            archive
                .cache
                .contains_key(&PeriodNum::new(INDEX_CACHE_MAX_ENTRIES as u32))
        );
    }

    #[test]
    fn two_archives_bound_their_caches_independently() {
        let mut first = archive("first");
        let mut second = archive("second");
        first.cache.insert(PeriodNum::new(0), entry(1, 0));
        second.cache.insert(PeriodNum::new(0), entry(1, 0));
        second.cache.insert(PeriodNum::new(1), entry(1, 1));
        first.enforce_bounds();
        second.enforce_bounds();
        assert_eq!(first.cached_periods(), 1);
        assert_eq!(second.cached_periods(), 2);
    }

    #[test]
    fn the_cache_is_bounded_by_total_segments() {
        let per_entry = INDEX_CACHE_MAX_SEGMENTS / 4 + 1;
        let mut archive = archive("first");
        for tick in 0..4u64 {
            archive.cache.insert(
                PeriodNum::new(u32::try_from(tick).expect("tick")),
                entry(per_entry, tick),
            );
        }
        archive.enforce_bounds();
        let total: usize = archive
            .cache
            .values()
            .map(|entry| entry.playlist.segments.len())
            .sum();
        assert!(total <= INDEX_CACHE_MAX_SEGMENTS, "total segments {total}");
        assert!(
            archive.cached_periods() < 4,
            "the oldest entries were evicted"
        );
        assert!(archive.cache.contains_key(&PeriodNum::new(3)));
    }
}
