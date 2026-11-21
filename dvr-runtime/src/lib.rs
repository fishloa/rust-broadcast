//! Sans-IO DVR archive for a live origin.
//!
//! `dvr-runtime` owns one on-disk archive format and the two engines around
//! it: an [`ArchiveWriter`] that turns the host's finished segments into a
//! plan of storage operations (committing its own state only when the host
//! reports the plan executed), and a [`read`] half that plans catch-up
//! playlists from the same files. Neither half does I/O or reads a clock; the
//! optional `fs` feature provides the blocking `std::fs` adapter.
//!
//! # The format
//!
//! One directory per archive, one *period* per media file:
//!
//! ```text
//! <archive_root>/<archive_name>/
//!   pN.m4s | pN.ts   period media file (N = period number, decimal, no padding)
//!   pN.m3u8          period playlist (the index)
//!   .pN.m3u8.tmp     transient, during playlist creation only
//! ```
//!
//! The playlist is a standard HLS Media Playlist (RFC 8216 / RFC 8216bis):
//! `PLAYLIST-TYPE:EVENT`, one segment record appended at a time, closed with
//! `#EXT-X-ENDLIST`. Every segment is an `EXT-X-BYTERANGE` into the period's
//! single media file, so a raw `pN.m4s`/`pN.ts` is playable on its own (the
//! fMP4 init is the file's head). Two vendor tags carry what a standard
//! playlist cannot:
//!
//! * `#EXT-X-RUSTBROADCAST-ARCHIVE:VERSION=1,CONTAINER=FMP4|TS,PERIOD=<N>`
//!   (playlist level) marks the file as an archive period and names its
//!   format version, container and period number.
//! * `#EXT-X-RUSTBROADCAST-TIMING:SEQ=<s>,DTS=<t>,DUR=<d>,TIMESCALE=<ts>`
//!   (segment scoped) carries the exact Trunk timeline values, because
//!   `#EXTINF` is a rounded decimal and catch-up needs the sequence, the
//!   start and the duration it was written with.
//!
//! Both are built and read by `broadcast-hls` ([`broadcast_hls::CustomTag`]),
//! as is every other line of the playlist; this crate never assembles tag
//! text by hand. RFC 8216bis §4.1 makes every `#EXT` line a tag and §6.3.1
//! requires clients to ignore unknown tags, which is why a standard player
//! skips both.
//!
//! Programme identity is an `EXT-X-DATERANGE` (RFC 8216bis §4.4.5.1) with
//! `CLASS="io.github.fishloa.rust-broadcast.programme"` and the `X-DVB-*`
//! client attributes, so one recording period can be found by programme.
//!
//! # Append-only and crash-safe
//!
//! There is no seek-and-overwrite anywhere. A period's header is written
//! once (temp file, `sync_all`, rename, directory `sync_all`); each segment
//! record is one append of the lines `broadcast-hls` renders; the close is
//! one appended `#EXT-X-ENDLIST`. A crash can therefore only leave a torn
//! line at the end of the playlist, which [`read::parse_period_playlist`]
//! cuts away and reports as `torn_bytes`/`torn_records`; every complete
//! record before it is read intact.
//!
//! # Features
//!
//! | Feature | Adds |
//! |---|---|
//! | `fs` | [`fs::FsRecorder`] (executes the writer's plans on `std::fs`) and [`fs::FsArchive`] (the read half plus its per-archive parsed-playlist cache). Both blocking. |
//!
//! The core is `std` (it reuses `media_plane`'s `std`-gated segment types) but
//! performs no I/O, starts no thread, reads no clock and has no async, no
//! tokio, no tracing, no metrics and no serde: the host supplies every
//! instant and executes every operation.
//!
//! # Example
//!
//! The crate has no I/O of its own, so the runnable examples go through the
//! `fs` adapter:
//!
//! ```text
//! cargo run -p dvr-runtime --example record_ts_file -- --help
//! cargo run -p dvr-runtime --example list_programmes <archive-dir>
//! ```
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

use std::num::{NonZeroU64, NonZeroUsize};
use std::time::Duration;

use media_plane::ids::SegmentSeq;

pub mod format;
pub mod layout;
pub mod read;
pub mod recover;
pub mod writer;

#[cfg(feature = "fs")]
#[cfg_attr(docsrs, doc(cfg(feature = "fs")))]
pub mod fs;

pub use writer::{
    ArchiveWriter, InitGeneration, Plan, PlanId, PlanOutcome, RollReason, Step, StoreFailure,
    StoreOp, WriterCounters, WriterEvent,
};

/// The longest accepted [`ArchiveName`], in bytes — one filesystem path
/// component.
pub const MAX_ARCHIVE_NAME_LEN: usize = 255;

/// A validated archive directory name: a single, safe path component.
///
/// The rules are the ones the archive always applied at the point the name
/// becomes a path: non-empty, not `.` or `..`, no `..` component, no path
/// separator (either platform's), no NUL byte, and at most
/// [`MAX_ARCHIVE_NAME_LEN`] bytes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct ArchiveName(String);

impl ArchiveName {
    /// Validate `name`.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] naming which rule it broke.
    pub fn new(name: &str) -> Result<Self, ConfigError> {
        if name.is_empty() {
            return Err(ConfigError::EmptyName);
        }
        if name == "." || name == ".." || name.contains("..") {
            return Err(ConfigError::TraversalName);
        }
        if name.len() > MAX_ARCHIVE_NAME_LEN {
            return Err(ConfigError::NameTooLong {
                len: name.len(),
                max: MAX_ARCHIVE_NAME_LEN,
            });
        }
        const FORBIDDEN: [char; 3] = ['/', '\\', '\u{0}'];
        if name
            .chars()
            .any(|c| FORBIDDEN.contains(&c) || c == std::path::MAIN_SEPARATOR)
        {
            return Err(ConfigError::SeparatorInName);
        }
        Ok(Self(name.to_owned()))
    }

    /// The name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for ArchiveName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ArchiveName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The container a period's media file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ArchiveContainer {
    /// Fragmented MP4: the period's one init first, then its segments.
    Fmp4,
    /// MPEG-2 TS: the segments' own packets, concatenated.
    Ts,
}

impl ArchiveContainer {
    /// The marker tag's `CONTAINER` token.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Fmp4 => "FMP4",
            Self::Ts => "TS",
        }
    }

    /// The period media file's extension, including the dot.
    #[must_use]
    pub fn ext(&self) -> &'static str {
        match self {
            Self::Fmp4 => ".m4s",
            Self::Ts => ".ts",
        }
    }
}

broadcast_common::impl_spec_display!(ArchiveContainer);

/// A period's own number: the `N` in `pN.m4s`/`pN.m3u8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[non_exhaustive]
pub struct PeriodNum(u32);

impl PeriodNum {
    /// The period numbered `value`.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// The number itself.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }

    /// The next period's number, or `None` when the `u32` range is exhausted.
    #[must_use]
    pub const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }
}

impl std::fmt::Display for PeriodNum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

/// Whole-period retention bounds. `None` disables that axis; at least one
/// must be set, or the archive would grow without bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct RetentionLimits {
    /// Keep at most this many closed periods.
    pub periods: Option<NonZeroUsize>,
    /// Keep at most this many total bytes across the media files.
    pub bytes: Option<NonZeroU64>,
}

impl RetentionLimits {
    /// Bounds with no axis set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Keep at most `periods` closed periods.
    #[must_use]
    pub fn with_periods(mut self, periods: NonZeroUsize) -> Self {
        self.periods = Some(periods);
        self
    }

    /// Keep at most `bytes` bytes across the media files.
    #[must_use]
    pub fn with_bytes(mut self, bytes: NonZeroU64) -> Self {
        self.bytes = Some(bytes);
        self
    }

    /// Whether either axis is set.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.periods.is_some() || self.bytes.is_some()
    }

    /// The most bytes one period's media file may hold: `bytes / (periods +
    /// 1)`, at least one. The `+ 1` reserves room for the open period, so
    /// `periods` closed periods plus the one being written fit inside the
    /// cap; with `periods` unset this is the whole cap.
    #[must_use]
    pub fn per_period_byte_budget(&self) -> Option<NonZeroU64> {
        let bytes = self.bytes?;
        let periods = self.periods.map_or(0, NonZeroUsize::get);
        let divisor = u64::try_from(periods).unwrap_or(u64::MAX).saturating_add(1);
        let budget = (bytes.get() / divisor.max(1)).max(1);
        NonZeroU64::new(budget)
    }
}

/// Per-archive configuration: what the writer needs to know about the route's
/// container, segmenter and retention policy.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ArchiveConfig {
    /// The period media file's container.
    pub container: ArchiveContainer,
    /// The route segmenter's target duration, which becomes the playlist's
    /// (rounded-up) `EXT-X-TARGETDURATION`.
    pub target_duration: Duration,
    /// Roll the period once it has been open this long; `None` never rolls on
    /// time.
    pub period_duration: Option<Duration>,
    /// Whole-period retention.
    pub retention: RetentionLimits,
    /// What the pinning cursor does when the live ring wants to evict a
    /// segment it has not consumed yet.
    pub overrun: media_plane::trunk::ArchiveOverrun,
    /// Added to a segment's Trunk sequence number to get its archive sequence
    /// number.
    pub seq_offset: u64,
    /// The service whose EIT present/following event marks programme
    /// boundaries. When set, the host must supply wall time with every
    /// segment (an `EXT-X-DATERANGE` requires a
    /// `EXT-X-PROGRAM-DATE-TIME`).
    pub programme_service: Option<u16>,
}

impl ArchiveConfig {
    /// A configuration with `None` period duration and no programme tracking.
    #[must_use]
    pub fn new(
        container: ArchiveContainer,
        target_duration: Duration,
        retention: RetentionLimits,
    ) -> Self {
        Self {
            container,
            target_duration,
            period_duration: None,
            retention,
            overrun: media_plane::trunk::ArchiveOverrun::Gap,
            seq_offset: 0,
            programme_service: None,
        }
    }

    /// Set [`Self::period_duration`].
    #[must_use]
    pub fn with_period_duration(mut self, duration: Option<Duration>) -> Self {
        self.period_duration = duration;
        self
    }

    /// Set [`Self::overrun`].
    #[must_use]
    pub fn with_overrun(mut self, overrun: media_plane::trunk::ArchiveOverrun) -> Self {
        self.overrun = overrun;
        self
    }

    /// Set [`Self::seq_offset`].
    #[must_use]
    pub fn with_seq_offset(mut self, offset: u64) -> Self {
        self.seq_offset = offset;
        self
    }

    /// Opt into programme alignment for `service_id`.
    #[must_use]
    pub fn with_programme_service(mut self, service_id: u16) -> Self {
        self.programme_service = Some(service_id);
        self
    }

    /// Validate the config.
    ///
    /// # Errors
    ///
    /// [`ConfigError::NoRetention`] with neither retention axis,
    /// [`ConfigError::UnboundedPeriod`] when nothing can ever roll the
    /// period or bound it, [`ConfigError::ZeroTargetDuration`] for a zero
    /// target duration.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !self.retention.is_active() {
            return Err(ConfigError::NoRetention);
        }
        if self.target_duration.is_zero() {
            return Err(ConfigError::ZeroTargetDuration);
        }
        if self.period_duration.is_none()
            && self.programme_service.is_none()
            && self.retention.bytes.is_none()
        {
            return Err(ConfigError::UnboundedPeriod);
        }
        Ok(())
    }

    /// The configured container's media file extension.
    #[must_use]
    pub fn media_ext(&self) -> &'static str {
        self.container.ext()
    }
}

/// A configuration value the chosen policy cannot accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// The archive name is empty.
    #[error("archive name must not be empty")]
    EmptyName,
    /// The archive name is `.`, `..` or contains a `..` component.
    #[error("archive name is a path-traversal component")]
    TraversalName,
    /// The archive name is longer than [`MAX_ARCHIVE_NAME_LEN`].
    #[error("archive name is {len} bytes, over the {max} limit")]
    NameTooLong {
        /// The name's byte length.
        len: usize,
        /// The limit.
        max: usize,
    },
    /// The archive name contains a path separator or a NUL byte.
    #[error("archive name contains a path separator or NUL byte")]
    SeparatorInName,
    /// Neither retention axis is set.
    #[error("at least one retention limit must be set (unbounded growth would fill the disk)")]
    NoRetention,
    /// Nothing can ever roll or bound the period.
    #[error(
        "period_duration is unset, no programme boundary can roll, and retention_bytes is unset (the open period would grow without bound)"
    )]
    UnboundedPeriod,
    /// The target duration is zero.
    #[error("target_duration must be greater than zero")]
    ZeroTargetDuration,
}

/// A write or seed failure the writer itself detected.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ArchiveError {
    /// The configuration is invalid.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// A plan is still outstanding; finish it with
    /// [`ArchiveWriter::complete`] first.
    #[error("a plan is still outstanding")]
    PlanPending,
    /// The plan id was not the outstanding plan's.
    #[error("unknown plan {id}")]
    UnknownPlan {
        /// The id the host reported.
        id: PlanId,
    },
    /// A segment's archive sequence number does not fit the `u32` range.
    #[error("segment {trunk_seq} + offset {offset} overflows the archive's u32 sequence range")]
    SeqOverflow {
        /// The Trunk's own sequence number.
        trunk_seq: SegmentSeq,
        /// The configured offset.
        offset: u64,
    },
    /// A segment's duration does not fit `u64` ticks at the playlist's
    /// timescale.
    #[error("segment {seq}'s duration does not fit u64 ticks")]
    DurationOverflow {
        /// The Trunk's sequence number.
        seq: SegmentSeq,
    },
    /// The archive's period numbers are exhausted (`u32::MAX` is taken).
    #[error("archive period numbers are exhausted")]
    PeriodNumbersExhausted,
    /// A segment with no bytes cannot be referenced by an `EXT-X-BYTERANGE`.
    #[error("segment {seq} carries no bytes")]
    EmptySegment {
        /// The Trunk's sequence number.
        seq: SegmentSeq,
    },
    /// The configuration tracks programmes but the caller supplied no wall
    /// time for this segment.
    #[error("programme tracking requires a wall-clock time for every segment")]
    WallClockRequired,
    /// `broadcast-hls` rejected a rendered line.
    #[error(transparent)]
    Render(#[from] broadcast_hls::Error),
}
