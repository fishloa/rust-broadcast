//! The sans-IO archive writer: a roll/retention state machine that returns an
//! ordered [`Plan`] of storage operations per input.
//!
//! The writer never touches a file. Every decision that changes the archive
//! becomes a [`StoreOp`] in one plan; the host executes the ops in order and
//! reports the outcome with [`ArchiveWriter::complete`]. Only a
//! [`PlanOutcome::Done`] commits state, so a failed roll leaves the recorder
//! exactly as it was, a partial write's length is resynchronised from the
//! store's own report rather than trusted from a cached offset, and the whole
//! state machine is testable without a disk.
//!
//! # Roll conditions
//!
//! A period rolls on: the first segment; a sequence gap; a segment whose
//! rounded duration exceeds the playlist's `EXT-X-TARGETDURATION`; the
//! configured whole-period byte budget; a record that would push the playlist
//! past `broadcast_hls::limits::MAX_PLAYLIST_BYTES`; an fMP4 init change; the
//! configured period duration (on the caller's monotonic clock); and a
//! programme transition.
//!
//! # Playlist creation
//!
//! A period's playlist is created at its first segment, not when the period
//! rolls, because `EXT-X-MEDIA-SEQUENCE` is that segment's archive sequence
//! number. A period with an init and no segment therefore has no playlist and
//! is not listed.

use std::collections::VecDeque;
use std::time::Duration;

use broadcast_common::Timestamp;
use broadcast_hls::{
    ByteRange, DecimalSeconds, MapTag, MediaPlaylist, MediaSegment, MediaSequence, PlaylistType,
    ProgramDateTime,
};
use bytes::Bytes;
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{ArchiveOverrun, SegmentCursorItem, SegmentEntry};

use crate::format;
use crate::layout::{self, ArchiveFile, SeedFile};
use crate::{ArchiveConfig, ArchiveContainer, ArchiveError, PeriodNum};

/// Store operations a period removal plans: its playlist, then its media.
const REMOVAL_OPS_PER_PERIOD: usize = 2;

/// First numeric suffix [`unique_daterange_id`] tries after the bare base id.
const FIRST_DATERANGE_ID_SUFFIX: u64 = 2;

/// One fMP4 init segment and the first Trunk segment number closed under it.
///
/// Generations are queued with [`ArchiveWriter::on_inits`]; a segment is
/// archived under the newest generation whose `first_seq` is at or before its
/// own, and a generation that differs from the open period's init rolls the
/// period (one init per period).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct InitGeneration {
    /// The first segment closed under this init.
    pub first_seq: SegmentSeq,
    /// The init bytes.
    pub init: Bytes,
}

impl InitGeneration {
    /// An init generation and the first segment closed under it.
    #[must_use]
    pub fn new(first_seq: SegmentSeq, init: Bytes) -> Self {
        Self { first_seq, init }
    }
}

/// A plan's identity, unique for the life of one writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[non_exhaustive]
pub struct PlanId(u64);

impl PlanId {
    /// The raw number.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for PlanId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

/// An ordered list of storage operations the host executes in order.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Plan {
    /// This plan's identity, echoed back to [`ArchiveWriter::complete`].
    pub id: PlanId,
    /// The operations, in execution order.
    pub ops: Vec<StoreOp>,
}

/// One storage operation.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum StoreOp {
    /// Create the archive directory (and any parent).
    CreateDir,
    /// Create the period's media file, failing if it already exists
    /// (`create_new`). A store must never truncate an existing file: an
    /// earlier run, or an earlier failed plan of this run, may own those
    /// bytes.
    CreateData {
        /// The period whose media file this is.
        period: PeriodNum,
    },
    /// Append `bytes` to the period's media file, which must currently be
    /// exactly `expect_len` bytes long.
    AppendData {
        /// The period whose media file this is.
        period: PeriodNum,
        /// The length the file must have before the append.
        expect_len: u64,
        /// The bytes to append.
        bytes: Bytes,
    },
    /// Make the period's media and playlist files durable.
    SyncData {
        /// The period to sync.
        period: PeriodNum,
    },
    /// Atomically create the period's playlist with `text` (the header plus
    /// its first segment record), failing if it already exists. The store
    /// writes a temporary file, syncs it, refuses a rename over an existing
    /// playlist, renames, and syncs the directory.
    CreatePlaylist {
        /// The period whose playlist this is.
        period: PeriodNum,
        /// The complete playlist text.
        text: String,
    },
    /// Append `text` (one segment record, or `#EXT-X-ENDLIST`) to the
    /// period's playlist, which must currently be exactly `expect_len` bytes
    /// long.
    AppendPlaylist {
        /// The period whose playlist this is.
        period: PeriodNum,
        /// The length the file must have before the append.
        expect_len: u64,
        /// The lines to append.
        text: String,
    },
    /// Remove one archive file. `NotFound` counts as done.
    Remove {
        /// The file to remove.
        file: ArchiveFile,
    },
}

impl StoreOp {
    /// The period the operation works on, when it names one.
    #[must_use]
    pub fn period(&self) -> Option<PeriodNum> {
        match self {
            Self::CreateDir => None,
            Self::CreateData { period }
            | Self::AppendData { period, .. }
            | Self::SyncData { period }
            | Self::CreatePlaylist { period, .. }
            | Self::AppendPlaylist { period, .. } => Some(*period),
            Self::Remove { file } => Some(file.period()),
        }
    }

    /// The operation's kind, for error context.
    #[must_use]
    pub fn kind(&self) -> StoreOpKind {
        match self {
            Self::CreateDir => StoreOpKind::CreateDir,
            Self::CreateData { .. } => StoreOpKind::CreateData,
            Self::AppendData { .. } => StoreOpKind::AppendData,
            Self::SyncData { .. } => StoreOpKind::SyncData,
            Self::CreatePlaylist { .. } => StoreOpKind::CreatePlaylist,
            Self::AppendPlaylist { .. } => StoreOpKind::AppendPlaylist,
            Self::Remove { .. } => StoreOpKind::Remove,
        }
    }
}

/// The kind of a [`StoreOp`], for a host's error context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StoreOpKind {
    /// [`StoreOp::CreateDir`].
    CreateDir,
    /// [`StoreOp::CreateData`].
    CreateData,
    /// [`StoreOp::AppendData`].
    AppendData,
    /// [`StoreOp::SyncData`].
    SyncData,
    /// [`StoreOp::CreatePlaylist`].
    CreatePlaylist,
    /// [`StoreOp::AppendPlaylist`].
    AppendPlaylist,
    /// [`StoreOp::Remove`].
    Remove,
    /// Listing the archive directory while seeding.
    List,
    /// Opening or locking the archive directory's lock file.
    Lock,
}

impl StoreOpKind {
    /// A stable label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::CreateDir => "create_dir",
            Self::CreateData => "create_data",
            Self::AppendData => "append_data",
            Self::SyncData => "sync_data",
            Self::CreatePlaylist => "create_playlist",
            Self::AppendPlaylist => "append_playlist",
            Self::Remove => "remove",
            Self::List => "list",
            Self::Lock => "lock",
        }
    }
}

broadcast_common::impl_spec_display!(StoreOpKind);

/// Why one storage operation failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreFailure {
    /// The file was not the expected length. The store truncates a too-long
    /// file back to the expected length and reports the length it settled on,
    /// which the writer adopts.
    LengthMismatch {
        /// The length the file really has after any truncation.
        actual: u64,
    },
    /// An I/O error, by kind.
    Io(std::io::ErrorKind),
    /// The operation's effect is in place and visible (a playlist renamed
    /// into place, bytes appended), but making it durable failed. The writer
    /// counts the operation as done, emits [`WriterEvent::NotDurable`], and
    /// treats the operations after it as not attempted.
    NotDurable(std::io::ErrorKind),
}

/// What the host did with a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PlanOutcome {
    /// Every operation succeeded.
    Done,
    /// The operation at `at_op` failed; operations before it succeeded.
    Failed {
        /// The index of the failed operation.
        at_op: usize,
        /// Why it failed.
        failure: StoreFailure,
    },
}

/// Why a period was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RollReason {
    /// No period was open yet (the first segment of a run).
    First,
    /// The configured period duration elapsed.
    Duration,
    /// The next segment would exceed the per-period byte budget.
    ByteBudget,
    /// A programme transition.
    Programme,
    /// The segment does not continue the archive's sequence numbers.
    SequenceGap,
    /// The segment is longer than the playlist's `EXT-X-TARGETDURATION`.
    TargetDuration,
    /// The record would push the playlist past `MAX_PLAYLIST_BYTES`.
    PlaylistFull,
    /// The fMP4 init segment changed.
    InitChanged,
}

impl RollReason {
    /// A stable label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Duration => "duration",
            Self::ByteBudget => "byte_budget",
            Self::Programme => "programme",
            Self::SequenceGap => "sequence_gap",
            Self::TargetDuration => "target_duration",
            Self::PlaylistFull => "playlist_full",
            Self::InitChanged => "init_changed",
        }
    }
}

broadcast_common::impl_spec_display!(RollReason);

/// Something the writer decided, for the host to log or count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum WriterEvent {
    /// A period was opened.
    PeriodOpened {
        /// The period.
        period: PeriodNum,
        /// Why.
        reason: RollReason,
    },
    /// A period was closed with `#EXT-X-ENDLIST`.
    PeriodClosed {
        /// The period.
        period: PeriodNum,
    },
    /// A closed period was evicted by retention.
    Evicted {
        /// The period.
        period: PeriodNum,
    },
    /// Retention wanted to evict a period but the host reported a failed
    /// removal; the period stays in the ledger.
    EvictionFailed {
        /// The period.
        period: PeriodNum,
    },
    /// The live ring evicted segments before the writer consumed them.
    Gap {
        /// How many segments were lost.
        skipped: u64,
    },
    /// A `StallIngest` pin was force-expired; the host must re-pin and the
    /// skipped segments are lost.
    Rearmed {
        /// How many segments were lost.
        skipped: u64,
    },
    /// Seeding found and the host removed a stale `.pN.m3u8.tmp` (a crash
    /// between the temp write and the rename); it never held a committed
    /// record, but its period number stays consumed.
    StaleTempPlaylistRemoved {
        /// The period the temp file belonged to.
        period: PeriodNum,
    },
    /// The writer stopped permanently.
    Terminated,
    /// An fMP4 segment arrived before any init was known and was dropped.
    SkippedBeforeInit {
        /// The dropped segment's Trunk sequence number.
        seq: SegmentSeq,
    },
    /// An operation took effect but the store could not make it durable
    /// ([`StoreFailure::NotDurable`]); the archive is visible but a power
    /// loss may lose it.
    NotDurable {
        /// The operation.
        op: StoreOpKind,
        /// The period it worked on, when it names one.
        period: Option<PeriodNum>,
        /// The I/O error, by kind.
        kind: std::io::ErrorKind,
    },
}

/// Cumulative writer counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct WriterCounters {
    /// Segments the live ring lost before the writer consumed them.
    pub gaps: u64,
    /// Segments committed to a period.
    pub segments: u64,
    /// Segment payload bytes committed.
    pub bytes: u64,
}

/// What the writer wants the host to do next.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Step {
    /// Nothing to do.
    Idle,
    /// Execute this plan and report the outcome.
    Execute(Plan),
    /// Re-pin the cursor at the live edge; `skipped` segments were lost.
    Rearm {
        /// How many segments were lost.
        skipped: u64,
    },
    /// Recording stopped permanently.
    Stopped,
}

#[derive(Debug, Clone, Copy)]
struct ClosedPeriod {
    period: PeriodNum,
    file_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
struct OpenPeriod {
    period: PeriodNum,
    media_len: u64,
    playlist_len: u64,
    target_duration: u32,
    opened_at: Timestamp,
    init: Option<Bytes>,
    segments: u64,
    daterange_ids: Vec<String>,
}

#[derive(Debug, Clone)]
struct EvictedPeriod {
    period: PeriodNum,
}

enum Commit {
    Segment(SegmentPost),
    Close(ClosePost),
}

struct SegmentPost {
    closed_now: Option<ClosedPeriod>,
    opened_reason: RollReason,
    opened: OpenPeriod,
    seq_archive: u64,
    trunk_seq: SegmentSeq,
    bytes: u64,
    evicted: Vec<EvictedPeriod>,
    first_remove_op: usize,
    consumed_inits: usize,
    attached_programme: bool,
    next_period: PeriodNum,
    clear_next_reason: bool,
    rolled: bool,
    new_period: PeriodNum,
    old_close_op: Option<usize>,
    create_data_op: Option<usize>,
    record_op: usize,
}

struct ClosePost {
    closed_now: ClosedPeriod,
    next_period: PeriodNum,
    next_reason: Option<RollReason>,
    next_programme: Option<dvb_si::epg::PresentEvent>,
}

struct Pending {
    id: PlanId,
    plan: Plan,
    commit: Commit,
}

/// The sans-IO archive writer.
pub struct ArchiveWriter {
    config: ArchiveConfig,
    next_period: PeriodNum,
    open: Option<OpenPeriod>,
    closed: Vec<ClosedPeriod>,
    programme: Option<dvb_si::epg::PresentEvent>,
    attach_programme: bool,
    next_reason: Option<RollReason>,
    last_archive_seq: Option<u64>,
    last_appended_seq: Option<SegmentSeq>,
    latest_closed: Option<SegmentSeq>,
    last_known_init: Option<Bytes>,
    pending_inits: Vec<InitGeneration>,
    pending: Option<Pending>,
    events: VecDeque<WriterEvent>,
    counters: WriterCounters,
    stopped: bool,
    starts_run: bool,
    next_plan: u64,
    current_init: Option<Bytes>,
    last_input_consumed: bool,
    periods_exhausted: bool,
}

impl ArchiveWriter {
    /// Create a writer for `config`. It has no period open and nothing
    /// seeded.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::Config`] when the configuration is invalid.
    pub fn new(config: ArchiveConfig) -> Result<Self, ArchiveError> {
        config.validate()?;
        Ok(Self {
            config,
            next_period: PeriodNum::new(0),
            open: None,
            closed: Vec::new(),
            programme: None,
            attach_programme: false,
            next_reason: None,
            last_archive_seq: None,
            last_appended_seq: None,
            latest_closed: None,
            last_known_init: None,
            pending_inits: Vec::new(),
            pending: None,
            events: VecDeque::new(),
            counters: WriterCounters::default(),
            stopped: false,
            starts_run: false,
            next_plan: 0,
            current_init: None,
            last_input_consumed: false,
            periods_exhausted: false,
        })
    }

    /// Adopt the periods an earlier run left on disk: the next period number
    /// is one past the highest period number of **any** archive file present
    /// (a media file with no playlist, left by a crash between creating a
    /// period's media file and its playlist, still owns its number because
    /// the store creates files with `create_new`). Every adopted period,
    /// including such an orphan media file, enters the retention ledger with
    /// its media file size and is evicted like any other period. The first
    /// segment of this run is marked discontinuous.
    ///
    /// Seq-floor contract: seeding reads no playlists, so it cannot see the
    /// archived sequence numbers. The caller must configure
    /// [`ArchiveConfig::seq_offset`] so this run's archive sequence numbers
    /// (Trunk sequence plus offset) are above the highest archived one
    /// (`FsArchive::seq_floor` under the `fs` feature). A run that restarts
    /// at or below an archived sequence still records, but the reader keeps
    /// the earlier period's record and reports the later claim as a
    /// `SeqConflict`.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::PlanPending`] while a plan is outstanding,
    /// [`ArchiveError::PeriodNumbersExhausted`] when an archive file numbered
    /// `u32::MAX` exists.
    pub fn seed(&mut self, existing: &[SeedFile]) -> Result<(), ArchiveError> {
        if self.pending.is_some() {
            return Err(ArchiveError::PlanPending);
        }
        let mut media: std::collections::BTreeMap<PeriodNum, u64> =
            std::collections::BTreeMap::new();
        let mut playlists: std::collections::BTreeMap<PeriodNum, u64> =
            std::collections::BTreeMap::new();
        for seeded in existing {
            match seeded.file {
                ArchiveFile::Media(period) => {
                    media.insert(period, seeded.bytes);
                }
                ArchiveFile::Playlist(period) => {
                    playlists.insert(period, seeded.bytes);
                }
            }
        }
        let highest = playlists
            .keys()
            .next_back()
            .copied()
            .max(media.keys().next_back().copied());
        let mut adopted = media.clone();
        for (period, playlist_bytes) in playlists {
            adopted.entry(period).or_insert(playlist_bytes);
        }
        for (period, file_bytes) in adopted {
            self.adopt(period, Some(file_bytes));
        }
        if let Some(highest) = highest {
            self.next_period = highest
                .checked_next()
                .ok_or(ArchiveError::PeriodNumbersExhausted)?;
            self.starts_run = true;
        }
        Ok(())
    }

    /// Record stale temp playlists the host removed while listing the
    /// directory: each period number stays consumed (the next period is
    /// above it) and a [`WriterEvent::StaleTempPlaylistRemoved`] is queued.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::PlanPending`] while a plan is outstanding,
    /// [`ArchiveError::PeriodNumbersExhausted`] when a temp file is numbered
    /// `u32::MAX`.
    pub fn note_stale_temp_playlists(&mut self, periods: &[PeriodNum]) -> Result<(), ArchiveError> {
        if self.pending.is_some() {
            return Err(ArchiveError::PlanPending);
        }
        for &period in periods {
            let next = period
                .checked_next()
                .ok_or(ArchiveError::PeriodNumbersExhausted)?;
            if next > self.next_period {
                self.next_period = next;
                self.starts_run = true;
            }
            self.events
                .push_back(WriterEvent::StaleTempPlaylistRemoved { period });
        }
        Ok(())
    }

    /// A cheap "is there anything to do" check: a closed segment this writer
    /// has not archived, or an init it has not yet seen. When this is
    /// `false` the host can skip the poll entirely.
    #[must_use]
    pub fn needs_poll(
        &self,
        latest_closed: Option<SegmentSeq>,
        latest_init: Option<&[u8]>,
    ) -> bool {
        if self.stopped {
            return false;
        }
        if self.is_fmp4()
            && let Some(latest) = latest_init
            && self.last_known_init.as_deref() != Some(latest)
        {
            return true;
        }
        match latest_closed {
            Some(latest) => self.last_appended_seq != Some(latest),
            None => false,
        }
    }

    /// Queue fMP4 init generations (oldest first). No I/O happens here; a
    /// generation rolls the period when the first segment closed under it
    /// arrives. Ignored for a TS archive.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::PlanPending`] while a plan is outstanding.
    pub fn on_inits(&mut self, inits: &[InitGeneration]) -> Result<(), ArchiveError> {
        self.guard()?;
        if !self.is_fmp4() {
            return Ok(());
        }
        for generation in inits {
            self.last_known_init = Some(generation.init.clone());
            self.pending_inits.push(generation.clone());
        }
        self.pending_inits.sort_by_key(|g| g.first_seq);
        Ok(())
    }

    /// Roll the open period if it has been open for `config.period_duration`
    /// on the caller's monotonic clock.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::PlanPending`] while a plan is outstanding,
    /// [`ArchiveError::PeriodNumbersExhausted`] at the end of the number
    /// range.
    pub fn on_tick(&mut self, now: Timestamp) -> Result<Step, ArchiveError> {
        self.guard()?;
        if self.stopped {
            return Ok(Step::Idle);
        }
        let Some(open) = &self.open else {
            return Ok(Step::Idle);
        };
        let Some(limit) = self.config.period_duration else {
            return Ok(Step::Idle);
        };
        if now.saturating_sub(open.opened_at) >= limit {
            return self.close_plan(RollReason::Duration, None);
        }
        Ok(Step::Idle)
    }

    /// Record the Trunk's newest closed segment, used to size the gap when a
    /// `StallIngest` pin is force-expired.
    pub fn set_latest_closed(&mut self, latest: Option<SegmentSeq>) {
        self.latest_closed = latest;
    }

    /// Handle one cursor item.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::PlanPending`] while a plan is outstanding,
    /// [`ArchiveError::SeqOverflow`], [`ArchiveError::DurationOverflow`],
    /// [`ArchiveError::EmptySegment`], [`ArchiveError::WallClockRequired`],
    /// [`ArchiveError::Render`], [`ArchiveError::PeriodNumbersExhausted`].
    pub fn on_item(
        &mut self,
        now: Timestamp,
        wall: Option<jiff::Timestamp>,
        item: &SegmentCursorItem,
    ) -> Result<Step, ArchiveError> {
        self.guard()?;
        if self.stopped {
            return Ok(Step::Idle);
        }
        match item {
            SegmentCursorItem::Segment(entry) => self.segment_step(now, wall, entry),
            SegmentCursorItem::Gap { skipped } | SegmentCursorItem::Lagged { skipped } => {
                self.count_gap(*skipped);
                Ok(Step::Idle)
            }
            SegmentCursorItem::Terminated => Ok(self.terminated_step()),
            _ => Ok(Step::Idle),
        }
    }

    /// Apply a programme change: a transition rolls the open period; a
    /// baseline only attaches the programme to the next record. The same
    /// event id again changes nothing (no roll, no second `EXT-X-DATERANGE`).
    ///
    /// # Errors
    ///
    /// [`ArchiveError::PlanPending`] while a plan is outstanding,
    /// [`ArchiveError::PeriodNumbersExhausted`].
    pub fn on_programme(
        &mut self,
        change: dvb_si::epg::PresentEventChange,
    ) -> Result<Step, ArchiveError> {
        self.guard()?;
        if self.stopped {
            return Ok(Step::Idle);
        }
        use dvb_si::epg::PresentEventChange;
        let (current, transition) = match &change {
            PresentEventChange::Baseline(event) => (event, false),
            PresentEventChange::Transition { current, .. } => (current, true),
            _ => return Ok(Step::Idle),
        };
        if let Some(service) = self.config.programme_service
            && current.service_id != service
        {
            return Ok(Step::Idle);
        }
        if self
            .programme
            .as_ref()
            .is_some_and(|p| p.event.event_id == current.event.event_id)
        {
            return Ok(Step::Idle);
        }
        if transition && self.open.is_some() {
            return self.close_plan(RollReason::Programme, Some(current.clone()));
        }
        self.programme = Some(current.clone());
        self.attach_programme = true;
        Ok(Step::Idle)
    }

    /// Report what the host did with `plan`.
    ///
    /// Every operation **before** the reported failure index is a durable
    /// fact, and the writer applies it even when the plan as a whole failed:
    /// a completed `#EXT-X-ENDLIST` append closes that period (it is never
    /// appended to again and never closed twice), a completed create for
    /// period `N` consumes `N` (the writer never targets it again, because
    /// the store creates files with `create_new`), and a completed record
    /// append commits the segment. A failure inside a record therefore keeps
    /// the [input consumed](Self::last_input_consumed); a failure before it
    /// does not, and the host must retry that input.
    ///
    /// A [`StoreFailure::NotDurable`] counts its own operation as done (a
    /// playlist renamed into place is visible and will be read) and emits
    /// [`WriterEvent::NotDurable`]; only the operations after it count as not
    /// attempted. An `AlreadyExists` failure of `CreateData` or
    /// `CreatePlaylist` consumes that period number, so the retry takes the
    /// next one. A new period's media file left without its playlist enters
    /// the retention ledger and is evicted like any period.
    ///
    /// A failure at a retention `Remove` keeps that period in the ledger and
    /// emits [`WriterEvent::EvictionFailed`]; removals after it were not
    /// attempted and keep their periods too. A `LengthMismatch` resynchronises
    /// the open period's offset to the length the store reports.
    ///
    /// # Errors
    ///
    /// [`ArchiveError::UnknownPlan`] when `id` is not the outstanding plan's.
    pub fn complete(&mut self, id: PlanId, outcome: PlanOutcome) -> Result<(), ArchiveError> {
        let Some(pending) = self.pending.as_ref() else {
            return Err(ArchiveError::UnknownPlan { id });
        };
        if pending.id != id {
            return Err(ArchiveError::UnknownPlan { id });
        }
        let Pending { plan, commit, .. } = self
            .pending
            .take()
            .ok_or(ArchiveError::UnknownPlan { id })?;
        self.last_input_consumed = false;
        match outcome {
            PlanOutcome::Done => {
                self.commit(commit, usize::MAX, None);
                self.last_input_consumed = true;
            }
            PlanOutcome::Failed { at_op, failure } => {
                self.failed(&plan, commit, at_op, failure);
            }
        }
        Ok(())
    }

    fn failed(&mut self, plan: &Plan, commit: Commit, at_op: usize, failure: StoreFailure) {
        let failed_op = plan.ops.get(at_op);
        let done = match failure {
            StoreFailure::NotDurable(kind) => {
                if let Some(op) = failed_op {
                    self.emit(WriterEvent::NotDurable {
                        op: op.kind(),
                        period: op.period(),
                        kind,
                    });
                }
                at_op.saturating_add(1)
            }
            _ => at_op,
        };
        let create_exists = failure == StoreFailure::Io(std::io::ErrorKind::AlreadyExists)
            && matches!(
                failed_op,
                Some(StoreOp::CreateData { .. } | StoreOp::CreatePlaylist { .. })
            );
        match commit {
            Commit::Segment(post) => {
                let close_old = post.old_close_op.is_some_and(|index| index < done);
                let created_new = post.create_data_op.is_some_and(|index| index < done);
                let foreign_new = create_exists && post.create_data_op == Some(at_op);
                let record_committed = post.record_op < done;
                if record_committed {
                    let reached = done >= post.first_remove_op;
                    let applied = if reached {
                        (done - post.first_remove_op) / REMOVAL_OPS_PER_PERIOD
                    } else {
                        0
                    };
                    let failed_remove = match failed_op {
                        Some(StoreOp::Remove { file }) if reached && done == at_op => {
                            Some(file.period())
                        }
                        _ => None,
                    };
                    self.commit_segment(post, applied, failed_remove);
                    self.last_input_consumed = true;
                } else if close_old {
                    let reason = post.opened_reason;
                    let consumed = post.new_period;
                    if let Some(closed) = post.closed_now
                        && self
                            .open
                            .as_ref()
                            .is_some_and(|open| open.period == closed.period)
                    {
                        self.closed.push(closed);
                        self.emit(WriterEvent::PeriodClosed {
                            period: closed.period,
                        });
                        self.open = None;
                    }
                    self.next_reason = Some(reason);
                    self.consume_new(&plan.ops, done, failure, consumed, created_new, foreign_new);
                } else {
                    self.resync_offsets(&plan.ops, done, failure);
                    self.consume_new(
                        &plan.ops,
                        done,
                        failure,
                        post.new_period,
                        created_new,
                        foreign_new,
                    );
                }
            }
            Commit::Close(post) => {
                if done > 0 {
                    self.commit_close(post);
                    self.last_input_consumed = true;
                } else {
                    self.resync_offsets(&plan.ops, done, failure);
                }
            }
        }
    }

    /// A new period's number is consumed once its media file was created by
    /// this plan, or found to exist already (`create_new` refused it). The
    /// media file is then an orphan with no playlist this writer will ever
    /// write, and enters the retention ledger with the bytes the plan
    /// appended to it (unknown for a file this writer did not create).
    fn consume_new(
        &mut self,
        ops: &[StoreOp],
        done: usize,
        failure: StoreFailure,
        period: PeriodNum,
        created: bool,
        foreign: bool,
    ) {
        if created {
            let mut len = 0u64;
            for op in ops.iter().take(done) {
                if let StoreOp::AppendData {
                    period: target,
                    expect_len,
                    bytes,
                } = op
                    && *target == period
                {
                    len = expect_len.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
                }
            }
            if let (
                StoreFailure::LengthMismatch { actual },
                Some(StoreOp::AppendData { period: target, .. }),
            ) = (failure, ops.get(done))
                && *target == period
            {
                len = actual;
            }
            self.adopt(period, Some(len));
        } else if foreign {
            self.adopt(period, None);
        } else {
            return;
        }
        match period.checked_next() {
            Some(next) if next > self.next_period => self.next_period = next,
            Some(_) => {}
            None => self.periods_exhausted = true,
        }
    }

    fn adopt(&mut self, period: PeriodNum, file_bytes: Option<u64>) {
        if self.closed.iter().any(|closed| closed.period == period) {
            return;
        }
        self.closed.push(ClosedPeriod { period, file_bytes });
        self.closed.sort_by_key(|closed| closed.period);
    }

    /// Whether the input behind the most recently completed plan is durable.
    ///
    /// `true` after [`PlanOutcome::Done`], and after a failed plan whose
    /// record (or `#EXT-X-ENDLIST`) had already landed; `false` when the host
    /// must retry the same input. A host that executes plans uses this to tell
    /// a retryable I/O failure from one that already committed its segment.
    #[must_use]
    pub fn last_input_consumed(&self) -> bool {
        self.last_input_consumed
    }

    /// The next queued event, if any.
    pub fn poll_event(&mut self) -> Option<WriterEvent> {
        self.events.pop_front()
    }

    /// The configured overrun policy.
    #[must_use]
    pub fn overrun(&self) -> ArchiveOverrun {
        self.config.overrun
    }

    /// The current programme, once the tracked EIT present event has been
    /// observed.
    #[must_use]
    pub fn programme(&self) -> Option<&dvb_si::epg::PresentEvent> {
        self.programme.as_ref()
    }

    /// The cumulative counters.
    #[must_use]
    pub fn counters(&self) -> WriterCounters {
        self.counters
    }

    fn is_fmp4(&self) -> bool {
        self.config.container == ArchiveContainer::Fmp4
    }

    fn guard(&self) -> Result<(), ArchiveError> {
        if self.pending.is_some() {
            return Err(ArchiveError::PlanPending);
        }
        Ok(())
    }

    fn emit(&mut self, event: WriterEvent) {
        self.events.push_back(event);
    }

    fn count_gap(&mut self, skipped: u64) {
        self.counters.gaps = self.counters.gaps.saturating_add(skipped);
        self.emit(WriterEvent::Gap { skipped });
    }

    fn terminated_step(&mut self) -> Step {
        if self.config.overrun == ArchiveOverrun::StallIngest {
            let skipped = match (self.last_appended_seq, self.latest_closed) {
                (Some(last), Some(newest)) => {
                    u64::from(newest.checked_distance_from(last).unwrap_or(0))
                }
                _ => 0,
            };
            self.counters.gaps = self.counters.gaps.saturating_add(skipped);
            self.emit(WriterEvent::Rearmed { skipped });
            Step::Rearm { skipped }
        } else {
            self.stopped = true;
            self.emit(WriterEvent::Terminated);
            Step::Stopped
        }
    }

    fn next_plan_id(&mut self) -> PlanId {
        let id = PlanId(self.next_plan);
        self.next_plan = self.next_plan.saturating_add(1);
        id
    }

    fn endlist_text(&self) -> Result<String, ArchiveError> {
        let playlist = MediaPlaylist {
            endlist: true,
            ..MediaPlaylist::default()
        };
        Ok(playlist.render_endlist()?)
    }

    fn close_plan(
        &mut self,
        reason: RollReason,
        next_programme: Option<dvb_si::epg::PresentEvent>,
    ) -> Result<Step, ArchiveError> {
        let Some(open) = &self.open else {
            return Ok(Step::Idle);
        };
        let open_period = open.period;
        let open_playlist_len = open.playlist_len;
        let open_media_len = open.media_len;
        let endlist = self.endlist_text()?;
        let next_period = open_period
            .checked_next()
            .ok_or(ArchiveError::PeriodNumbersExhausted)?;
        let ops = vec![
            StoreOp::AppendPlaylist {
                period: open_period,
                expect_len: open_playlist_len,
                text: endlist,
            },
            StoreOp::SyncData {
                period: open_period,
            },
        ];
        let id = self.next_plan_id();
        let plan = Plan { id, ops };
        self.pending = Some(Pending {
            id,
            plan: plan.clone(),
            commit: Commit::Close(ClosePost {
                closed_now: ClosedPeriod {
                    period: open_period,
                    file_bytes: Some(open_media_len),
                },
                next_period,
                next_reason: Some(reason),
                next_programme,
            }),
        });
        Ok(Step::Execute(plan))
    }

    fn init_for(&self, seq: SegmentSeq) -> Option<Bytes> {
        let mut chosen = None;
        for generation in &self.pending_inits {
            if generation.first_seq <= seq {
                chosen = Some(generation.init.clone());
            }
        }
        if chosen.is_none() {
            chosen = self
                .open
                .as_ref()
                .and_then(|o| o.init.clone())
                .or_else(|| self.current_init.clone());
        }
        if chosen.is_none() && self.pending_inits.is_empty() {
            chosen = self.last_known_init.clone();
        }
        chosen
    }

    fn consumed_inits(&self, seq: SegmentSeq) -> usize {
        self.pending_inits
            .iter()
            .filter(|g| g.first_seq <= seq)
            .count()
    }

    fn over_budget(&self, open: &OpenPeriod, entry: &SegmentEntry) -> bool {
        let Some(budget) = self.config.retention.per_period_byte_budget() else {
            return false;
        };
        open.segments > 0
            && open
                .media_len
                .saturating_add(u64::try_from(entry.bytes.len()).unwrap_or(u64::MAX))
                > budget.get()
    }

    fn init_changed(&self, open: &OpenPeriod, init: Option<&Bytes>) -> bool {
        self.is_fmp4() && open.init.as_ref() != init
    }

    fn media_name(&self, period: PeriodNum) -> String {
        layout::media_file_name(period, self.config.container)
    }

    fn record(
        &self,
        period: PeriodNum,
        media_offset: u64,
        init_len: u64,
        archive_seq: u64,
        dur_nanos: u64,
        entry: &SegmentEntry,
        wall: Option<jiff::Timestamp>,
        discontinuous: bool,
        daterange_ids: &[String],
        attach: bool,
    ) -> Result<(MediaSegment, String, bool, Option<String>), ArchiveError> {
        let mut pre_tags = Vec::new();
        let mut attached = false;
        let mut new_id = None;
        if attach && let (Some(programme), Some(wall)) = (&self.programme, wall) {
            let base = format!(
                "programme-{}-{}",
                programme.service_id, programme.event.event_id
            );
            let id = unique_daterange_id(&base, daterange_ids);
            let range =
                format::programme_date_range(programme, ProgramDateTime::from_timestamp(wall))?;
            let range = range.with_id(id.clone())?;
            pre_tags.push(range.to_tag_line()?);
            attached = true;
            new_id = Some(id);
        }
        let timing = format::Timing {
            seq: archive_seq,
            dts: entry.timeline_position.as_nanos(),
            dur: dur_nanos,
            timescale: format::TIMING_TIMESCALE,
        };
        pre_tags.push(format::render_timing(timing)?);
        let segment = MediaSegment {
            uri: self.media_name(period),
            duration: DecimalSeconds::from_duration(entry.duration),
            discontinuous,
            byte_range: Some(ByteRange {
                length: u64::try_from(entry.bytes.len()).unwrap_or(u64::MAX),
                offset: Some(media_offset),
            }),
            map: self.is_fmp4().then(|| MapTag {
                uri: self.media_name(period),
                byte_range: Some(ByteRange {
                    length: init_len,
                    offset: Some(0),
                }),
                extra_attrs: Vec::new(),
            }),
            program_date_time: wall.map(ProgramDateTime::from_timestamp),
            pre_tags,
            ..MediaSegment::default()
        };
        let mut probe = MediaPlaylist {
            segments: vec![segment.clone(), segment.clone()],
            ..MediaPlaylist::default()
        };
        probe.version = probe.computed_version().unwrap_or(1);
        let text = probe.render_segment(1)?;
        Ok((segment, text, attached, new_id))
    }

    fn create_playlist_text(
        &self,
        period: PeriodNum,
        target_duration: u32,
        msn: u64,
        segment: MediaSegment,
    ) -> Result<String, ArchiveError> {
        let marker = format::render_marker(self.config.container, period)?;
        let mut playlist = MediaPlaylist {
            target_duration,
            msn: MediaSequence::new(msn),
            segments: vec![segment],
            playlist_type: Some(PlaylistType::Event),
            extra_tags: vec![marker],
            ..MediaPlaylist::default()
        };
        playlist.version = playlist.computed_version().unwrap_or(1);
        let header = playlist.render_header()?;
        let body = playlist.render_segment(0)?;
        Ok(header + &body)
    }

    fn evictions(&self, closed_now: Option<ClosedPeriod>, open_bytes: u64) -> Vec<EvictedPeriod> {
        let mut ledger = self.closed.clone();
        if let Some(closed) = closed_now {
            ledger.push(closed);
        }
        let mut evicted = Vec::new();
        if let Some(cap) = self.config.retention.bytes {
            loop {
                let known = ledger
                    .iter()
                    .filter_map(|c| c.file_bytes)
                    .fold(0u64, u64::saturating_add);
                if known.saturating_add(open_bytes) <= cap.get() || ledger.is_empty() {
                    break;
                }
                let period = ledger.remove(0);
                evicted.push(EvictedPeriod {
                    period: period.period,
                });
            }
        }
        if let Some(periods) = self.config.retention.periods {
            while ledger.len() > periods.get() {
                let period = ledger.remove(0);
                evicted.push(EvictedPeriod {
                    period: period.period,
                });
            }
        }
        evicted
    }

    fn removal_ops(evicted: &[EvictedPeriod]) -> Vec<StoreOp> {
        let mut ops = Vec::with_capacity(evicted.len() * REMOVAL_OPS_PER_PERIOD);
        for period in evicted {
            ops.push(StoreOp::Remove {
                file: ArchiveFile::Playlist(period.period),
            });
            ops.push(StoreOp::Remove {
                file: ArchiveFile::Media(period.period),
            });
        }
        ops
    }

    fn segment_step(
        &mut self,
        now: Timestamp,
        wall: Option<jiff::Timestamp>,
        entry: &SegmentEntry,
    ) -> Result<Step, ArchiveError> {
        let archive_seq_u64 = self
            .config
            .seq_offset
            .checked_add(u64::from(entry.sequence_number.value()))
            .filter(|v| u32::try_from(*v).is_ok())
            .ok_or(ArchiveError::SeqOverflow {
                trunk_seq: entry.sequence_number,
                offset: self.config.seq_offset,
            })?;
        let dur_nanos = u64::try_from(entry.duration.as_nanos()).map_err(|_| {
            ArchiveError::DurationOverflow {
                seq: entry.sequence_number,
            }
        })?;
        if entry.bytes.is_empty() {
            return Err(ArchiveError::EmptySegment {
                seq: entry.sequence_number,
            });
        }
        if self.config.programme_service.is_some() && wall.is_none() {
            return Err(ArchiveError::WallClockRequired);
        }
        let init = if self.is_fmp4() {
            match self.init_for(entry.sequence_number) {
                Some(value) => Some(value),
                None => {
                    self.emit(WriterEvent::SkippedBeforeInit {
                        seq: entry.sequence_number,
                    });
                    return Ok(Step::Idle);
                }
            }
        } else {
            None
        };
        let rounded = ceil_secs(entry.duration);
        let discontinuous = entry.meta.discontinuous || self.starts_run;
        let mut roll = None;
        if let Some(open) = &self.open
            && open.segments > 0
        {
            let expected = self.last_archive_seq.map(|seq| seq + 1);
            if expected != Some(archive_seq_u64) {
                roll = Some(RollReason::SequenceGap);
            } else if rounded > open.target_duration {
                roll = Some(RollReason::TargetDuration);
            } else if self.over_budget(open, entry) {
                roll = Some(RollReason::ByteBudget);
            } else {
                let init_len = open
                    .init
                    .as_ref()
                    .map_or(0, |b| u64::try_from(b.len()).unwrap_or(u64::MAX));
                let (_, probe, _, _) = self.record(
                    open.period,
                    open.media_len,
                    init_len,
                    archive_seq_u64,
                    dur_nanos,
                    entry,
                    wall,
                    discontinuous,
                    &open.daterange_ids,
                    self.attach_programme,
                )?;
                let endlist = self.endlist_text()?;
                if open
                    .playlist_len
                    .saturating_add(u64::try_from(probe.len()).unwrap_or(u64::MAX))
                    .saturating_add(u64::try_from(endlist.len()).unwrap_or(u64::MAX))
                    > u64::try_from(broadcast_hls::limits::MAX_PLAYLIST_BYTES).unwrap_or(u64::MAX)
                {
                    roll = Some(RollReason::PlaylistFull);
                } else if self.init_changed(open, init.as_ref()) {
                    roll = Some(RollReason::InitChanged);
                }
            }
        }
        let (period, media_offset, init_len, daterange_ids, opened_reason) =
            match (&self.open, roll) {
                (None, _) => {
                    let reason = self.next_reason.unwrap_or(RollReason::First);
                    let len = init
                        .as_ref()
                        .map_or(0, |b| u64::try_from(b.len()).unwrap_or(u64::MAX));
                    (self.next_period, len, len, Vec::new(), reason)
                }
                (Some(_), Some(reason)) => {
                    let len = init
                        .as_ref()
                        .map_or(0, |b| u64::try_from(b.len()).unwrap_or(u64::MAX));
                    (self.next_period, len, len, Vec::new(), reason)
                }
                (Some(open), None) => {
                    let len = open
                        .init
                        .as_ref()
                        .map_or(0, |b| u64::try_from(b.len()).unwrap_or(u64::MAX));
                    (
                        open.period,
                        open.media_len,
                        len,
                        open.daterange_ids.clone(),
                        RollReason::First,
                    )
                }
            };
        let rolled = !matches!((&self.open, roll), (Some(_), None));
        if rolled && self.periods_exhausted {
            return Err(ArchiveError::PeriodNumbersExhausted);
        }
        let attach = self.attach_programme || (rolled && self.programme.is_some());
        let discontinuous = discontinuous || roll == Some(RollReason::SequenceGap);
        let target_duration = match (&self.open, roll) {
            (Some(open), None) => open.target_duration,
            _ => ceil_secs(self.config.target_duration).max(rounded),
        };
        let (segment, record, attached, new_id) = self.record(
            period,
            media_offset,
            init_len,
            archive_seq_u64,
            dur_nanos,
            entry,
            wall,
            discontinuous,
            &daterange_ids,
            attach,
        )?;
        let byte_len = u64::try_from(entry.bytes.len()).unwrap_or(u64::MAX);
        let media_len = media_offset.saturating_add(byte_len);
        let mut ops = Vec::new();
        let mut old_close_op = None;
        let mut create_data_op = None;
        let record_op;
        let playlist_len = if rolled {
            if let Some(open) = &self.open {
                let endlist = self.endlist_text()?;
                old_close_op = Some(ops.len());
                ops.push(StoreOp::AppendPlaylist {
                    period: open.period,
                    expect_len: open.playlist_len,
                    text: endlist,
                });
                ops.push(StoreOp::SyncData {
                    period: open.period,
                });
            }
            ops.push(StoreOp::CreateDir);
            create_data_op = Some(ops.len());
            ops.push(StoreOp::CreateData { period });
            let init_bytes = init.clone().unwrap_or_default();
            if !init_bytes.is_empty() {
                ops.push(StoreOp::AppendData {
                    period,
                    expect_len: 0,
                    bytes: init_bytes.clone(),
                });
            }
            ops.push(StoreOp::AppendData {
                period,
                expect_len: init_len,
                bytes: entry.bytes.clone(),
            });
            ops.push(StoreOp::SyncData { period });
            let text =
                self.create_playlist_text(period, target_duration, archive_seq_u64, segment)?;
            let playlist_len = u64::try_from(text.len()).unwrap_or(u64::MAX);
            record_op = ops.len();
            ops.push(StoreOp::CreatePlaylist { period, text });
            ops.push(StoreOp::SyncData { period });
            playlist_len
        } else {
            ops.push(StoreOp::AppendData {
                period,
                expect_len: media_offset,
                bytes: entry.bytes.clone(),
            });
            ops.push(StoreOp::SyncData { period });
            let open = self
                .open
                .as_ref()
                .ok_or(ArchiveError::PeriodNumbersExhausted)?;
            let playlist_len = open
                .playlist_len
                .saturating_add(u64::try_from(record.len()).unwrap_or(u64::MAX));
            record_op = ops.len();
            ops.push(StoreOp::AppendPlaylist {
                period,
                expect_len: open.playlist_len,
                text: record,
            });
            ops.push(StoreOp::SyncData { period });
            playlist_len
        };
        let first_remove_op = ops.len();
        let closed_now = if rolled {
            self.open.as_ref().map(|open| ClosedPeriod {
                period: open.period,
                file_bytes: Some(open.media_len),
            })
        } else {
            None
        };
        let evicted = self.evictions(closed_now, media_len);
        ops.extend(Self::removal_ops(&evicted));
        let next_period = if rolled {
            period
                .checked_next()
                .ok_or(ArchiveError::PeriodNumbersExhausted)?
        } else {
            self.next_period
        };
        let mut daterange_ids = daterange_ids;
        if let Some(id) = new_id {
            daterange_ids.push(id);
        }
        let opened_at = match (&self.open, roll) {
            (Some(open), None) => open.opened_at,
            _ => now,
        };
        let opened = OpenPeriod {
            period,
            media_len,
            playlist_len,
            target_duration,
            opened_at,
            init: if self.is_fmp4() { init.clone() } else { None },
            segments: if rolled {
                1
            } else {
                self.open.as_ref().map_or(1, |open| open.segments + 1)
            },
            daterange_ids,
        };
        let post = SegmentPost {
            closed_now,
            opened_reason,
            opened,
            seq_archive: archive_seq_u64,
            trunk_seq: entry.sequence_number,
            bytes: byte_len,
            evicted,
            first_remove_op,
            consumed_inits: self.consumed_inits(entry.sequence_number),
            attached_programme: attached,
            next_period,
            clear_next_reason: self.open.is_none() && self.next_reason.is_some(),
            rolled,
            new_period: period,
            old_close_op,
            create_data_op,
            record_op,
        };
        let id = self.next_plan_id();
        let plan = Plan { id, ops };
        self.pending = Some(Pending {
            id,
            plan: plan.clone(),
            commit: Commit::Segment(post),
        });
        Ok(Step::Execute(plan))
    }

    fn commit(
        &mut self,
        commit: Commit,
        applied_removals: usize,
        failed_remove: Option<PeriodNum>,
    ) {
        match commit {
            Commit::Segment(post) => self.commit_segment(post, applied_removals, failed_remove),
            Commit::Close(post) => self.commit_close(post),
        }
    }

    fn commit_segment(
        &mut self,
        post: SegmentPost,
        applied_removals: usize,
        failed_remove: Option<PeriodNum>,
    ) {
        if let Some(closed) = post.closed_now {
            self.closed.push(closed);
            self.emit(WriterEvent::PeriodClosed {
                period: closed.period,
            });
        }
        for (index, evicted) in post.evicted.iter().enumerate() {
            if index < applied_removals {
                self.closed.retain(|c| c.period != evicted.period);
                self.emit(WriterEvent::Evicted {
                    period: evicted.period,
                });
            } else if failed_remove == Some(evicted.period) {
                self.emit(WriterEvent::EvictionFailed {
                    period: evicted.period,
                });
            }
        }
        if post.rolled {
            self.emit(WriterEvent::PeriodOpened {
                period: post.opened.period,
                reason: post.opened_reason,
            });
        }
        self.current_init = post.opened.init.clone();
        self.open = Some(post.opened);
        self.next_period = post.next_period;
        self.last_archive_seq = Some(post.seq_archive);
        self.last_appended_seq = Some(post.trunk_seq);
        self.counters.segments = self.counters.segments.saturating_add(1);
        self.counters.bytes = self.counters.bytes.saturating_add(post.bytes);
        if post.attached_programme {
            self.attach_programme = false;
        }
        self.starts_run = false;
        for _ in 0..post.consumed_inits {
            if !self.pending_inits.is_empty() {
                self.pending_inits.remove(0);
            }
        }
        if post.clear_next_reason {
            self.next_reason = None;
        }
    }

    fn commit_close(&mut self, post: ClosePost) {
        self.closed.push(post.closed_now);
        self.emit(WriterEvent::PeriodClosed {
            period: post.closed_now.period,
        });
        self.open = None;
        self.next_period = post.next_period;
        self.next_reason = post.next_reason;
        if let Some(programme) = post.next_programme {
            self.programme = Some(programme);
            self.attach_programme = true;
        } else {
            self.attach_programme = self.attach_programme || self.programme.is_some();
        }
    }

    fn resync_offsets(&mut self, ops: &[StoreOp], at_op: usize, failure: StoreFailure) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        for op in ops.iter().take(at_op) {
            match op {
                StoreOp::AppendData {
                    period,
                    expect_len,
                    bytes,
                } if *period == open.period => {
                    open.media_len =
                        expect_len.saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
                }
                StoreOp::AppendPlaylist {
                    period,
                    expect_len,
                    text,
                } if *period == open.period => {
                    open.playlist_len =
                        expect_len.saturating_add(u64::try_from(text.len()).unwrap_or(u64::MAX));
                }
                _ => {}
            }
        }
        if let StoreFailure::LengthMismatch { actual } = failure {
            match ops.get(at_op) {
                Some(StoreOp::AppendData { period, .. }) if *period == open.period => {
                    open.media_len = actual;
                }
                Some(StoreOp::AppendPlaylist { period, .. }) if *period == open.period => {
                    open.playlist_len = actual;
                }
                _ => {}
            }
        }
    }
}

/// The first id not already `used`: `base`, then `base-2`, `base-3`, ... so a
/// repeated programme in one playlist always gets a distinct id.
fn unique_daterange_id(base: &str, used: &[String]) -> String {
    if !used.iter().any(|id| id == base) {
        return base.to_owned();
    }
    let mut suffix = FIRST_DATERANGE_ID_SUFFIX;
    loop {
        let candidate = format!("{base}-{suffix}");
        if !used.contains(&candidate) {
            return candidate;
        }
        suffix = suffix.saturating_add(1);
    }
}

/// `duration` rounded up to whole seconds, saturating at `u32::MAX`.
fn ceil_secs(duration: Duration) -> u32 {
    let secs = duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() > 0));
    u32::try_from(secs).unwrap_or(u32::MAX)
}

impl std::fmt::Debug for ArchiveWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveWriter")
            .field("container", &self.config.container)
            .field("next_period", &self.next_period)
            .field("open", &self.open.as_ref().map(|o| o.period))
            .field("closed", &self.closed.len())
            .field("pending", &self.pending.is_some())
            .field("stopped", &self.stopped)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RetentionLimits;
    use crate::layout::ArchiveFile;

    fn config() -> ArchiveConfig {
        ArchiveConfig::new(
            ArchiveContainer::Ts,
            Duration::from_secs(2),
            RetentionLimits::new().with_periods(std::num::NonZeroUsize::new(10).expect("nonzero")),
        )
        .with_period_duration(Some(Duration::from_secs(3600)))
    }

    fn entry(seq: u32, len: usize, discontinuous: bool) -> SegmentEntry {
        SegmentEntry::new(
            Bytes::from(vec![0xAB; len]),
            SegmentSeq::new(seq),
            Duration::from_secs(2),
            Timestamp::from_nanos(u64::from(seq) * 2_000_000_000),
            transmux::SegmentMeta { discontinuous },
        )
    }

    fn open_one(writer: &mut ArchiveWriter) {
        let step = writer
            .on_item(
                Timestamp::ZERO,
                None,
                &SegmentCursorItem::Segment(entry(0, 100, false)),
            )
            .expect("first");
        let Step::Execute(plan) = step else {
            panic!("expected a plan");
        };
        writer
            .complete(plan.id, PlanOutcome::Done)
            .expect("complete");
    }

    #[test]
    fn playlist_full_counts_the_discontinuity_line() {
        let mut writer = ArchiveWriter::new(config()).expect("writer");
        open_one(&mut writer);
        let open = writer.open.as_ref().expect("open");
        let period = open.period;
        let media_len = open.media_len;
        let (_, record, _, _) = writer
            .record(
                period,
                media_len,
                0,
                1,
                2_000_000_000,
                &entry(1, 100, true),
                None,
                true,
                &open.daterange_ids,
                false,
            )
            .expect("record");
        let endlist = writer.endlist_text().expect("endlist").len() as u64;
        let boundary = u64::try_from(broadcast_hls::limits::MAX_PLAYLIST_BYTES).expect("limit")
            - endlist
            - u64::try_from(record.len()).expect("len")
            + 1;
        writer.open.as_mut().expect("open").playlist_len = boundary;
        let step = writer
            .on_item(
                Timestamp::ZERO,
                None,
                &SegmentCursorItem::Segment(entry(1, 100, true)),
            )
            .expect("next");
        let Step::Execute(plan) = step else {
            panic!("expected a plan");
        };
        assert!(
            matches!(plan.ops[0], StoreOp::AppendPlaylist { .. }),
            "a record that would exceed MAX_PLAYLIST_BYTES must roll first, got {:?}",
            plan.ops.iter().map(StoreOp::kind).collect::<Vec<_>>()
        );
        writer
            .complete(plan.id, PlanOutcome::Done)
            .expect("complete");
    }

    const SERVICE: u16 = 1537;
    const EVENT: u16 = 30807;

    fn baseline() -> dvb_si::epg::PresentEventChange {
        use dvb_si::descriptors::DescriptorLoop;
        use dvb_si::tables::eit::{EitEvent, EitKind, EitSection};
        use dvb_si::tables::{RunningStatus, SectionVersion};
        let title = b"Title";
        let mut descriptors = vec![0x4D, 3 + 1 + 5 + 1];
        descriptors.extend_from_slice(b"eng");
        descriptors.push(5);
        descriptors.extend_from_slice(title);
        descriptors.push(0);
        let mut event = EitEvent::new(
            EVENT,
            [0; 5],
            [0; 3],
            RunningStatus::Running,
            false,
            DescriptorLoop::new(&descriptors),
        );
        event.set_start_time(wall(0)).expect("start");
        event
            .set_duration(Duration::from_secs(3600))
            .expect("duration");
        let section = EitSection::new(
            EitKind::PresentFollowingActual,
            0x4E,
            SERVICE,
            SectionVersion::new(0, true, 0, 0),
            1,
            1,
            0,
            0x4E,
            vec![event],
        );
        dvb_si::epg::PresentEventTracker::new(SERVICE)
            .on_section(&section)
            .expect("a present event")
    }

    fn wall(secs: i64) -> jiff::Timestamp {
        jiff::Timestamp::from_second(1_716_055_200 + secs).expect("timestamp")
    }

    fn run(writer: &mut ArchiveWriter, entry: SegmentEntry, secs: i64) -> Plan {
        let step = writer
            .on_item(
                Timestamp::ZERO,
                Some(wall(secs)),
                &SegmentCursorItem::Segment(entry),
            )
            .expect("item");
        let Step::Execute(plan) = step else {
            panic!("expected a plan");
        };
        writer
            .complete(plan.id, PlanOutcome::Done)
            .expect("complete");
        plan
    }

    fn programmed(config: ArchiveConfig) -> ArchiveWriter {
        let mut writer =
            ArchiveWriter::new(config.with_programme_service(SERVICE)).expect("writer");
        if writer.is_fmp4() {
            writer
                .on_inits(&[InitGeneration::new(
                    SegmentSeq::new(0),
                    Bytes::from_static(b"INIT"),
                )])
                .expect("inits");
        }
        writer.on_programme(baseline()).expect("baseline");
        let first = run(&mut writer, entry(0, 100, false), 0);
        assert_eq!(
            created_playlist(&first)
                .matches("#EXT-X-DATERANGE:")
                .count(),
            1
        );
        writer
    }

    fn created_playlist(plan: &Plan) -> String {
        plan.ops
            .iter()
            .find_map(|op| match op {
                StoreOp::CreatePlaylist { text, .. } => Some(text.clone()),
                _ => None,
            })
            .expect("a new period")
    }

    fn assert_rolled_with_programme(
        writer: &mut ArchiveWriter,
        plan: &Plan,
        reason: RollReason,
        next_seq: u32,
    ) {
        let text = created_playlist(plan);
        assert_eq!(text.matches("#EXT-X-DATERANGE:").count(), 1, "{text}");
        assert_eq!(
            text.matches(&format!("X-DVB-EVENT-ID={EVENT}")).count(),
            1,
            "{text}"
        );
        assert_eq!(
            text.matches(&format!("ID=\"programme-{SERVICE}-{EVENT}\""))
                .count(),
            1,
            "{text}"
        );
        let opened: Vec<RollReason> = std::iter::from_fn(|| writer.poll_event())
            .filter_map(|event| match event {
                WriterEvent::PeriodOpened { reason, .. } => Some(reason),
                _ => None,
            })
            .collect();
        assert_eq!(opened.last().copied(), Some(reason));
        let next = run(writer, entry(next_seq, 10, false), 30);
        assert!(
            !next
                .ops
                .iter()
                .any(|op| matches!(op, StoreOp::CreatePlaylist { .. })),
            "the follow-up record must append to the same period"
        );
        let tail = playlist_text_of(&next);
        assert!(!tail.contains("#EXT-X-DATERANGE:"), "{tail}");
    }

    fn playlist_text_of(plan: &Plan) -> String {
        plan.ops
            .iter()
            .filter_map(|op| match op {
                StoreOp::AppendPlaylist { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_sequence_gap_roll_repeats_the_programme() {
        let mut writer = programmed(config());
        let plan = run(&mut writer, entry(5, 100, false), 10);
        assert_eq!(
            created_playlist(&plan)
                .matches("#EXT-X-DISCONTINUITY\n")
                .count(),
            1,
            "the first segment after lost ring entries is discontinuous"
        );
        assert_rolled_with_programme(&mut writer, &plan, RollReason::SequenceGap, 6);
    }

    #[test]
    fn a_target_duration_roll_repeats_the_programme() {
        let mut writer = programmed(config());
        let mut long = entry(1, 100, false);
        long.duration = Duration::from_secs(9);
        let plan = run(&mut writer, long, 2);
        assert_rolled_with_programme(&mut writer, &plan, RollReason::TargetDuration, 2);
    }

    #[test]
    fn a_byte_budget_roll_repeats_the_programme() {
        let config = ArchiveConfig::new(
            ArchiveContainer::Ts,
            Duration::from_secs(2),
            RetentionLimits::new()
                .with_bytes(std::num::NonZeroU64::new(1650).expect("nonzero"))
                .with_periods(std::num::NonZeroUsize::new(10).expect("nonzero")),
        );
        let mut writer = programmed(config);
        let plan = run(&mut writer, entry(1, 100, false), 2);
        assert_rolled_with_programme(&mut writer, &plan, RollReason::ByteBudget, 2);
    }

    #[test]
    fn a_playlist_full_roll_repeats_the_programme() {
        let mut writer = programmed(config());
        writer.open.as_mut().expect("open").playlist_len =
            u64::try_from(broadcast_hls::limits::MAX_PLAYLIST_BYTES).expect("limit");
        let plan = run(&mut writer, entry(1, 100, false), 2);
        assert_rolled_with_programme(&mut writer, &plan, RollReason::PlaylistFull, 2);
    }

    #[test]
    fn an_init_change_roll_repeats_the_programme() {
        let config = ArchiveConfig::new(
            ArchiveContainer::Fmp4,
            Duration::from_secs(2),
            RetentionLimits::new().with_periods(std::num::NonZeroUsize::new(10).expect("nonzero")),
        )
        .with_period_duration(Some(Duration::from_secs(3600)));
        let mut writer = programmed(config);
        writer
            .on_inits(&[InitGeneration::new(
                SegmentSeq::new(1),
                Bytes::from_static(b"INIT2"),
            )])
            .expect("inits");
        let plan = run(&mut writer, entry(1, 100, false), 2);
        assert_rolled_with_programme(&mut writer, &plan, RollReason::InitChanged, 2);
    }

    #[test]
    fn repeated_programmes_get_distinct_ids() {
        let mut used: Vec<String> = Vec::new();
        let first = unique_daterange_id("programme-1-7", &used);
        assert_eq!(first, "programme-1-7");
        used.push(first);
        let second = unique_daterange_id("programme-1-7", &used);
        assert_eq!(second, "programme-1-7-2");
        used.push(second);
        let third = unique_daterange_id("programme-1-7", &used);
        assert_eq!(third, "programme-1-7-3");
        assert!(!used.contains(&third));
        let _ = ArchiveFile::Media(PeriodNum::new(0));
    }
}
