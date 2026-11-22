//! The read half: parse one period's playlist into typed segments and plan a
//! catch-up playlist over the archive plus the live Trunk's tail.
//!
//! [`parse_period_playlist`] is the only reader of the format. It cuts a torn
//! tail line (a crash can leave one), parses the rest through
//! [`broadcast_hls::MediaPlaylist::parse_with_options`] with the timing tag
//! declared segment-scoped, and validates every archive invariant: the marker
//! tag matches the file, every segment's URI is the period's own media file,
//! its byte range is explicit, its timing tag is exactly one and its sequence
//! is continuous, and no segment carries a gap, a key or a part. A failure
//! makes that one period unreadable; it never affects another period.

use std::num::NonZeroU64;
use std::time::Duration;

use broadcast_common::Timestamp;
use broadcast_common::range::ByteSpan;
use broadcast_hls::{MediaPlaylist, ParseDiagnosticKind, ParseOptions};
use media_plane::ids::SegmentSeq;

use crate::format;
use crate::{ArchiveContainer, PeriodNum};

/// The number of a byte range, or `None` for a zero length (which
/// [`ByteSpan`] cannot hold).
fn span(offset: u64, length: u64) -> Option<ByteSpan> {
    NonZeroU64::new(length).map(|len| ByteSpan::new(offset, len))
}

/// One archived segment, with the exact values it was written with and the
/// byte ranges that locate it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ArchivedSegment {
    /// The archive sequence number (`EXT-X-MEDIA-SEQUENCE + index`).
    pub seq: SegmentSeq,
    /// The segment's start on the Trunk's timeline.
    pub start: Timestamp,
    /// The segment's exact duration.
    pub duration: Duration,
    /// Whether `#EXT-X-DISCONTINUITY` precedes this segment.
    pub discontinuous: bool,
    /// The period this segment's bytes live in.
    pub period: PeriodNum,
    /// The segment's bytes within the period's media file.
    pub span: ByteSpan,
    /// The period's init bytes (fMP4 only), located within the same media
    /// file.
    pub init: Option<ByteSpan>,
    /// The wall-clock time the segment was recorded with, if the host
    /// supplied one.
    pub wall: Option<jiff::Timestamp>,
}

/// One programme boundary an archive playlist records.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ArchivedProgramme {
    /// The period holding the boundary.
    pub period: PeriodNum,
    /// The sequence number of the segment the `EXT-X-DATERANGE` is attached
    /// to (the observed boundary).
    pub first_seq: SegmentSeq,
    /// The DVB service id.
    pub service_id: u16,
    /// The DVB event id.
    pub event_id: u16,
    /// The programme title, when the event carried one.
    pub title: Option<String>,
    /// The announced start time, when decodable.
    pub scheduled_start: Option<jiff::Timestamp>,
    /// The announced duration, when decodable.
    pub planned: Option<Duration>,
}

/// One period's parsed playlist.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PeriodPlaylist {
    /// Every complete segment record, in playlist order.
    pub segments: Vec<ArchivedSegment>,
    /// Every programme boundary the playlist records.
    pub programmes: Vec<ArchivedProgramme>,
    /// Whether the playlist ends with `#EXT-X-ENDLIST`.
    pub closed: bool,
    /// Bytes discarded from the tail: trailing NULs plus an incomplete final
    /// line.
    pub torn_bytes: usize,
    /// Incomplete final records discarded (a record whose URI line never
    /// landed): zero or one.
    pub torn_records: usize,
}

/// A tag an archive period never carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnsupportedTag {
    /// `#EXT-X-GAP` — the archive never records a gap segment.
    Gap,
    /// `#EXT-X-KEY` — the archive is unencrypted.
    Key,
    /// `#EXT-X-PART` / an open LL-HLS segment.
    Parts,
}

impl UnsupportedTag {
    /// The spec token of the tag.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Gap => "EXT-X-GAP",
            Self::Key => "EXT-X-KEY",
            Self::Parts => "EXT-X-PART",
        }
    }
}

broadcast_common::impl_spec_display!(UnsupportedTag);

/// Why one period's playlist is not a readable archive period.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PlaylistError {
    /// `broadcast-hls` rejected the playlist text.
    #[error(transparent)]
    Hls(#[from] broadcast_hls::Error),
    /// The archive marker tag is missing (or the file is not UTF-8 text).
    #[error("not an archive period playlist")]
    NotAnArchive,
    /// The marker names a format version this reader does not know.
    #[error("unsupported archive format version {found}")]
    UnsupportedVersion {
        /// The version the marker names.
        found: u64,
    },
    /// The marker's container is not the one being read.
    #[error("the archive marker names a different container")]
    ContainerMismatch,
    /// The marker's period number is not the file's.
    #[error("archive marker names period {tag}, but the file is period {file}")]
    PeriodMismatch {
        /// The marker's period number.
        tag: u32,
        /// The file's period number.
        file: u32,
    },
    /// A segment has no timing tag, or more than one.
    #[error("segment {index} has no exactly-one timing tag")]
    MissingTiming {
        /// The segment's playlist index.
        index: usize,
    },
    /// A segment's timing `SEQ` does not match its playlist position.
    #[error("segment {index}'s timing sequence number does not match its playlist position")]
    SequenceMismatch {
        /// The segment's playlist index.
        index: usize,
    },
    /// A segment's timing tag is present but malformed (a non-zero
    /// `TIMESCALE` is required).
    #[error("segment {index}'s timing tag is malformed")]
    MalformedTiming {
        /// The segment's playlist index.
        index: usize,
    },
    /// A segment's URI is not the period's own media file.
    #[error("segment {index}'s URI is not this period's media file")]
    ForeignUri {
        /// The segment's playlist index.
        index: usize,
    },
    /// A segment has no explicit byte-range offset (or a zero length).
    #[error("segment {index} has no explicit byte range")]
    MissingOffset {
        /// The segment's playlist index.
        index: usize,
    },
    /// A segment's byte range is not after the previous one.
    #[error("segment {index}'s byte range overlaps or precedes the previous segment")]
    Overlap {
        /// The segment's playlist index.
        index: usize,
    },
    /// An fMP4 segment carries no `EXT-X-MAP` byte range.
    #[error("segment {index} has no EXT-X-MAP byte range")]
    MissingMap {
        /// The segment's playlist index.
        index: usize,
    },
    /// The playlist carries a tag the archive never writes.
    #[error("segment {index} carries an unsupported tag: {tag}")]
    Unsupported {
        /// The segment's playlist index.
        index: usize,
        /// The tag.
        tag: UnsupportedTag,
    },
    /// A media segment follows `#EXT-X-ENDLIST` (RFC 8216 §4.3.3.4 closes the
    /// playlist).
    #[error("a media segment follows EXT-X-ENDLIST")]
    RecordAfterEndlist,
    /// `#EXT-X-ENDLIST` appears more than once (RFC 8216 §4.3.3.4).
    #[error("EXT-X-ENDLIST appears more than once")]
    DuplicateEndlist,
    /// The playlist file disappeared between listing and reading (a racing
    /// eviction), distinct from corruption.
    #[error("period playlist not found")]
    NotFound,
    /// Reading the playlist failed with an I/O error.
    #[error("io: {kind}")]
    Io {
        /// The error kind.
        kind: std::io::ErrorKind,
    },
}

/// Why reading an archived segment's bytes failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReadError {
    /// The period's media file no longer exists (retention evicted it).
    #[error("archived period {period} was evicted")]
    Gone {
        /// The period.
        period: PeriodNum,
    },
    /// The recorded byte range is not backed by the media file.
    #[error("byte range {span} is beyond the {file_len}-byte period file")]
    RangeBeyondFile {
        /// The recorded range.
        span: ByteSpan,
        /// The media file's real length.
        file_len: u64,
    },
    /// The period's playlist is unreadable.
    #[error(transparent)]
    Playlist(#[from] PlaylistError),
    /// An fMP4 recovery scan failed.
    #[error(transparent)]
    Recover(#[from] crate::recover::RecoverError),
    /// An I/O error, by kind.
    #[error("io: {kind}")]
    Io {
        /// The error kind.
        kind: std::io::ErrorKind,
    },
}

impl ArchivedSegment {
    /// A segment with a continuous timeline, no init and no wall time.
    #[must_use]
    pub fn new(
        seq: SegmentSeq,
        start: Timestamp,
        duration: Duration,
        period: PeriodNum,
        span: ByteSpan,
    ) -> Self {
        Self {
            seq,
            start,
            duration,
            discontinuous: false,
            period,
            span,
            init: None,
            wall: None,
        }
    }

    /// Set whether a discontinuity precedes the segment.
    #[must_use]
    pub fn with_discontinuous(mut self, discontinuous: bool) -> Self {
        self.discontinuous = discontinuous;
        self
    }

    /// Set the period's init span (fMP4).
    #[must_use]
    pub fn with_init(mut self, init: Option<ByteSpan>) -> Self {
        self.init = init;
        self
    }

    /// Set the wall-clock time the segment was recorded with.
    #[must_use]
    pub fn with_wall(mut self, wall: Option<jiff::Timestamp>) -> Self {
        self.wall = wall;
        self
    }
}

/// One archived segment, or a live Tail segment, in a catch-up playlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct CatchupSegment<G> {
    /// The segment's sequence number.
    pub seq: SegmentSeq,
    /// Its start on the Trunk's timeline.
    pub start: Timestamp,
    /// Its exact duration.
    pub duration: Duration,
    /// Whether a discontinuity precedes it.
    pub discontinuous: bool,
    /// The init that decodes it.
    pub init: Option<InitRef<G>>,
}

impl<G> CatchupSegment<G> {
    /// A catch-up segment with a continuous timeline and no init.
    #[must_use]
    pub fn new(seq: SegmentSeq, start: Timestamp, duration: Duration) -> Self {
        Self {
            seq,
            start,
            duration,
            discontinuous: false,
            init: None,
        }
    }

    /// Set whether a discontinuity precedes the segment.
    #[must_use]
    pub fn with_discontinuous(mut self, discontinuous: bool) -> Self {
        self.discontinuous = discontinuous;
        self
    }

    /// Set the init reference.
    #[must_use]
    pub fn with_init(mut self, init: Option<InitRef<G>>) -> Self {
        self.init = init;
        self
    }
}

/// Where a catch-up segment's init comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InitRef<G> {
    /// The init at the head of archive period `N`, at `span` in its media
    /// file.
    Archived {
        /// The period.
        period: PeriodNum,
        /// The init's bytes within the period's media file.
        span: ByteSpan,
    },
    /// The live origin's init generation `G`.
    Live(G),
}

/// One still-live segment the host supplies for the merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct LiveSegment<G> {
    /// The segment's sequence number.
    pub seq: SegmentSeq,
    /// Its start on the Trunk's timeline.
    pub start: Timestamp,
    /// Its exact duration.
    pub duration: Duration,
    /// Whether a discontinuity precedes it.
    pub discontinuous: bool,
    /// The live init generation that decodes it.
    pub init: G,
}

impl<G> LiveSegment<G> {
    /// A live tail segment with a continuous timeline.
    #[must_use]
    pub fn new(seq: SegmentSeq, start: Timestamp, duration: Duration, init: G) -> Self {
        Self {
            seq,
            start,
            duration,
            discontinuous: false,
            init,
        }
    }

    /// Set whether a discontinuity precedes the segment.
    #[must_use]
    pub fn with_discontinuous(mut self, discontinuous: bool) -> Self {
        self.discontinuous = discontinuous;
        self
    }
}

/// The result of reading an archive directory.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ArchiveScan {
    /// Every readable segment, ascending by sequence number.
    pub segments: Vec<ArchivedSegment>,
    /// Periods that could not be read, with the reason.
    pub unreadable: Vec<(PeriodNum, PlaylistError)>,
    /// Sequence numbers more than one period claimed. Only the earliest
    /// period's record is in `segments`; each later claim is reported here.
    pub conflicts: Vec<SeqConflict>,
}

/// One sequence number claimed by two periods (a store that reported a
/// playlist creation failed after it had landed). The earlier period's
/// record is kept; the later one is dropped from the scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SeqConflict {
    /// The archive sequence number.
    pub seq: SegmentSeq,
    /// The period whose record was kept.
    pub kept: PeriodNum,
    /// The period whose record was dropped.
    pub dropped: PeriodNum,
}

impl SeqConflict {
    /// A conflict over `seq` between the `kept` and `dropped` periods.
    #[must_use]
    pub fn new(seq: SegmentSeq, kept: PeriodNum, dropped: PeriodNum) -> Self {
        Self { seq, kept, dropped }
    }
}

fn endlist_line() -> Result<String, broadcast_hls::Error> {
    let rendered = MediaPlaylist {
        endlist: true,
        ..MediaPlaylist::default()
    }
    .render_endlist()?;
    Ok(rendered.trim_end_matches('\n').to_owned())
}

fn endlist_violation(text: &str, endlist: &str) -> Option<PlaylistError> {
    let mut ended = false;
    for line in text.lines() {
        if line == endlist {
            if ended {
                return Some(PlaylistError::DuplicateEndlist);
            }
            ended = true;
        } else if ended && !line.trim().is_empty() && !line.starts_with('#') {
            return Some(PlaylistError::RecordAfterEndlist);
        }
    }
    None
}

/// Convert `ticks` at `timescale` to whole nanoseconds.
fn ticks_to_nanos(ticks: u64, timescale: u64) -> Option<u64> {
    if timescale == 0 {
        return None;
    }
    let nanos = u128::from(ticks) * u128::from(format::TIMING_TIMESCALE) / u128::from(timescale);
    u64::try_from(nanos).ok()
}

/// Check that `span` is backed by a file of `file_len` bytes.
///
/// # Errors
///
/// [`ReadError::RangeBeyondFile`] when the range's end overflows or lies past
/// the file.
pub fn checked_range(span: ByteSpan, file_len: u64) -> Result<ByteSpan, ReadError> {
    let end = span.offset.checked_add(span.len.get());
    if end.is_none_or(|end| end > file_len) {
        return Err(ReadError::RangeBeyondFile { span, file_len });
    }
    Ok(span)
}

/// Parse and validate one period's playlist.
///
/// # Errors
///
/// [`PlaylistError`] naming the first invariant the file breaks. Any error
/// makes this one period unreadable, never another period.
pub fn parse_period_playlist(
    period: PeriodNum,
    container: ArchiveContainer,
    bytes: &[u8],
) -> Result<PeriodPlaylist, PlaylistError> {
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == 0 {
        end -= 1;
    }
    let trimmed_nuls = bytes.len() - end;
    let body = &bytes[..end];
    let (complete, torn_line) = match body.iter().rposition(|&b| b == b'\n') {
        Some(last) => {
            let cut = last + 1;
            (&body[..cut], body.len() - cut)
        }
        None => (&body[..0], body.len()),
    };
    let torn_bytes = trimmed_nuls + torn_line;
    let text = std::str::from_utf8(complete).map_err(|_| PlaylistError::NotAnArchive)?;
    if let Some(violation) = endlist_violation(text, &endlist_line()?) {
        return Err(violation);
    }
    let options = ParseOptions::default()
        .with_segment_scoped_tag(broadcast_hls::CustomTagName::new(format::TIMING_TAG_NAME)?);
    let playlist = MediaPlaylist::parse_with_options(
        text,
        &broadcast_hls::limits::Limits::default(),
        &options,
    )?;

    let marker = playlist
        .extra_tags
        .iter()
        .find_map(|line| format::parse_marker(line))
        .ok_or(PlaylistError::NotAnArchive)?;
    if marker.version != format::ARCHIVE_FORMAT_VERSION {
        return Err(PlaylistError::UnsupportedVersion {
            found: marker.version,
        });
    }
    if marker.container != container {
        return Err(PlaylistError::ContainerMismatch);
    }
    if marker.period != period.value() {
        return Err(PlaylistError::PeriodMismatch {
            tag: marker.period,
            file: period.value(),
        });
    }

    let mut torn_records = usize::from(playlist.open_segment.is_some());
    if playlist
        .diagnostics
        .iter()
        .any(|d| d.kind == ParseDiagnosticKind::DanglingSegmentTags)
    {
        torn_records = 1;
    }
    let keys = playlist.segment_keys()?;
    if let Some(index) = keys.iter().position(Option::is_some) {
        return Err(PlaylistError::Unsupported {
            index,
            tag: UnsupportedTag::Key,
        });
    }

    let media_name = crate::layout::media_file_name(period, container);
    let mut segments = Vec::with_capacity(playlist.segments.len());
    let mut programmes = Vec::new();
    let mut previous_end = None;
    for (index, segment) in playlist.segments.iter().enumerate() {
        let position = playlist
            .msn
            .value()
            .checked_add(u64::try_from(index).unwrap_or(u64::MAX))
            .ok_or(PlaylistError::SequenceMismatch { index })?;
        let seq = u32::try_from(position).map_err(|_| PlaylistError::SequenceMismatch { index })?;
        if segment.uri != media_name {
            return Err(PlaylistError::ForeignUri { index });
        }
        if segment.gap {
            return Err(PlaylistError::Unsupported {
                index,
                tag: UnsupportedTag::Gap,
            });
        }
        if !segment.parts.is_empty() {
            return Err(PlaylistError::Unsupported {
                index,
                tag: UnsupportedTag::Parts,
            });
        }
        let byte_range = segment
            .byte_range
            .as_ref()
            .and_then(|range| span(range.offset?, range.length))
            .ok_or(PlaylistError::MissingOffset { index })?;
        if let Some(end) = previous_end
            && byte_range.offset < end
        {
            return Err(PlaylistError::Overlap { index });
        }
        previous_end = Some(byte_range.offset.saturating_add(byte_range.len.get()));

        let timings: Vec<format::Timing> = segment
            .pre_tags
            .iter()
            .filter_map(|tag| format::parse_timing(tag))
            .collect();
        if timings.len() != 1 {
            return Err(PlaylistError::MissingTiming { index });
        }
        let timing = timings[0];
        if timing.seq != position {
            return Err(PlaylistError::SequenceMismatch { index });
        }
        let nanos = ticks_to_nanos(timing.dts, timing.timescale)
            .ok_or(PlaylistError::MalformedTiming { index })?;
        let duration = ticks_to_nanos(timing.dur, timing.timescale)
            .map(Duration::from_nanos)
            .ok_or(PlaylistError::MalformedTiming { index })?;
        let init = if container == ArchiveContainer::Fmp4 {
            let map = segment
                .map
                .as_ref()
                .ok_or(PlaylistError::MissingMap { index })?;
            let range = map
                .byte_range
                .as_ref()
                .ok_or(PlaylistError::MissingMap { index })?;
            let offset = range.offset.ok_or(PlaylistError::MissingMap { index })?;
            Some(span(offset, range.length).ok_or(PlaylistError::MissingMap { index })?)
        } else {
            None
        };
        for tag in &segment.pre_tags {
            if let Some(programme) = format::parse_programme(tag) {
                programmes.push(ArchivedProgramme {
                    period,
                    first_seq: SegmentSeq::new(seq),
                    service_id: programme.service_id,
                    event_id: programme.event_id,
                    title: programme.title,
                    scheduled_start: programme.scheduled_start,
                    planned: programme.planned,
                });
            }
        }
        segments.push(ArchivedSegment {
            seq: SegmentSeq::new(seq),
            start: Timestamp::from_nanos(nanos),
            duration,
            discontinuous: segment.discontinuous,
            period,
            span: byte_range,
            init,
            wall: segment.program_date_time.map(|pdt| pdt.timestamp()),
        });
    }
    Ok(PeriodPlaylist {
        segments,
        programmes,
        closed: playlist.endlist,
        torn_bytes,
        torn_records,
    })
}

/// Merge the archive with the live Trunk's still-unarchived closed tail into
/// one ascending sequence. `live` is filtered to sequence numbers strictly
/// greater than the archive's highest, so a segment the live window still
/// holds and the archive already wrote never appears twice. Every segment
/// whose sequence number does not follow its predecessor's (lost ring
/// entries, an evicted period, the archive-to-live boundary) is marked
/// discontinuous.
pub fn merge<G: Copy>(
    archived: &[ArchivedSegment],
    live: &[LiveSegment<G>],
) -> Vec<CatchupSegment<G>> {
    let archive_max = archived.last().map(|s| s.seq);
    let mut combined: Vec<CatchupSegment<G>> = archived
        .iter()
        .map(|s| CatchupSegment {
            seq: s.seq,
            start: s.start,
            duration: s.duration,
            discontinuous: s.discontinuous,
            init: s.init.map(|span| InitRef::Archived {
                period: s.period,
                span,
            }),
        })
        .collect();
    combined.extend(live.iter().filter_map(|s| {
        let is_tail = archive_max.is_none_or(|max| s.seq > max);
        is_tail.then_some(CatchupSegment {
            seq: s.seq,
            start: s.start,
            duration: s.duration,
            discontinuous: s.discontinuous,
            init: Some(InitRef::Live(s.init)),
        })
    }));
    let mut previous: Option<SegmentSeq> = None;
    for segment in &mut combined {
        if previous.is_some_and(|prev| segment.seq != prev.saturating_next()) {
            segment.discontinuous = true;
        }
        previous = Some(segment.seq);
    }
    combined
}

/// Restrict an ascending sequence to the trailing window of `window` before
/// the last segment's start, measured by walking back over the segments'
/// durations (a segment's own start restarts after every reconnect, so it
/// cannot anchor the window). `None` or a zero window returns everything.
pub fn window<G: Copy>(
    segments: &[CatchupSegment<G>],
    window: Option<Duration>,
) -> Vec<CatchupSegment<G>> {
    let Some(window) = window.filter(|w| !w.is_zero()) else {
        return segments.to_vec();
    };
    let Some((_edge, earlier)) = segments.split_last() else {
        return Vec::new();
    };
    let mut keep_from = segments.len() - 1;
    let mut distance = Duration::ZERO;
    for (index, segment) in earlier.iter().enumerate().rev() {
        distance = distance.saturating_add(segment.duration);
        if distance > window {
            break;
        }
        keep_from = index;
    }
    segments[keep_from..].to_vec()
}
