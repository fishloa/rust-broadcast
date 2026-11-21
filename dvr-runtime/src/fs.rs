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

use crate::layout::{self, ArchiveFile};
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
                        existing.push(file);
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
    /// # Errors
    ///
    /// [`FsError::Archive`] when the writer rejects an input;
    /// [`FsError::Io`] when a storage operation failed (the writer has
    /// already resynchronised its state and will retry on the next poll).
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
        self.handle_step(tick)?;
        loop {
            let Some(item) = self.cursor.poll() else {
                break;
            };
            let wall_time = match &item {
                SegmentCursorItem::Segment(entry) => wall(entry),
                _ => None,
            };
            let step = self.writer.on_item(now, wall_time, &item)?;
            self.handle_step(step)?;
        }
        Ok(())
    }

    /// Feed raw MPEG-2 TS bytes for EIT present/following tracking. A no-op
    /// unless the configuration names a programme service. Blocking (a
    /// transition rolls the period).
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
        for change in changes {
            let step = self.writer.on_programme(change)?;
            self.handle_step(step)?;
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

    fn handle_step(&mut self, step: Step) -> Result<(), FsError> {
        match step {
            Step::Idle | Step::Stopped => Ok(()),
            Step::Rearm { .. } => {
                self.cursor = self.trunk.pin_segments(self.writer.overrun());
                Ok(())
            }
            Step::Execute(plan) => self.execute(plan),
        }
    }

    fn execute(&mut self, plan: Plan) -> Result<(), FsError> {
        let mut outcome = PlanOutcome::Done;
        let mut error = None;
        for (at_op, op) in plan.ops.iter().enumerate() {
            if let Err(failure) = self.store_op(op) {
                error = match failure {
                    StoreFailure::Io(kind) => Some(io_error(
                        path_of(&self.dir, op, self.container),
                        op.kind(),
                        std::io::Error::from(kind),
                    )),
                    StoreFailure::LengthMismatch { .. } => None,
                };
                outcome = PlanOutcome::Failed { at_op, failure };
                break;
            }
        }
        self.writer.complete(plan.id, outcome)?;
        match error {
            Some(error) => Err(error),
            None => Ok(()),
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
                    .create(true)
                    .truncate(true)
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
                sync_file(&media)?;
                let playlist = self.dir.join(layout::playlist_file_name(*period));
                match sync_file(&playlist) {
                    Err(StoreFailure::Io(std::io::ErrorKind::NotFound)) => Ok(()),
                    other => other,
                }
            }
            StoreOp::CreatePlaylist { period, text } => {
                let tmp = self.dir.join(layout::playlist_tmp_name(*period));
                let path = self.dir.join(layout::playlist_file_name(*period));
                let write = || -> std::io::Result<()> {
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

impl SegmentEgress for FsRecorder {
    type Error = FsError;

    fn on_segment(&mut self, item: &SegmentCursorItem) -> Result<(), Self::Error> {
        let now = self.last_now;
        let step = self.writer.on_item(now, None, item)?;
        self.handle_step(step)
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
        let metadata = fs::metadata(&path).map_err(|_| PlaylistError::NotAnArchive)?;
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
        let data = fs::read(&path).map_err(|_| PlaylistError::NotAnArchive)?;
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

fn io_read_error(period: PeriodNum, error: std::io::Error) -> ReadError {
    if error.kind() == std::io::ErrorKind::NotFound {
        ReadError::Gone { period }
    } else {
        ReadError::Io { kind: error.kind() }
    }
}
