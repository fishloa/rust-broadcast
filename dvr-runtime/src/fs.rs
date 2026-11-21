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
use media_plane::egress::SegmentEgress;
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{SegmentCursor, SegmentCursorItem, SegmentEntry, Trunk};

use crate::layout::{self, ArchiveFile, SeedFile};
use crate::read::{
    ArchiveScan, ArchivedSegment, PeriodPlaylist, PlaylistError, ReadError, checked_range,
    parse_period_playlist,
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
    /// The store could not commit this input. The
    /// [`SegmentEgress`] adapter cannot queue it, so the caller must drive
    /// the same cursor item through [`FsRecorder::poll`].
    #[error("the store did not commit this input; retry it through FsRecorder::poll")]
    RetryRequired,
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
pub struct FsRecorder {
    writer: ArchiveWriter,
    cursor: SegmentCursor,
    trunk: Arc<Trunk>,
    dir: PathBuf,
    tracker: Option<dvb_si::epg::PresentEventTracker>,
    last_now: Timestamp,
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
    /// # Errors
    ///
    /// [`FsError::Archive`] for an invalid configuration or exhausted period
    /// numbers; [`FsError::Io`] when the archive directory exists but cannot
    /// be listed (a permission error is surfaced, never read as an empty
    /// archive).
    pub fn new(
        root: &Path,
        name: ArchiveName,
        config: ArchiveConfig,
        trunk: &Arc<Trunk>,
    ) -> Result<Self, FsError> {
        let dir = layout::route_dir(root, &name);
        let container = config.container;
        let mut existing = Vec::new();
        match fs::read_dir(&dir) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry
                        .map_err(|e| io_error(dir.clone(), crate::writer::StoreOpKind::List, e))?;
                    let file_name = entry.file_name();
                    let Some(file_name) = file_name.to_str() else {
                        continue;
                    };
                    if let Some(file) = layout::classify(file_name, container) {
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
        let cursor = trunk.pin_segments(config.overrun);
        let tracker = config
            .programme_service
            .map(dvb_si::epg::PresentEventTracker::new);
        Ok(Self {
            writer,
            cursor,
            trunk: Arc::clone(trunk),
            dir,
            tracker,
            last_now: Timestamp::ZERO,
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
        self.last_now = now;
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
            if let Err(failure) = self.store_op(op) {
                if let StoreFailure::Io(kind) = failure {
                    error = Some(io_error(
                        path_of(&self.dir, op, self.container),
                        op.kind(),
                        std::io::Error::from(kind),
                    ));
                }
                outcome = PlanOutcome::Failed { at_op, failure };
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

impl FsRecorder {
    fn store_op(&self, op: &StoreOp) -> Result<(), StoreFailure> {
        match op {
            StoreOp::CreateDir => {
                fs::create_dir_all(&self.dir).map_err(|e| StoreFailure::Io(e.kind()))
            }
            StoreOp::CreateData { period } => {
                let path = self
                    .dir
                    .join(layout::media_file_name(*period, self.container));
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .map(|_| ())
                    .map_err(|e| StoreFailure::Io(e.kind()))
            }
            StoreOp::AppendData {
                period,
                expect_len,
                bytes,
            } => {
                let path = self
                    .dir
                    .join(layout::media_file_name(*period, self.container));
                append_at(&path, *expect_len, bytes)
            }
            StoreOp::SyncData { period } => {
                let media = self
                    .dir
                    .join(layout::media_file_name(*period, self.container));
                let mut failed = sync_file(&media).is_err();
                let playlist = self.dir.join(layout::playlist_file_name(*period));
                match sync_file(&playlist) {
                    Err(StoreFailure::Io(std::io::ErrorKind::NotFound)) => {}
                    Err(_) => failed = true,
                    Ok(()) => {}
                }
                if failed {
                    eprintln!(
                        "dvr-runtime: syncing period {period} failed; durability is not guaranteed"
                    );
                }
                Ok(())
            }
            StoreOp::CreatePlaylist { period, text } => {
                let tmp = self.dir.join(layout::playlist_tmp_name(*period));
                let path = self.dir.join(layout::playlist_file_name(*period));
                let write = || -> std::io::Result<()> {
                    if path.exists() {
                        return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
                    }
                    let mut file = File::create(&tmp)?;
                    file.write_all(text.as_bytes())?;
                    file.sync_all()?;
                    fs::rename(&tmp, &path)?;
                    File::open(&self.dir)?.sync_all()
                };
                write().map_err(|e| StoreFailure::Io(e.kind()))
            }
            StoreOp::AppendPlaylist {
                period,
                expect_len,
                text,
            } => {
                let path = self.dir.join(layout::playlist_file_name(*period));
                append_at(&path, *expect_len, text.as_bytes())
            }
            StoreOp::Remove { file } => {
                let path = self.dir.join(layout::file_name(*file, self.container));
                match fs::remove_file(path) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(e) => Err(StoreFailure::Io(e.kind())),
                }
            }
        }
    }
}

fn sync_file(path: &Path) -> Result<(), StoreFailure> {
    OpenOptions::new()
        .append(true)
        .open(path)
        .and_then(|file| file.sync_data())
        .map_err(|e| StoreFailure::Io(e.kind()))
}

fn append_at(path: &Path, expect_len: u64, bytes: &[u8]) -> Result<(), StoreFailure> {
    let mut file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| StoreFailure::Io(e.kind()))?;
    let actual = file
        .metadata()
        .map_err(|e| StoreFailure::Io(e.kind()))?
        .len();
    if actual != expect_len {
        let settled = if actual > expect_len {
            file.set_len(expect_len)
                .map_err(|e| StoreFailure::Io(e.kind()))?;
            expect_len
        } else {
            actual
        };
        return Err(StoreFailure::LengthMismatch { actual: settled });
    }
    let result = file
        .seek(SeekFrom::End(0))
        .and_then(|_| file.write_all(bytes));
    if let Err(e) = result {
        let _ = file.set_len(expect_len);
        return Err(StoreFailure::Io(e.kind()));
    }
    Ok(())
}

/// The egress adapter drives the writer without a wall-clock source, so a
/// writer configured with a programme service is rejected up front rather
/// than failing every segment with `WallClockRequired`; use
/// [`FsRecorder::poll`] for that configuration.
impl SegmentEgress for FsRecorder {
    type Error = FsError;

    fn on_segment(&mut self, item: &SegmentCursorItem) -> Result<(), Self::Error> {
        if self.tracker.is_some() {
            return Err(ArchiveError::WallClockRequired.into());
        }
        let step = self.writer.on_item(self.last_now, None, item)?;
        match self.handle_step(step)? {
            Execution::Consumed => Ok(()),
            Execution::Retry => Err(FsError::RetryRequired),
        }
    }
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
    /// could not be read.
    pub fn scan(&mut self) -> ArchiveScan {
        let mut scan = ArchiveScan::default();
        for period in self.periods() {
            match self.playlist(period) {
                Ok(playlist) => scan.segments.extend(playlist.segments.iter().copied()),
                Err(error) => scan.unreadable.push((period, error)),
            }
        }
        self.prune();
        scan
    }

    /// The archived segment numbered `seq`, if any.
    pub fn find(&mut self, seq: SegmentSeq) -> Option<ArchivedSegment> {
        for period in self.periods() {
            if let Ok(playlist) = self.playlist(period)
                && let Some(segment) = playlist.segments.iter().find(|s| s.seq == seq)
            {
                return Some(*segment);
            }
        }
        None
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

    /// The highest archived sequence number (`0` for an empty archive), for a
    /// restarted route to number above.
    pub fn seq_floor(&mut self) -> SegmentSeq {
        self.scan()
            .segments
            .iter()
            .map(|s| s.seq)
            .max()
            .unwrap_or_else(|| SegmentSeq::new(0))
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

    fn periods(&self) -> Vec<PeriodNum> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut periods: Vec<PeriodNum> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name();
                let name = name.to_str()?;
                match layout::classify(name, self.container)? {
                    ArchiveFile::Playlist(period) => Some(period),
                    ArchiveFile::Media(_) => None,
                }
            })
            .collect();
        periods.sort_unstable();
        periods
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
        let data = fs::read(&path).map_err(io_playlist_error)?;
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
