//! Sans-IO progressive (single-file) MP4 writer driven by a sample plan.
//!
//! [`ProgressiveWriter`] writes one non-fragmented `.mp4`: an `ftyp`, a single
//! `moov` carrying the full sample tables (`stsd`/`stts`/`ctts`/`stsc`/`stsz`/
//! `stco`|`co64`/`stss`, ISO/IEC 14496-12:2026 §8.5-§8.7) and one `mdat`.
//!
//! The whole layout is computed from the
//! [`OwnedPlan`] before a sample byte arrives: chunk
//! boundaries, the cross-track interleave order, the `stco`-versus-`co64`
//! choice, the `mdat` header form (§4.2) and every sample's absolute offset.
//! [`Layout`] states the two choices the caller owns — where `moov` goes and
//! how the samples are chunked — and [`Sink`] states what the caller's sink can
//! do: seek, or only append.
//!
//! A seekable sink lets the caller place each sample at its planned offset in
//! any order; the writer holds no sample bytes. An append-only sink forces the
//! caller to push in file order, so an out-of-turn sample is buffered, and the
//! buffer is capped at the caller's `reorder_bytes`
//! ([`Error::ReorderBufferFull`](crate::Error)).
//!
//! Spec: ISO/IEC 14496-12:2026 §4.2, §8.2-§8.7
//! (`private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`).

use super::{FileType, IDENTITY_MATRIX, MVHD_RATE_UNITY, MVHD_VOLUME_UNITY, sample_entry_variant};
use crate::box_types::{BoxHeader, BoxType};
use crate::error::{Error, Result};
use crate::init_segment::{
    ChunkLargeOffsetBox, ChunkOffsetBox, EditBox, MovieBox, MovieHeaderBox, SampleDescriptionBox,
    SampleEntryVariant, SampleSizeBox, SampleTableBox, StblChild,
};
use crate::limits::Limits;
use crate::plan::{BoxPosition, MovieShell, OwnedPlan, PlannedTrack, SampleSizes};
use crate::sample_table::build_stsc_with_descriptions;
use crate::segments::{FileTypeBox, MDAT};
use crate::timing::{EditListBox, EditListEntry};
use crate::track_id::TrackId;
use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use broadcast_core::Serialize;
use broadcast_core::timescale::{Rounding, Timescale, rescale_i64, rescale_u64};

/// Milliseconds per second, for a caller's chunk length in milliseconds.
const MS_PER_SECOND: Timescale = broadcast_core::timescale!(1_000);
/// Position of `ctts` among the `stbl` children: after `stsd`/`stts` (§8.5.1).
const STBL_CTTS_INDEX: usize = 2;
/// Writes [`ProgressiveWriter::header`] can return: `ftyp`, `moov` and the
/// `mdat` header.
const HEADER_WRITES: usize = 3;
/// Writes [`ProgressiveWriter::finish`] reserves on top of the carried
/// top-level boxes: the trailing `moov` and nothing else.
const TRAILER_WRITES: usize = 2;

/// Where a finished file's bytes go.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sink {
    /// The caller can write any offset in any order.
    Seekable,
    /// The caller can only append, so samples must arrive in file order; an
    /// out-of-turn sample is held until its turn, up to `reorder_bytes` bytes.
    AppendOnly {
        /// The cap on held sample bytes, chosen by the caller.
        reorder_bytes: u64,
    },
}

/// How a track's samples are cut into chunks (§8.7.4).
///
/// Caller policy: this crate has no default, because the chunk length trades
/// interleaving granularity against chunk-table size.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interleave {
    /// Cut each track into chunks spanning about `millis` of the movie
    /// timeline, then interleave the chunks by start time.
    ByDuration {
        /// Chunk length in milliseconds of the movie timescale.
        millis: u32,
    },
    /// Reuse the plan's source chunk layout, so a file read through
    /// [`Reader::plan`](crate::reader::Reader::plan) is laid out as it was.
    AsPlanned,
}

/// The progressive writer's caller-owned choices.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// `true` places `moov` before `mdat` (progressive download).
    pub faststart: bool,
    /// How the samples are chunked.
    pub interleave: Interleave,
    /// What the caller's sink can do.
    pub sink: Sink,
}

impl Layout {
    /// A layout with the caller's box order, chunking and sink.
    #[must_use]
    pub const fn new(faststart: bool, interleave: Interleave, sink: Sink) -> Self {
        Self {
            faststart,
            interleave,
            sink,
        }
    }
}

/// The `mvhd` fields the caller owns.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovieHeader {
    /// `mvhd.timescale` (§8.2.2).
    pub timescale: Timescale,
    /// `mvhd.creation_time`, seconds since 1904-01-01.
    pub creation_time: u64,
    /// `mvhd.modification_time`, same epoch.
    pub modification_time: u64,
}

impl MovieHeader {
    /// A movie header with a zero creation/modification time.
    #[must_use]
    pub const fn new(timescale: Timescale) -> Self {
        Self {
            timescale,
            creation_time: 0,
            modification_time: 0,
        }
    }

    /// A movie header carrying the source file's epoch timestamps.
    #[must_use]
    pub const fn with_times(
        timescale: Timescale,
        creation_time: u64,
        modification_time: u64,
    ) -> Self {
        Self {
            timescale,
            creation_time,
            modification_time,
        }
    }
}

/// One run of bytes to place at an absolute file offset.
///
/// A header write owns its bytes; a pushed sample borrows the caller's buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Write<'a> {
    /// Absolute file offset of the first byte.
    pub offset: u64,
    /// The bytes to write.
    pub bytes: Cow<'a, [u8]>,
}

/// The top-level boxes a plan carries around the movie, serialized once so
/// every header/finish write can borrow them.
#[derive(Debug, Clone, Default)]
struct TopLevelExtras {
    after_ftyp: Vec<Vec<u8>>,
    after_moov: Vec<Vec<u8>>,
    after_mdat: Vec<Vec<u8>>,
}

impl TopLevelExtras {
    /// Serialize the shell's carried top-level boxes by position.
    fn new(shell: Option<&MovieShell>) -> Result<Self> {
        let Some(shell) = shell else {
            return Ok(Self::default());
        };
        Ok(Self {
            after_ftyp: serialized(shell, BoxPosition::AfterFtyp)?,
            after_moov: serialized(shell, BoxPosition::AfterMoov)?,
            after_mdat: serialized(shell, BoxPosition::AfterMdat)?,
        })
    }

    fn len_of(boxes: &[Vec<u8>]) -> u64 {
        boxes.iter().fold(0u64, |total, bytes| {
            total.saturating_add(bytes.len() as u64)
        })
    }

    fn after_ftyp_len(&self) -> u64 {
        Self::len_of(&self.after_ftyp)
    }

    fn after_moov_len(&self) -> u64 {
        Self::len_of(&self.after_moov)
    }

    fn after_mdat_len(&self) -> u64 {
        Self::len_of(&self.after_mdat)
    }

    fn total_len(&self) -> u64 {
        self.after_ftyp_len()
            .saturating_add(self.after_moov_len())
            .saturating_add(self.after_mdat_len())
    }

    /// Append one position's boxes to a write list at `offset`.
    fn append<'a>(&'a self, boxes: &'a [Vec<u8>], offset: &mut u64, out: &mut Vec<Write<'a>>) {
        for bytes in boxes {
            out.push(Write {
                offset: *offset,
                bytes: Cow::Borrowed(bytes.as_slice()),
            });
            *offset = offset.saturating_add(bytes.len() as u64);
        }
    }
}

/// The shell's carried boxes at one position, as complete box bytes.
fn serialized(shell: &MovieShell, position: BoxPosition) -> Result<Vec<Vec<u8>>> {
    shell
        .top_level_at(position)
        .map(Serialize::try_to_bytes)
        .collect()
}

/// One planned chunk: a run of consecutive samples of one track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Chunk {
    /// Index of the chunk's first sample within its track.
    first_sample: u32,
    /// Number of consecutive samples in the chunk.
    count: u32,
    /// One-based `stsd` entry the chunk's samples use.
    description_index: u32,
    /// Byte offset of the chunk's first sample, relative to the `mdat` payload.
    rel_offset: u64,
    /// File-order position of the chunk's first sample over all tracks.
    file_pos: u64,
}

/// One track's chunks and which of its samples have been pushed.
struct TrackPlacement {
    chunks: Vec<Chunk>,
    pushed: Vec<bool>,
}

impl TrackPlacement {
    /// The chunk holding `index`, by binary search over the chunk starts.
    fn chunk_of(&self, index: u32) -> Result<usize> {
        match self
            .chunks
            .binary_search_by(|chunk| chunk.first_sample.cmp(&index))
        {
            Ok(found) => Ok(found),
            Err(0) => Err(Error::NoPlan { what: "chunk" }),
            Err(next) => Ok(next - 1),
        }
    }
}

/// One buffered out-of-turn sample of an append-only sink.
struct BufferedSample {
    offset: u64,
    data: Vec<u8>,
}

/// The sink the writer was built for, as internal state.
#[derive(Debug, Clone, Copy)]
enum SinkState {
    Seekable,
    AppendOnly { reorder_bytes: u64 },
}

/// The sans-IO progressive writer returned by
/// [`Writer::progressive`](super::Writer::progressive).
pub struct ProgressiveWriter {
    plan: OwnedPlan,
    faststart: bool,
    sink: SinkState,
    ftyp: Vec<u8>,
    moov: Vec<u8>,
    mdat_header: Vec<u8>,
    extras: TopLevelExtras,
    payload_offset: u64,
    payload_len: u64,
    moov_offset: u64,
    placements: Vec<TrackPlacement>,
    order: Vec<(usize, usize)>,
    next_file_pos: u64,
    cursor: (usize, u32),
    buffered: BTreeMap<u64, BufferedSample>,
    buffered_bytes: u64,
}

impl ProgressiveWriter {
    pub(crate) fn new(
        plan: OwnedPlan,
        layout: Layout,
        file_type: FileType,
        movie: MovieHeader,
        limits: Limits,
    ) -> Result<Self> {
        if plan.tracks().is_empty() {
            return Err(Error::InvalidInput("cannot write a movie with no tracks"));
        }
        for track in plan.tracks() {
            if track.entries.is_empty() {
                return Err(Error::UnexpectedBox {
                    expected: "stsd entry",
                });
            }
            if track.tables.sample_count > limits.max_samples_per_track() {
                return Err(Error::LimitExceeded {
                    what: "samples per track",
                    limit: u64::from(limits.max_samples_per_track()),
                });
            }
            for index in 0..track.tables.sample_count {
                if track.tables.size(index)? > limits.max_sample_bytes() {
                    return Err(Error::LimitExceeded {
                        what: "sample size",
                        limit: u64::from(limits.max_sample_bytes()),
                    });
                }
            }
        }
        let extras = TopLevelExtras::new(plan.shell())?;
        let planned = plan_layout(&plan, &layout, movie.timescale)?;

        let mut ftyp = Vec::new();
        if file_type.emit_ftyp {
            let ftyp_box = FileTypeBox {
                major_brand: file_type.major_brand,
                minor_version: file_type.minor_version,
                compatible_brands: file_type.compatible_brands,
            };
            ftyp = vec![0u8; ftyp_box.serialized_len()];
            let ftyp_len = ftyp_box.serialize_into(&mut ftyp)?;
            ftyp.truncate(ftyp_len);
        }

        let payload_len = planned.payload_len;
        let mdat = BoxHeader::for_payload(BoxType::from_bytes(MDAT), None, payload_len);
        let mut mdat_header = vec![0u8; mdat.header_size()];
        mdat.serialize_into(&mut mdat_header)?;

        let source_chunks = matches!(layout.interleave, Interleave::AsPlanned);
        // A chunk whose *relative* offset already passes the 32-bit field needs
        // `co64` whatever the `moov` size turns out to be, so the measuring
        // pass below can never narrow it into `stco`.
        let mut use_co64 = planned
            .tracks
            .iter()
            .flatten()
            .any(|chunk| chunk.rel_offset > u64::from(u32::MAX));
        let (moov, payload_offset) = loop {
            let measured = build_moov(&plan, &movie, &planned.tracks, source_chunks, 0, use_co64)?;
            let measured_len = measured.len();
            let moov_len = u64::try_from(measured_len)
                .map_err(|_| Error::InvalidInput("progressive: moov exceeds the address space"))?;
            // The measuring copy holds one `stsz` (an entry per sample); free it
            // before the real build so a long file holds one `moov`, not two.
            drop(measured);
            let payload_offset = mdat_payload_offset(
                u64::try_from(ftyp.len()).unwrap_or(u64::MAX),
                moov_len,
                &extras,
                payload_len,
                layout.faststart,
            );
            if !use_co64 && needs_co64(payload_offset, &planned.tracks) {
                use_co64 = true;
                continue;
            }
            let moov = build_moov(
                &plan,
                &movie,
                &planned.tracks,
                source_chunks,
                payload_offset,
                use_co64,
            )?;
            debug_assert_eq!(moov.len(), measured_len, "moov size must be offset-stable");
            break (moov, payload_offset);
        };

        let ftyp_len = u64::try_from(ftyp.len())
            .map_err(|_| Error::InvalidInput("progressive: ftyp exceeds the address space"))?;
        let mdat_header_len = u64::try_from(mdat_header.len()).map_err(|_| {
            Error::InvalidInput("progressive: mdat header exceeds the address space")
        })?;
        let moov_offset = if layout.faststart {
            ftyp_len + extras.after_ftyp_len()
        } else {
            ftyp_len
                + extras.after_ftyp_len()
                + mdat_header_len
                + payload_len
                + extras.after_mdat_len()
        };

        let placements = planned
            .tracks
            .into_iter()
            .zip(plan.tracks())
            .map(|(chunks, track)| TrackPlacement {
                chunks,
                pushed: vec![false; track.tables.sample_count as usize],
            })
            .collect();

        Ok(Self {
            plan,
            faststart: layout.faststart,
            sink: match layout.sink {
                Sink::Seekable => SinkState::Seekable,
                Sink::AppendOnly { reorder_bytes } => SinkState::AppendOnly { reorder_bytes },
            },
            ftyp,
            moov,
            mdat_header,
            extras,
            payload_offset,
            payload_len,
            moov_offset,
            placements,
            order: planned.order,
            next_file_pos: 0,
            cursor: (0, 0),
            buffered: BTreeMap::new(),
            buffered_bytes: 0,
        })
    }

    /// Everything the file needs before its first sample byte: `ftyp`, then
    /// `moov` and the `mdat` header under a fast-start layout, or just the
    /// `mdat` header otherwise.
    #[must_use]
    pub fn header(&self) -> Vec<Write<'_>> {
        let capacity = HEADER_WRITES
            .saturating_add(self.extras.after_ftyp.len())
            .saturating_add(self.extras.after_moov.len());
        let mut out = Vec::with_capacity(capacity);
        if !self.ftyp.is_empty() {
            out.push(Write {
                offset: 0,
                bytes: Cow::Borrowed(&self.ftyp),
            });
        }
        let mut offset = u64::try_from(self.ftyp.len()).unwrap_or(u64::MAX);
        self.extras
            .append(&self.extras.after_ftyp, &mut offset, &mut out);
        if self.faststart {
            out.push(Write {
                offset,
                bytes: Cow::Borrowed(&self.moov),
            });
            offset = offset.saturating_add(u64::try_from(self.moov.len()).unwrap_or(u64::MAX));
            self.extras
                .append(&self.extras.after_moov, &mut offset, &mut out);
        }
        out.push(Write {
            offset,
            bytes: Cow::Borrowed(&self.mdat_header),
        });
        out
    }

    /// The `mdat` payload's length, in bytes.
    #[must_use]
    pub const fn payload_len(&self) -> u64 {
        self.payload_len
    }

    /// The sample plan this writer was built over: what each sample's size and
    /// position is, so a muxer feeding it can name a mismatch itself.
    #[must_use]
    pub const fn plan(&self) -> &OwnedPlan {
        &self.plan
    }

    /// The whole file's length once every write has been placed: the size a
    /// caller with a resizeable sink should allocate up front. The box order
    /// does not change it.
    #[must_use]
    pub fn file_len(&self) -> u64 {
        [self.ftyp.len(), self.moov.len(), self.mdat_header.len()]
            .into_iter()
            .try_fold(
                self.payload_len.saturating_add(self.extras.total_len()),
                |total, len| total.checked_add(u64::try_from(len).ok()?),
            )
            .unwrap_or(u64::MAX)
    }
}

impl ProgressiveWriter {
    /// Place one sample's bytes.
    ///
    /// The length must equal the plan's entry for `index`
    /// ([`Error::PlanMismatch`](crate::Error)). A seekable sink
    /// takes any order; an append-only sink holds an out-of-turn sample until
    /// its turn, and refuses one that would grow the buffer past the caller's
    /// cap ([`Error::ReorderBufferFull`](crate::Error)).
    pub fn push<'a>(
        &'a mut self,
        track: TrackId,
        index: u32,
        data: &'a [u8],
    ) -> Result<Vec<Write<'a>>> {
        let track_index = self
            .plan
            .index_of(track)
            .ok_or(Error::NoPlan { what: "track" })?;
        let planned_track = &self.plan.tracks()[track_index];
        if index >= planned_track.tables.sample_count {
            return Err(Error::NoPlan { what: "sample" });
        }
        let planned = planned_track.tables.size(index)?;
        let got = u64::try_from(data.len()).map_err(|_| Error::PlanMismatch {
            track,
            index,
            planned,
            got: u64::MAX,
        })?;
        if got != u64::from(planned) {
            return Err(Error::PlanMismatch {
                track,
                index,
                planned,
                got,
            });
        }
        if self.placements[track_index].pushed[index as usize] {
            return Err(Error::InvalidInput("sample pushed twice"));
        }
        let offset = self.sample_offset(track_index, index)?;
        match self.sink {
            SinkState::Seekable => {
                self.placements[track_index].pushed[index as usize] = true;
                Ok(vec![Write {
                    offset,
                    bytes: Cow::Borrowed(data),
                }])
            }
            SinkState::AppendOnly { reorder_bytes } => {
                let position = self.file_position(track_index, index)?;
                if position != self.next_file_pos {
                    let len = got;
                    if self.buffered_bytes.saturating_add(len) > reorder_bytes {
                        return Err(Error::ReorderBufferFull { cap: reorder_bytes });
                    }
                }
                let mut out = Vec::new();
                self.placements[track_index].pushed[index as usize] = true;
                if position == self.next_file_pos {
                    out.push(Write {
                        offset,
                        bytes: Cow::Borrowed(data),
                    });
                    self.advance_file_pos();
                    self.drain(&mut out);
                } else {
                    self.buffered.insert(
                        position,
                        BufferedSample {
                            offset,
                            data: data.to_vec(),
                        },
                    );
                    self.buffered_bytes = self.buffered_bytes.saturating_add(got);
                }
                Ok(out)
            }
        }
    }

    /// Finish the file: every planned sample must have been pushed, then the
    /// trailing `moov` is emitted for a non-fast-start layout.
    pub fn finish(&mut self) -> Result<Vec<Write<'_>>> {
        for (track_index, track) in self.plan.tracks().iter().enumerate() {
            if let Some(index) = self.placements[track_index]
                .pushed
                .iter()
                .position(|pushed| !pushed)
            {
                return Err(Error::PlanIncomplete {
                    track: track.setup.track_id,
                    missing: u32::try_from(index).unwrap_or(u32::MAX),
                });
            }
        }
        if self.faststart {
            let mut out = Vec::with_capacity(self.extras.after_mdat.len());
            let mut offset = self.payload_offset.saturating_add(self.payload_len);
            self.extras
                .append(&self.extras.after_mdat, &mut offset, &mut out);
            return Ok(out);
        }
        let mut out = Vec::with_capacity(
            TRAILER_WRITES + self.extras.after_mdat.len() + self.extras.after_moov.len(),
        );
        let mut offset = self.payload_offset.saturating_add(self.payload_len);
        self.extras
            .append(&self.extras.after_mdat, &mut offset, &mut out);
        out.push(Write {
            offset: self.moov_offset,
            bytes: Cow::Borrowed(&self.moov),
        });
        let mut offset = self.moov_offset.saturating_add(self.moov.len() as u64);
        self.extras
            .append(&self.extras.after_moov, &mut offset, &mut out);
        Ok(out)
    }

    /// The absolute file offset of one planned sample.
    fn sample_offset(&self, track_index: usize, index: u32) -> Result<u64> {
        let placement = self
            .placements
            .get(track_index)
            .ok_or(Error::NoPlan { what: "track" })?;
        let chunk = placement
            .chunks
            .get(placement.chunk_of(index)?)
            .ok_or(Error::NoPlan { what: "chunk" })?;
        let mut offset = self.payload_offset.saturating_add(chunk.rel_offset);
        let tables = &self
            .plan
            .tracks()
            .get(track_index)
            .ok_or(Error::NoPlan { what: "track" })?
            .tables;
        let mut sample = chunk.first_sample;
        while sample < index {
            offset = offset.saturating_add(u64::from(tables.size(sample)?));
            sample += 1;
        }
        Ok(offset)
    }

    /// A sample's position in file order.
    fn file_position(&self, track_index: usize, index: u32) -> Result<u64> {
        let placement = self
            .placements
            .get(track_index)
            .ok_or(Error::NoPlan { what: "track" })?;
        let chunk = placement
            .chunks
            .get(placement.chunk_of(index)?)
            .ok_or(Error::NoPlan { what: "chunk" })?;
        Ok(chunk
            .file_pos
            .saturating_add(u64::from(index.saturating_sub(chunk.first_sample))))
    }

    /// Step the append-only cursor one sample forward in file order.
    fn advance_file_pos(&mut self) {
        let (order_index, within) = self.cursor;
        let Some(&(track_index, chunk_index)) = self.order.get(order_index) else {
            return;
        };
        let count = self
            .placements
            .get(track_index)
            .and_then(|placement| placement.chunks.get(chunk_index))
            .map_or(0, |chunk| chunk.count);
        self.cursor = if within.saturating_add(1) >= count {
            (order_index.saturating_add(1), 0)
        } else {
            (order_index, within.saturating_add(1))
        };
        self.next_file_pos = self.next_file_pos.saturating_add(1);
    }

    /// Emit every buffered sample whose turn has come.
    fn drain<'a>(&mut self, out: &mut Vec<Write<'a>>) {
        while let Some(sample) = self.buffered.remove(&self.next_file_pos) {
            let len = u64::try_from(sample.data.len()).unwrap_or(u64::MAX);
            out.push(Write {
                offset: sample.offset,
                bytes: Cow::Owned(sample.data),
            });
            self.buffered_bytes = self.buffered_bytes.saturating_sub(len);
            self.advance_file_pos();
        }
    }
}

/// A planned `mdat`: every track's chunks and the cross-track file order.
struct PlannedLayout {
    tracks: Vec<Vec<Chunk>>,
    order: Vec<(usize, usize)>,
    payload_len: u64,
}

/// Per-sample decode times of one track, from the `stts` runs (§8.6.1.2).
struct DecodeTimes<'a> {
    entries: &'a [crate::timing::SttsEntry],
    run: usize,
    left: u32,
    delta: u32,
    time: u64,
    emitted: u32,
    total: u32,
}

impl<'a> DecodeTimes<'a> {
    fn new(stts: &'a crate::timing::TimeToSampleBox, total: u32, start: u64) -> Self {
        Self {
            entries: &stts.entries,
            run: 0,
            left: 0,
            delta: 0,
            time: start,
            emitted: 0,
            total,
        }
    }

    fn next_time(&mut self) -> Result<Option<u64>> {
        if self.emitted >= self.total {
            return Ok(None);
        }
        while self.left == 0 {
            let entry = self.entries.get(self.run).ok_or(Error::InvalidInput(
                "stts covers fewer samples than stsz (ISO/IEC 14496-12:2026 §8.6.1.2)",
            ))?;
            self.run += 1;
            self.left = entry.sample_count;
            self.delta = entry.sample_delta;
        }
        let now = self.time;
        self.time = self.time.saturating_add(u64::from(self.delta));
        self.left -= 1;
        self.emitted += 1;
        Ok(Some(now))
    }
}

/// Lay the plan's samples out in the `mdat` payload.
///
/// Every chunk of every track is cut first (per [`Interleave`]), then the
/// chunks are merged into file order and their payload offsets assigned in
/// that order.
fn plan_layout(
    plan: &OwnedPlan,
    layout: &Layout,
    movie_timescale: Timescale,
) -> Result<PlannedLayout> {
    let mut pending: Vec<(u64, usize, usize, usize)> = Vec::new();
    let mut track_chunks: Vec<Vec<Chunk>> = Vec::with_capacity(plan.tracks().len());
    for (index, track) in plan.tracks().iter().enumerate() {
        let chunks = match layout.interleave {
            Interleave::ByDuration { millis } => duration_chunks(track, millis, movie_timescale)?,
            Interleave::AsPlanned => planned_chunks(track)?,
        };
        for &(key, first, count, _) in &chunks {
            pending.push((
                key,
                index,
                usize::try_from(first).unwrap_or(usize::MAX),
                usize::try_from(count).unwrap_or(usize::MAX),
            ));
        }
        track_chunks.push(
            chunks
                .iter()
                .map(|&(_, first, count, description_index)| Chunk {
                    first_sample: first,
                    count,
                    description_index,
                    rel_offset: 0,
                    file_pos: 0,
                })
                .collect(),
        );
    }
    let merged = interleave_order(pending);
    let mut order = Vec::with_capacity(merged.len());
    let mut cursors = vec![0usize; track_chunks.len()];
    let mut payload_len = 0u64;
    let mut file_pos = 0u64;
    for (_, track_index, first, count) in merged {
        let index = cursors[track_index];
        let chunk = track_chunks
            .get_mut(track_index)
            .and_then(|chunks| chunks.get_mut(index))
            .ok_or(Error::InvalidInput(
                "progressive: chunk plan is inconsistent",
            ))?;
        if usize::try_from(chunk.first_sample).unwrap_or(usize::MAX) != first
            || usize::try_from(chunk.count).unwrap_or(usize::MAX) != count
        {
            return Err(Error::InvalidInput(
                "progressive: chunk plan is out of order",
            ));
        }
        cursors[track_index] += 1;
        chunk.rel_offset = payload_len;
        chunk.file_pos = file_pos;
        let tables = &plan
            .tracks()
            .get(track_index)
            .ok_or(Error::NoPlan { what: "track" })?
            .tables;
        for sample in chunk.first_sample..chunk.first_sample.saturating_add(chunk.count) {
            payload_len = payload_len.saturating_add(u64::from(tables.size(sample)?));
        }
        file_pos = file_pos.saturating_add(u64::from(chunk.count));
        order.push((track_index, index));
    }
    Ok(PlannedLayout {
        tracks: track_chunks,
        order,
        payload_len,
    })
}

/// Cut one track into chunks of about `millis` of the movie timeline.
///
/// Each chunk starts at its first sample's decode time and runs while the next
/// sample is inside the window and uses the same `stsd` entry (`stsc` is
/// per-chunk, §8.7.4). A track whose own decode times step backwards still
/// keeps its samples in order: the boundary test is on the elapsed span from
/// the chunk's start, and the interleave merges per-track sequences rather than
/// sorting a flat list.
fn duration_chunks(
    track: &PlannedTrack,
    millis: u32,
    movie_timescale: Timescale,
) -> Result<Vec<(u64, u32, u32, u32)>> {
    let total = track.tables.sample_count;
    if total == 0 {
        return Ok(Vec::new());
    }
    let timescale = track.setup.timescale;
    let to_movie = |ticks: u64| -> Result<u64> {
        let ticks = i64::try_from(ticks).map_err(|_| {
            Error::InvalidInput("progressive: decode time is past the movie timeline")
        })?;
        let movie = rescale_i64(ticks, timescale, movie_timescale, Rounding::Floor);
        Ok(u64::try_from(movie).unwrap_or(0))
    };
    let chunk_ticks = rescale_u64(
        u64::from(millis),
        MS_PER_SECOND,
        movie_timescale,
        Rounding::Floor,
    )
    .max(1);
    let mut times = DecodeTimes::new(&track.tables.stts, total, track.start_decode_time);
    let mut pending = Vec::new();
    let Some(mut now) = times.next_time()? else {
        return Ok(Vec::new());
    };
    let mut index = 0u32;
    loop {
        let start_tick = to_movie(now)?;
        let description = track.tables.description_index(index);
        let first = index;
        let mut end = index.saturating_add(1);
        let mut next = times.next_time()?;
        while end < total {
            let Some(time) = next else {
                break;
            };
            if to_movie(time)?.saturating_sub(start_tick) >= chunk_ticks {
                break;
            }
            if track.tables.description_index(end) != description {
                break;
            }
            end = end.saturating_add(1);
            next = times.next_time()?;
        }
        pending.push((start_tick, first, end.saturating_sub(first), description));
        if end >= total {
            break;
        }
        now = next.ok_or(Error::InvalidInput(
            "stts covers fewer samples than stsz (ISO/IEC 14496-12:2026 §8.6.1.2)",
        ))?;
        index = end;
    }
    Ok(pending)
}

/// The plan's source chunk layout, for [`Interleave::AsPlanned`].
fn planned_chunks(track: &PlannedTrack) -> Result<Vec<(u64, u32, u32, u32)>> {
    let chunks = track.chunks.as_ref().ok_or(Error::NoPlan {
        what: "source chunk table",
    })?;
    let mut pending = Vec::with_capacity(chunks.samples_per_chunk.len());
    let mut first = 0u32;
    for ((&count, &offset), &description) in chunks
        .samples_per_chunk
        .iter()
        .zip(&chunks.offsets)
        .zip(&chunks.description_index)
    {
        pending.push((offset, first, count, description));
        first = first.saturating_add(count);
    }
    if first != track.tables.sample_count {
        return Err(Error::InvalidInput(
            "progressive: the source chunk table does not cover every sample",
        ));
    }
    Ok(pending)
}

/// Order the per-track chunks into the `mdat` interleave.
///
/// The cross-track order is by `key`; the per-track order is **sample order**,
/// which the key cannot express when a track's own chunks step backwards. Each
/// track's sequence is merged rather than sorted, so the merge's cursor never
/// has to reject a track's chunk arriving out of turn. Ties break on the lowest
/// track index, so the result is deterministic.
fn interleave_order<K: Ord + Copy>(
    pending: Vec<(K, usize, usize, usize)>,
) -> Vec<(K, usize, usize, usize)> {
    let mut per_track: BTreeMap<usize, Vec<(K, usize, usize, usize)>> = BTreeMap::new();
    for entry in pending {
        per_track.entry(entry.1).or_default().push(entry);
    }
    let mut heads: Vec<usize> = vec![0; per_track.len()];
    let mut out = Vec::new();
    loop {
        let mut best: Option<(usize, K)> = None;
        for (&track, chunks) in &per_track {
            let Some(head) = chunks.get(heads[track]) else {
                continue;
            };
            if best.is_none_or(|(_, key)| head.0 < key) {
                best = Some((track, head.0));
            }
        }
        let Some((track, _)) = best else {
            break;
        };
        out.push(per_track[&track][heads[track]]);
        heads[track] += 1;
    }
    out
}

/// Absolute file offset of the `mdat` payload: after `ftyp` (and `moov` under
/// fast start) and the `mdat` header, whose length depends on the payload size
/// (16 bytes once the box needs `largesize`, §4.2).
fn mdat_payload_offset(
    ftyp_len: u64,
    moov_len: u64,
    extras: &TopLevelExtras,
    mdat_payload_len: u64,
    faststart: bool,
) -> u64 {
    let mut before = ftyp_len.saturating_add(extras.after_ftyp_len());
    if faststart {
        before = before
            .saturating_add(moov_len)
            .saturating_add(extras.after_moov_len());
    }
    let header = BoxHeader::for_payload(BoxType::from_bytes(MDAT), None, mdat_payload_len);
    before + header.header_size() as u64
}

/// Whether any chunk's absolute offset needs the 64-bit `co64` (§8.7.5).
fn needs_co64(mdat_payload_offset: u64, tracks: &[Vec<Chunk>]) -> bool {
    tracks
        .iter()
        .flatten()
        .any(|chunk| mdat_payload_offset.saturating_add(chunk.rel_offset) > u64::from(u32::MAX))
}

/// Assemble the file's `moov` from the plan and the chunk offsets.
fn build_moov(
    plan: &OwnedPlan,
    movie: &MovieHeader,
    chunks: &[Vec<Chunk>],
    source_chunks: bool,
    payload_offset: u64,
    use_co64: bool,
) -> Result<Vec<u8>> {
    let mut tracks = Vec::with_capacity(plan.tracks().len());
    let mut max_presentation = 0u64;
    for (index, track) in plan.tracks().iter().enumerate() {
        let track_chunks = chunks
            .get(index)
            .ok_or(Error::NoPlan { what: "chunk plan" })?;
        let offsets: Vec<u64> = track_chunks
            .iter()
            .map(|chunk| payload_offset.saturating_add(chunk.rel_offset))
            .collect();
        let stbl = sample_table(track, track_chunks, &offsets, source_chunks, use_co64)?;
        let mut trak = super::track_box(&track.setup, stbl)?;
        apply_durations(&mut trak, track);
        max_presentation = max_presentation.max(track.presentation_duration);
        tracks.push(trak);
    }
    let next_track_id = plan
        .tracks()
        .iter()
        .map(|track| track.setup.track_id)
        .max()
        .map_or(TrackId::new(1), TrackId::next);
    let moov = match plan.shell() {
        Some(shell) => {
            let mut mvhd = shell.mvhd.clone();
            if !duration_fits_v0(mvhd.duration) {
                mvhd.version = 1;
            }
            MovieBox {
                mvhd,
                tracks,
                mvex: shell.mvex.clone(),
                opaque: shell.opaque.clone(),
                order: shell.order.clone(),
            }
        }
        None => {
            let mvhd = MovieHeaderBox {
                version: u8::from(!duration_fits_v0(max_presentation)),
                flags: 0,
                creation_time: movie.creation_time,
                modification_time: movie.modification_time,
                timescale: movie.timescale,
                duration: max_presentation,
                rate: MVHD_RATE_UNITY,
                volume: MVHD_VOLUME_UNITY,
                reserved: Default::default(),
                pre_defined: Default::default(),
                matrix: IDENTITY_MATRIX,
                next_track_id,
            };
            MovieBox::new(mvhd, tracks, None, vec![])
        }
    };
    let mut out = vec![0u8; moov.serialized_len()];
    let written = moov.serialize_into(&mut out)?;
    out.truncate(written);
    Ok(out)
}

/// One track's `stbl`, with the full sample tables (§8.5.1-§8.7.5).
fn sample_table(
    track: &PlannedTrack,
    chunks: &[Chunk],
    offsets: &[u64],
    source_chunks: bool,
    use_co64: bool,
) -> Result<SampleTableBox> {
    let sizes = match &track.tables.sizes {
        SampleSizes::Uniform(size) => StblChild::Stsz(SampleSizeBox {
            version: 0,
            flags: 0,
            sample_size: *size,
            sample_count: track.tables.sample_count,
            entries: Vec::new(),
        }),
        SampleSizes::PerSample(entries) => StblChild::Stsz(SampleSizeBox {
            version: 0,
            flags: 0,
            sample_size: 0,
            sample_count: track.tables.sample_count,
            entries: entries.clone(),
        }),
        SampleSizes::Compact(compact) => StblChild::Stz2(compact.clone()),
    };
    let stsc = if source_chunks {
        track
            .chunks
            .as_ref()
            .ok_or(Error::NoPlan {
                what: "source chunk table",
            })?
            .stsc
            .clone()
    } else {
        let counts: Vec<u32> = chunks.iter().map(|chunk| chunk.count).collect();
        let descriptions: Vec<u32> = chunks.iter().map(|chunk| chunk.description_index).collect();
        build_stsc_with_descriptions(&counts, &descriptions)?
    };
    let mut children = vec![
        StblChild::Stsd(sample_description(track)?),
        StblChild::Stts(track.tables.stts.clone()),
        StblChild::Stsc(stsc),
        sizes,
    ];
    if let Some(ctts) = &track.tables.ctts {
        children.insert(STBL_CTTS_INDEX, StblChild::Ctts(ctts.clone()));
    }
    if use_co64 {
        children.push(StblChild::Co64(ChunkLargeOffsetBox {
            version: 0,
            flags: 0,
            entries: offsets.to_vec(),
        }));
    } else {
        let entries: Vec<u32> = offsets
            .iter()
            .map(|&offset| {
                u32::try_from(offset).map_err(|_| {
                    Error::InvalidInput("progressive: a chunk offset exceeds the 32-bit stco field")
                })
            })
            .collect::<Result<_>>()?;
        children.push(StblChild::Stco(ChunkOffsetBox {
            version: 0,
            flags: 0,
            entries,
        }));
    }
    if let Some(stss) = &track.tables.stss {
        children.push(StblChild::Stss(stss.clone()));
    }
    Ok(SampleTableBox {
        children: stbl_children(stbl_template(track.setup.shell.as_ref()), children),
    })
}

/// The source `stbl` child order, when the track carries its parsed `trak`.
pub(crate) fn stbl_template(shell: Option<&crate::init_segment::TrackBox>) -> Option<&[StblChild]> {
    shell?
        .mdia
        .as_ref()?
        .minf
        .as_ref()?
        .stbl
        .as_ref()
        .map(|stbl| stbl.children.as_slice())
}

/// Whether the plan rebuilds a table child, so the source's value must not be
/// carried (its offsets and pointers describe the source layout).
fn rebuilt(kind: &[u8; crate::box_types::BOX_TYPE_SIZE]) -> bool {
    kind == b"stsd"
        || kind == b"stts"
        || kind == b"ctts"
        || kind == b"stsc"
        || kind == b"stsz"
        || kind == b"stz2"
        || kind == b"stco"
        || kind == b"co64"
        || kind == b"stss"
}

/// The four-CC of one `stbl` child.
fn stbl_four_cc(child: &StblChild) -> [u8; crate::box_types::BOX_TYPE_SIZE] {
    use crate::box_types::{BOX_HEADER_MIN_SIZE, BOX_TYPE_OFFSET, BOX_TYPE_SIZE};
    match child {
        StblChild::Stsd(_) => *b"stsd",
        StblChild::Stts(_) => *b"stts",
        StblChild::Ctts(_) => *b"ctts",
        StblChild::Stsc(_) => *b"stsc",
        StblChild::Stsz(_) => *b"stsz",
        StblChild::Stz2(_) => *b"stz2",
        StblChild::Sdtp(_) => *b"sdtp",
        StblChild::Stco(_) => *b"stco",
        StblChild::Co64(_) => *b"co64",
        StblChild::Stss(_) => *b"stss",
        StblChild::Stdp(_) => *b"stdp",
        StblChild::Padb(_) => *b"padb",
        StblChild::Csgp(_) => *b"csgp",
        StblChild::Opaque(bytes) => bytes
            .get(BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE)
            .and_then(|slice| <[u8; BOX_TYPE_SIZE]>::try_from(slice).ok())
            .unwrap_or([0u8; BOX_TYPE_SIZE]),
    }
}

/// Emit the plan's rebuilt tables in the source `stbl` child order, carrying
/// every child the plan does not model (`sgpd`, `sbgp`, a typed `sdtp`, an
/// opaque box) in place.
///
/// A plan a [`PlanBuilder`](crate::plan::PlanBuilder) made has no template and
/// keeps this crate's conventional order.
pub(crate) fn stbl_children(
    template: Option<&[StblChild]>,
    built: Vec<StblChild>,
) -> Vec<StblChild> {
    let Some(template) = template else {
        return built;
    };
    let mut used = alloc::vec![false; built.len()];
    let mut out = Vec::with_capacity(built.len().max(template.len()));
    for want in template {
        let kind = stbl_four_cc(want);
        let found = built
            .iter()
            .enumerate()
            .position(|(index, child)| !used[index] && stbl_four_cc(child) == kind);
        match found {
            Some(index) => {
                used[index] = true;
                out.push(built[index].clone());
            }
            None => {
                if !rebuilt(&kind) {
                    out.push(want.clone());
                }
            }
        }
    }
    for (index, child) in built.into_iter().enumerate() {
        if !used[index] {
            out.push(child);
        }
    }
    out
}

/// The track's `stsd` entries (§8.5.2).
fn sample_description(track: &PlannedTrack) -> Result<SampleDescriptionBox> {
    let mut entries: Vec<SampleEntryVariant> = Vec::with_capacity(track.entries.len());
    for entry in &track.entries {
        entries.push(sample_entry_variant(entry)?);
    }
    let version = stsd_template_version(track).unwrap_or_else(|| {
        entries
            .iter()
            .map(SampleEntryVariant::required_stsd_version)
            .max()
            .unwrap_or(0)
    });
    Ok(SampleDescriptionBox {
        version,
        flags: 0,
        entries,
    })
}

/// The source `stsd` version, when the track carries its parsed `trak`.
fn stsd_template_version(track: &PlannedTrack) -> Option<u8> {
    stbl_template(track.setup.shell.as_ref())?
        .iter()
        .find_map(|child| match child {
            StblChild::Stsd(stsd) => Some(stsd.version),
            _ => None,
        })
}

/// Set the header durations and the edit list a non-fragmented movie carries
/// in `mvhd`/`tkhd`/`mdhd` (§8.2.2.2, §8.3.2.2, §8.4.2.2, §8.6.6).
fn apply_durations(trak: &mut crate::init_segment::TrackBox, track: &PlannedTrack) {
    // A source header keeps the version the file stated (a QuickTime or
    // out-of-spec version is not this writer's to normalize); only a duration
    // that no longer fits the 32-bit form forces the 64-bit one.
    if !duration_fits_v0(track.presentation_duration) {
        trak.tkhd.version = 1;
    }
    trak.tkhd.duration = track.presentation_duration;
    trak.edts = edit_box(&track.edits);
    if trak.edts.is_some() && track.edts_after_mdia && trak.order.is_empty() {
        // `edts` goes after `mdia`, the order this crate's progressive output
        // has always carried; a plan read from a file keeps that file's order.
        let mut order = crate::box_support::ChildOrder::default();
        order.push(*b"tkhd");
        order.push(*b"mdia");
        order.push(*b"edts");
        trak.order = order;
    }
    if let Some(mdhd) = trak.mdia.as_mut().and_then(|media| media.mdhd.as_mut()) {
        if !duration_fits_v0(track.media_duration) {
            mdhd.version = 1;
        }
        mdhd.duration = track.media_duration;
    }
}

/// The `edts` carrying the plan's `elst` entries, or `None` when it has none.
fn edit_box(edits: &[EditListEntry]) -> Option<EditBox> {
    if edits.is_empty() {
        return None;
    }
    // Version 1 carries 64-bit fields; version 0 only 32-bit (§8.6.6).
    let wide = edits.iter().any(|entry| {
        u32::try_from(entry.segment_duration).is_err() || i32::try_from(entry.media_time).is_err()
    });
    Some(EditBox::new(
        Some(EditListBox {
            version: u8::from(wide),
            flags: 0,
            entries: edits.to_vec(),
        }),
        Vec::new(),
    ))
}

/// Whether a header box's `duration` fits the 32-bit version-0 field
/// (§8.2.2.2).
fn duration_fits_v0(duration: u64) -> bool {
    duration <= u64::from(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timing::{SttsEntry, TimeToSampleBox};

    /// The interleave must keep each track's own chunks in sample order even
    /// when that track's chunk-start keys step backwards.
    #[test]
    fn interleave_order_keeps_a_track_in_sample_order() {
        let pending = vec![
            (0i64, 0usize, 0usize, 1usize),
            (1000, 0, 1, 1),
            (2000, 0, 2, 1),
            (500, 0, 3, 1),
            (3000, 0, 4, 1),
            (500, 1, 0, 1),
            (1500, 1, 1, 1),
        ];
        let ordered = interleave_order(pending);

        let mut cursor = [0usize; 2];
        for &(_, track, first, _) in &ordered {
            assert_eq!(
                first, cursor[track],
                "track {track} chunk arrived out of sample order: {ordered:?}"
            );
            cursor[track] += 1;
        }
        assert_eq!(cursor, [5, 2], "every chunk was consumed");
        let tracks: Vec<usize> = ordered.iter().map(|&(_, t, ..)| t).collect();
        assert_eq!(
            tracks,
            vec![0, 1, 0, 1, 0, 0, 0],
            "tracks interleave by key: {ordered:?}"
        );
    }

    /// On equal chunk-start keys the lowest track index wins, so the `mdat`
    /// order (and every `stco` offset) is deterministic.
    #[test]
    fn interleave_order_breaks_ties_on_the_lowest_track_index() {
        let ordered = interleave_order(vec![(0i64, 1usize, 0usize, 1usize), (0, 0, 0, 1)]);
        assert_eq!(
            ordered,
            vec![(0, 0, 0, 1), (0, 1, 0, 1)],
            "track 0's chunk must come first on a tie"
        );
    }

    #[test]
    fn mdat_payload_offset_accounts_for_largesize_header() {
        let fits = u64::from(u32::MAX) - 8;
        let over = fits + 1;
        let none = TopLevelExtras::default();
        assert_eq!(
            mdat_payload_offset(24, 1000, &none, fits, true),
            24 + 1000 + 8
        );
        assert_eq!(mdat_payload_offset(24, 1000, &none, fits, false), 24 + 8);
        assert_eq!(
            mdat_payload_offset(24, 1000, &none, over, true),
            24 + 1000 + 16
        );
        assert_eq!(mdat_payload_offset(24, 1000, &none, over, false), 24 + 16);
    }

    /// Top-level boxes the plan carries shift the `mdat` payload exactly by
    /// their serialized length, at each position the fast-start choice reaches.
    #[test]
    fn mdat_payload_offset_accounts_for_carried_top_level_boxes() {
        let extras = TopLevelExtras {
            after_ftyp: alloc::vec![alloc::vec![0u8; 8]],
            after_moov: alloc::vec![alloc::vec![0u8; 4]],
            after_mdat: alloc::vec![alloc::vec![0u8; 2]],
        };
        assert_eq!(
            mdat_payload_offset(24, 1000, &extras, 10, true),
            24 + 8 + 1000 + 4 + 8
        );
        assert_eq!(
            mdat_payload_offset(24, 1000, &extras, 10, false),
            24 + 8 + 8
        );
        assert_eq!(extras.total_len(), 14);
    }

    #[test]
    fn co64_chosen_once_an_offset_exceeds_u32() {
        let base = 1024;
        let layout = |offsets: &[u64]| {
            vec![
                offsets
                    .iter()
                    .map(|&rel_offset| Chunk {
                        first_sample: 0,
                        count: 1,
                        description_index: 1,
                        rel_offset,
                        file_pos: 0,
                    })
                    .collect::<Vec<_>>(),
            ]
        };
        assert!(!needs_co64(base, &layout(&[0, u64::from(u32::MAX) - base])));
        assert!(needs_co64(
            base,
            &layout(&[0, u64::from(u32::MAX) - base + 1])
        ));
    }

    #[test]
    fn decode_times_walk_the_stts_runs() {
        let stts = TimeToSampleBox {
            version: 0,
            flags: 0,
            entries: vec![
                SttsEntry {
                    sample_count: 2,
                    sample_delta: 10,
                },
                SttsEntry {
                    sample_count: 1,
                    sample_delta: 40,
                },
            ],
        };
        let mut times = DecodeTimes::new(&stts, 3, 7);
        assert_eq!(times.next_time().expect("time"), Some(7));
        assert_eq!(times.next_time().expect("time"), Some(17));
        assert_eq!(times.next_time().expect("time"), Some(27));
        assert_eq!(times.next_time().expect("time"), None);
    }

    #[test]
    fn a_short_stts_is_an_error_not_a_panic() {
        let stts = TimeToSampleBox {
            version: 0,
            flags: 0,
            entries: vec![SttsEntry {
                sample_count: 1,
                sample_delta: 10,
            }],
        };
        let mut times = DecodeTimes::new(&stts, 2, 0);
        assert_eq!(times.next_time().expect("time"), Some(0));
        assert!(matches!(times.next_time(), Err(Error::InvalidInput(_))));
    }
}
