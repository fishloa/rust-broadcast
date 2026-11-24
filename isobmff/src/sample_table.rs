//! Lazy per-track sample cursor over the ISO/IEC 14496-12:2026 sample tables.
//!
//! The rules that turn `stts`/`ctts`/`stsc`/`stsz`/`stco`/`co64`/`stss`
//! (§8.6–§8.7) into per-sample positions live here, in one forward pass. The
//! cursor yields one [`SampleRef`] at a time and never expands a run-length
//! table to one entry per sample, so a hostile `sample_count` cannot make the
//! crate allocate in proportion to it.
//!
//! Offsets are `u64` and counts are `u32`, as on the wire. A conversion to
//! `usize` for indexing a caller's buffer is checked by that caller.

use crate::box_types::{BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE};
use crate::error::{Error, Result};
use crate::init_segment::{
    ChunkLargeOffsetBox, ChunkOffsetBox, CompactSampleSizeBox, SampleSizeBox, SampleTableBox,
    SampleToChunkBox, StblChild, StscEntry, SyncSampleBox,
};
use crate::limits::Limits;
use crate::timing::{CompositionOffsetBox, TimeToSampleBox};
use crate::track_id::TrackId;
use alloc::vec::Vec;
use broadcast_common::Parse;
use broadcast_common::error::BufferTooShort;

/// Zero-based index a `stss` sync list is compared against, plus one.
const SYNC_INDEX_BIAS: u32 = 1;

/// One sample's position and timing, handed out by [`TrackCursor`].
///
/// Plain data: it borrows nothing from the tables or the file, so a fragment
/// sample can be kept after the `moof` buffer is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SampleRef {
    /// The track this sample belongs to.
    pub track: TrackId,
    /// Zero-based index in decode order.
    pub index: u32,
    /// Decode time in the track timescale, from 0.
    pub decode_time: u64,
    /// `ctts` composition offset (§8.6.1.3); 0 when the box is absent.
    pub composition_offset: i32,
    /// Decode duration, in the track timescale (§8.6.1.2).
    pub duration: u32,
    /// Whether `stss` lists this sample as random-access (§8.6.2); every
    /// sample is sync when the box is absent.
    pub sync: bool,
    /// Absolute file byte offset of the sample's data.
    pub offset: u64,
    /// Size of the sample's data, in bytes (§8.7.3).
    pub size: u32,
    /// One-based `stsd` entry that describes this sample (§8.7.4).
    pub description_index: u32,
}

/// The size table a track carries: `stsz` or the compact `stz2` (§8.7.3).
#[derive(Debug, Clone, Copy)]
enum SizeTable<'a> {
    Stsz(&'a SampleSizeBox),
    Stz2(&'a CompactSampleSizeBox),
}

impl SizeTable<'_> {
    fn count(self) -> u32 {
        match self {
            Self::Stsz(b) => b.sample_count,
            Self::Stz2(b) => b.sample_count,
        }
    }

    fn size(self, index: u32) -> Result<u32> {
        match self {
            Self::Stsz(b) if b.sample_size != 0 => Ok(b.sample_size),
            Self::Stsz(b) => b
                .entries
                .get(index as usize)
                .copied()
                .ok_or(Error::InvalidInput("stsz has fewer entries than samples")),
            Self::Stz2(b) => b
                .entries
                .get(index as usize)
                .copied()
                .ok_or(Error::InvalidInput("stz2 has fewer entries than samples")),
        }
    }

    fn walk_sizes(self, count: u32, mut check: impl FnMut(u32) -> Result<()>) -> Result<()> {
        match self {
            Self::Stsz(b) if b.sample_size != 0 => check(b.sample_size),
            Self::Stsz(b) => {
                if b.entries.len() < count as usize {
                    return Err(Error::InvalidInput("stsz has fewer entries than samples"));
                }
                for &size in b.entries.iter().take(count as usize) {
                    check(size)?;
                }
                Ok(())
            }
            Self::Stz2(b) => {
                if b.entries.len() < count as usize {
                    return Err(Error::InvalidInput("stz2 has fewer entries than samples"));
                }
                for &size in b.entries.iter().take(count as usize) {
                    check(size)?;
                }
                Ok(())
            }
        }
    }
}

/// The chunk-offset table a track carries: `stco` or `co64` (§8.7.5).
#[derive(Debug, Clone, Copy)]
enum ChunkOffsets<'a> {
    Stco(&'a ChunkOffsetBox),
    Co64(&'a ChunkLargeOffsetBox),
}

impl ChunkOffsets<'_> {
    fn len(self) -> usize {
        match self {
            Self::Stco(b) => b.entries.len(),
            Self::Co64(b) => b.entries.len(),
        }
    }

    fn get(self, index: u32) -> u64 {
        match self {
            Self::Stco(b) => u64::from(b.entries[index as usize]),
            Self::Co64(b) => b.entries[index as usize],
        }
    }
}

/// The iteration state, small enough to snapshot for [`TrackCursor::seek_sync`].
#[derive(Debug, Clone, Copy)]
struct Pos {
    sample: u32,
    decode_time: u64,
    total_bytes: u64,
    stts_run: usize,
    stts_left: u32,
    ctts_run: usize,
    ctts_left: u32,
    chunk_no: u32,
    chunk_remaining: u32,
    chunk_cursor: u64,
    stsc_run: usize,
    stss_pos: usize,
}

/// One track's sample tables, walked lazily in decode order.
///
/// Borrows the typed boxes out of [`SampleTableBox`]; it allocates nothing
/// proportional to the sample count. Construct with [`TrackCursor::new`],
/// which validates the tables up front, then iterate it one [`SampleRef`] at a time.
#[derive(Debug)]
pub struct TrackCursor<'a> {
    track: TrackId,
    stts: &'a TimeToSampleBox,
    ctts: Option<&'a CompositionOffsetBox>,
    stsc: &'a SampleToChunkBox,
    size_table: SizeTable<'a>,
    chunk_offsets: ChunkOffsets<'a>,
    stss: Option<&'a SyncSampleBox>,
    total_samples: u32,
    file_len: Option<u64>,
    pos: Pos,
    done: bool,
}

impl<'a> TrackCursor<'a> {
    /// Resolve and validate one track's sample tables.
    ///
    /// `file_len`, when known, bounds every sample range and the aggregate
    /// sample bytes; pass `Some` for a whole-file read, `None` for a stream
    /// whose length is unknown.
    pub fn new(
        stbl: &'a SampleTableBox,
        track: TrackId,
        limits: &Limits,
        file_len: Option<u64>,
    ) -> Result<Self> {
        let children = &stbl.children;
        let stts = find_child(
            children,
            b"stts",
            |c| match c {
                StblChild::Stts(b) => Some(b),
                _ => None,
            },
            |b| TimeToSampleBox::parse(b),
        )?
        .ok_or(Error::UnexpectedBox { expected: "stts" })?;
        let ctts = find_child(
            children,
            b"ctts",
            |c| match c {
                StblChild::Ctts(b) => Some(b),
                _ => None,
            },
            |b| CompositionOffsetBox::parse(b),
        )?;
        let stss = find_child(
            children,
            b"stss",
            |c| match c {
                StblChild::Stss(b) => Some(b),
                _ => None,
            },
            |b| SyncSampleBox::parse(b),
        )?;
        let stsz = find_child(
            children,
            b"stsz",
            |c| match c {
                StblChild::Stsz(b) => Some(b),
                _ => None,
            },
            |b| SampleSizeBox::parse(b),
        )?;
        let stz2 = find_child(
            children,
            b"stz2",
            |c| match c {
                StblChild::Stz2(b) => Some(b),
                _ => None,
            },
            |b| CompactSampleSizeBox::parse(b),
        )?;
        let size_table = match (stsz, stz2) {
            (Some(b), None) => SizeTable::Stsz(b),
            (None, Some(b)) => SizeTable::Stz2(b),
            (None, None) => {
                return Err(Error::UnexpectedBox {
                    expected: "stsz or stz2",
                });
            }
            (Some(_), Some(_)) => {
                return Err(Error::InvalidInput("both stsz and stz2 are present"));
            }
        };
        let stsc = find_child(
            children,
            b"stsc",
            |c| match c {
                StblChild::Stsc(b) => Some(b),
                _ => None,
            },
            |b| SampleToChunkBox::parse(b),
        )?
        .ok_or(Error::UnexpectedBox { expected: "stsc" })?;
        let co64 = find_child(
            children,
            b"co64",
            |c| match c {
                StblChild::Co64(b) => Some(b),
                _ => None,
            },
            |b| ChunkLargeOffsetBox::parse(b),
        )?;
        let stco = find_child(
            children,
            b"stco",
            |c| match c {
                StblChild::Stco(b) => Some(b),
                _ => None,
            },
            |b| ChunkOffsetBox::parse(b),
        )?;
        let chunk_offsets = match (co64, stco) {
            (Some(b), _) => ChunkOffsets::Co64(b),
            (None, Some(b)) => ChunkOffsets::Stco(b),
            (None, None) => {
                return Err(Error::UnexpectedBox {
                    expected: "stco or co64",
                });
            }
        };

        let total_samples = size_table.count();
        if let Some(len) = file_len
            && u64::from(total_samples) > len
        {
            return Err(Error::BufferTooShort(BufferTooShort {
                need: total_samples as usize,
                have: len as usize,
                what: "stsz sample_count vs file length",
            }));
        }
        if u64::from(total_samples) > u64::from(limits.max_samples_per_track()) {
            return Err(Error::LimitExceeded {
                what: "samples per track",
                limit: u64::from(limits.max_samples_per_track()),
            });
        }
        size_table.walk_sizes(total_samples, |size| {
            if size > limits.max_sample_bytes() {
                Err(Error::LimitExceeded {
                    what: "sample size",
                    limit: u64::from(limits.max_sample_bytes()),
                })
            } else {
                Ok(())
            }
        })?;
        validate_stsc(&stsc.entries)?;
        let needed = chunks_needed(&stsc.entries, total_samples)?;
        if u64::from(needed) > chunk_offsets.len() as u64 {
            return Err(Error::InvalidInput(
                "stsc/stco sample-to-chunk mapping did not cover all samples",
            ));
        }

        let mut cursor = Self {
            track,
            stts,
            ctts,
            stsc,
            size_table,
            chunk_offsets,
            stss,
            total_samples,
            file_len,
            pos: Pos {
                sample: 0,
                decode_time: 0,
                total_bytes: 0,
                stts_run: 0,
                stts_left: 0,
                ctts_run: 0,
                ctts_left: 0,
                chunk_no: 0,
                chunk_remaining: 0,
                chunk_cursor: 0,
                stsc_run: 0,
                stss_pos: 0,
            },
            done: false,
        };
        cursor.pos = cursor.reset()?;
        Ok(cursor)
    }

    /// Samples not yet yielded.
    pub fn len(&self) -> u32 {
        self.total_samples.saturating_sub(self.pos.sample)
    }

    /// Whether every sample has been yielded.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The decode time of the next sample, without consuming it.
    pub fn peek_decode_time(&self) -> Option<u64> {
        (self.pos.sample < self.total_samples).then_some(self.pos.decode_time)
    }

    /// Position on the last sync sample whose decode time is at or before
    /// `decode_time`; the first sample when none is earlier.
    ///
    /// Walks the tables run by run and allocates nothing per sample.
    pub fn seek_sync(&mut self, decode_time: u64) -> Result<()> {
        let start = self.reset()?;
        self.pos = start;
        self.done = false;
        let mut best = None;
        while self.pos.sample < self.total_samples {
            let sample = self.sample_ref()?;
            if sample.decode_time > decode_time {
                break;
            }
            if sample.sync {
                best = Some(self.pos);
            }
            self.advance()?;
        }
        self.pos = best.unwrap_or(start);
        self.done = false;
        Ok(())
    }

    fn reset(&self) -> Result<Pos> {
        let mut pos = Pos {
            sample: 0,
            decode_time: 0,
            total_bytes: 0,
            stts_run: 0,
            stts_left: self.stts.entries.first().map_or(0, |e| e.sample_count),
            ctts_run: 0,
            ctts_left: self
                .ctts
                .and_then(|c| c.entries.first())
                .map_or(0, |e| e.sample_count),
            chunk_no: 0,
            chunk_remaining: 0,
            chunk_cursor: 0,
            stsc_run: 0,
            stss_pos: 0,
        };
        self.advance_stts_run(&mut pos);
        self.advance_ctts_run(&mut pos);
        if self.total_samples > 0 {
            self.open_next_chunk(&mut pos)?;
        }
        if let Some(stss) = self.stss {
            while pos.stss_pos < stss.entries.len() && stss.entries[pos.stss_pos] < SYNC_INDEX_BIAS
            {
                pos.stss_pos += 1;
            }
        }
        Ok(pos)
    }

    fn advance_stts_run(&self, pos: &mut Pos) {
        while pos.stts_left == 0 {
            if pos.stts_run >= self.stts.entries.len() {
                break;
            }
            pos.stts_run += 1;
            pos.stts_left = self
                .stts
                .entries
                .get(pos.stts_run)
                .map_or(0, |e| e.sample_count);
        }
    }

    fn advance_ctts_run(&self, pos: &mut Pos) {
        let Some(ctts) = self.ctts else {
            return;
        };
        while pos.ctts_left == 0 {
            if pos.ctts_run >= ctts.entries.len() {
                break;
            }
            pos.ctts_run += 1;
            pos.ctts_left = ctts.entries.get(pos.ctts_run).map_or(0, |e| e.sample_count);
        }
    }

    fn open_next_chunk(&self, pos: &mut Pos) -> Result<()> {
        loop {
            pos.chunk_no = pos
                .chunk_no
                .checked_add(1)
                .ok_or(Error::InvalidInput("chunk index overflow"))?;
            if pos.chunk_no as usize > self.chunk_offsets.len() {
                return Err(Error::InvalidInput(
                    "stsc/stco sample-to-chunk mapping did not cover all samples",
                ));
            }
            while pos.stsc_run + 1 < self.stsc.entries.len()
                && u64::from(self.stsc.entries[pos.stsc_run + 1].first_chunk)
                    <= u64::from(pos.chunk_no)
            {
                pos.stsc_run += 1;
            }
            let per_chunk = match self.stsc.entries.get(pos.stsc_run) {
                Some(e) if e.first_chunk <= pos.chunk_no => e.samples_per_chunk,
                _ => 0,
            };
            if per_chunk != 0 {
                pos.chunk_remaining = per_chunk;
                pos.chunk_cursor = self.chunk_offsets.get(pos.chunk_no - 1);
                return Ok(());
            }
        }
    }

    fn current_duration(&self) -> Result<u32> {
        self.stts
            .entries
            .get(self.pos.stts_run)
            .map(|e| e.sample_delta)
            .ok_or(Error::InvalidInput(
                "stts sample count does not match chunk layout",
            ))
    }

    fn current_composition_offset(&self) -> Result<i32> {
        let Some(ctts) = self.ctts else {
            return Ok(0);
        };
        ctts.entries
            .get(self.pos.ctts_run)
            .map(|e| e.sample_offset)
            .ok_or(Error::InvalidInput(
                "ctts sample count does not match chunk layout",
            ))
    }

    fn current_description_index(&self) -> u32 {
        match self.stsc.entries.get(self.pos.stsc_run) {
            Some(e) if e.first_chunk <= self.pos.chunk_no => e.sample_description_index,
            _ => DEFAULT_SAMPLE_DESCRIPTION_INDEX,
        }
    }

    fn current_sync(&self) -> bool {
        let Some(stss) = self.stss else {
            return true;
        };
        let Some(index) = self.pos.sample.checked_add(SYNC_INDEX_BIAS) else {
            return false;
        };
        stss.entries.get(self.pos.stss_pos).copied() == Some(index)
    }

    fn sample_ref(&self) -> Result<SampleRef> {
        let size = self.size_table.size(self.pos.sample)?;
        let offset = self.pos.chunk_cursor;
        Ok(SampleRef {
            track: self.track,
            index: self.pos.sample,
            decode_time: self.pos.decode_time,
            composition_offset: self.current_composition_offset()?,
            duration: self.current_duration()?,
            sync: self.current_sync(),
            offset,
            size,
            description_index: self.current_description_index(),
        })
    }

    fn advance(&mut self) -> Result<()> {
        let mut pos = self.pos;
        let size = self.size_table.size(pos.sample)?;
        let duration = self.current_duration()?;
        let limit = self
            .file_len
            .map_or(usize::MAX, |len| usize::try_from(len).unwrap_or(usize::MAX));
        pos.total_bytes = pos
            .total_bytes
            .checked_add(u64::from(size))
            .ok_or(Error::SampleDataBudgetExceeded { limit })?;
        if let Some(len) = self.file_len {
            if pos.total_bytes > len {
                return Err(Error::SampleDataBudgetExceeded { limit });
            }
            let end = pos
                .chunk_cursor
                .checked_add(u64::from(size))
                .ok_or(Error::InvalidInput("sample byte range overflow"))?;
            if end > len {
                return Err(Error::BufferTooShort(BufferTooShort {
                    need: usize::try_from(end).unwrap_or(usize::MAX),
                    have: usize::try_from(len).unwrap_or(usize::MAX),
                    what: "progressive sample data",
                }));
            }
        }
        pos.decode_time = pos
            .decode_time
            .checked_add(u64::from(duration))
            .ok_or(Error::InvalidInput("decode time overflow"))?;
        pos.sample += 1;
        if pos.stts_left > 0 {
            pos.stts_left -= 1;
        }
        self.advance_stts_run(&mut pos);
        if self.ctts.is_some() {
            if pos.ctts_left > 0 {
                pos.ctts_left -= 1;
            }
            self.advance_ctts_run(&mut pos);
        }
        pos.chunk_remaining = pos.chunk_remaining.saturating_sub(1);
        pos.chunk_cursor = pos
            .chunk_cursor
            .checked_add(u64::from(size))
            .ok_or(Error::InvalidInput("chunk cursor overflow"))?;
        if let Some(stss) = self.stss {
            let next_index = pos.sample.saturating_add(SYNC_INDEX_BIAS);
            while pos.stss_pos < stss.entries.len() && stss.entries[pos.stss_pos] < next_index {
                pos.stss_pos += 1;
            }
        }
        if pos.chunk_remaining == 0 && pos.sample < self.total_samples {
            self.open_next_chunk(&mut pos)?;
        }
        self.pos = pos;
        Ok(())
    }

    fn stts_remainder(&self) -> u64 {
        let mut remainder = u64::from(self.pos.stts_left);
        for entry in self
            .stts
            .entries
            .iter()
            .skip(self.pos.stts_run.saturating_add(1))
        {
            remainder = remainder.saturating_add(u64::from(entry.sample_count));
        }
        remainder
    }

    fn ctts_remainder(&self) -> u64 {
        let Some(ctts) = self.ctts else {
            return 0;
        };
        let mut remainder = u64::from(self.pos.ctts_left);
        for entry in ctts
            .entries
            .iter()
            .skip(self.pos.ctts_run.saturating_add(1))
        {
            remainder = remainder.saturating_add(u64::from(entry.sample_count));
        }
        remainder
    }
}

impl Iterator for TrackCursor<'_> {
    type Item = Result<SampleRef>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        if self.pos.sample >= self.total_samples {
            self.done = true;
            if self.stts_remainder() != 0 {
                return Some(Err(Error::InvalidInput(
                    "stts sample count does not match chunk layout",
                )));
            }
            if self.ctts_remainder() != 0 {
                return Some(Err(Error::InvalidInput(
                    "ctts sample count does not match chunk layout",
                )));
            }
            return None;
        }
        match self.sample_ref() {
            Ok(sample) => match self.advance() {
                Ok(()) => Some(Ok(sample)),
                Err(err) => {
                    self.done = true;
                    Some(Err(err))
                }
            },
            Err(err) => {
                self.done = true;
                Some(Err(err))
            }
        }
    }
}

/// One-based `stsd` index used when no `stsc` run names the current chunk.
const DEFAULT_SAMPLE_DESCRIPTION_INDEX: u32 = 1;

/// Validate `stsc`'s run table (§8.7.4): `first_chunk` starts at 1 and rises
/// strictly, and every run names a real `stsd` entry.
fn validate_stsc(entries: &[StscEntry]) -> Result<()> {
    let mut previous = 0u32;
    for entry in entries {
        if entry.first_chunk <= previous {
            return Err(Error::InvalidValue(broadcast_common::error::InvalidValue {
                field: "stsc.first_chunk",
                value: u64::from(entry.first_chunk),
                reason: "first_chunk must be >= 1 and strictly ascending (ISO/IEC 14496-12:2026 §8.7.4)",
            }));
        }
        if entry.sample_description_index == 0 {
            return Err(Error::InvalidValue(broadcast_common::error::InvalidValue {
                field: "stsc.sample_description_index",
                value: 0,
                reason: "sample_description_index is one-based (ISO/IEC 14496-12:2026 §8.7.4)",
            }));
        }
        previous = entry.first_chunk;
    }
    Ok(())
}

/// The highest one-based chunk index the `stsc` runs need to cover
/// `total_samples` samples (§8.7.4).
fn chunks_needed(entries: &[StscEntry], total_samples: u32) -> Result<u32> {
    if total_samples == 0 {
        return Ok(0);
    }
    let mut delivered = 0u64;
    for (index, entry) in entries.iter().enumerate() {
        let per_chunk = u64::from(entry.samples_per_chunk);
        if per_chunk == 0 {
            continue;
        }
        let run_first = u64::from(entry.first_chunk);
        let next_first = entries
            .get(index + 1)
            .map_or(u64::MAX, |next| u64::from(next.first_chunk));
        let span = next_first.saturating_sub(run_first);
        let capacity = span.saturating_mul(per_chunk);
        let remaining = u64::from(total_samples).saturating_sub(delivered);
        if capacity >= remaining {
            let chunks = remaining.div_ceil(per_chunk);
            let needed = run_first.saturating_add(chunks).saturating_sub(1);
            return Ok(u32::try_from(needed).unwrap_or(u32::MAX));
        }
        delivered = delivered.saturating_add(capacity);
    }
    Err(Error::InvalidInput(
        "stsc/stco sample-to-chunk mapping did not cover all samples",
    ))
}

/// Find a typed `stbl` child, recovering the real parse error from a same
/// four-CC [`StblChild::Opaque`] (a box that was present but failed to parse).
fn find_child<'a, T>(
    children: &'a [StblChild],
    fourcc: &[u8; BOX_TYPE_SIZE],
    get: impl Fn(&'a StblChild) -> Option<&'a T>,
    parse: impl Fn(&[u8]) -> Result<T>,
) -> Result<Option<&'a T>> {
    for child in children {
        if let Some(value) = get(child) {
            return Ok(Some(value));
        }
    }
    for child in children {
        if let StblChild::Opaque(bytes) = child
            && bytes.len() >= BOX_HEADER_MIN_SIZE
            && &bytes[BOX_TYPE_SIZE..BOX_HEADER_MIN_SIZE] == fourcc
        {
            parse(bytes)?;
        }
    }
    Ok(None)
}

/// One-based `stsd` entry the builders place on every new `stsc` run.
const SAMPLE_DESCRIPTION_INDEX: u32 = 1;

/// Run-length code a per-sample decode-delta list into `stts` entries
/// (ISO/IEC 14496-12:2026 §8.6.1.2): consecutive equal deltas collapse into a
/// single `(sample_count, sample_delta)` run.
#[must_use]
pub fn build_stts(deltas: &[u32]) -> TimeToSampleBox {
    let mut entries: Vec<crate::timing::SttsEntry> = Vec::new();
    for &delta in deltas {
        match entries.last_mut() {
            Some(last) if last.sample_delta == delta => last.sample_count += 1,
            _ => entries.push(crate::timing::SttsEntry {
                sample_count: 1,
                sample_delta: delta,
            }),
        }
    }
    TimeToSampleBox {
        version: 0,
        flags: 0,
        entries,
    }
}

/// Run-length code a per-sample composition-offset list into `ctts` entries
/// (§8.6.1.3), or `None` when every offset is zero and the box is omitted.
/// Writes version 1 so a negative B-frame offset survives.
#[must_use]
pub fn build_ctts(offsets: &[i32]) -> Option<CompositionOffsetBox> {
    let mut entries: Vec<crate::timing::CttsEntry> = Vec::new();
    for &offset in offsets {
        match entries.last_mut() {
            Some(last) if last.sample_offset == offset => last.sample_count += 1,
            _ => entries.push(crate::timing::CttsEntry {
                sample_count: 1,
                sample_offset: offset,
            }),
        }
    }
    if entries.iter().all(|entry| entry.sample_offset == 0) {
        return None;
    }
    Some(CompositionOffsetBox {
        version: 1,
        flags: 0,
        entries,
    })
}

/// Build the `stss` sync-sample list of 1-based indices (§8.6.2), or `None`
/// when every sample is a sync sample and the box is omitted.
#[must_use]
pub fn build_stss(sync: &[bool]) -> Option<SyncSampleBox> {
    if sync.iter().all(|&is_sync| is_sync) {
        return None;
    }
    let entries: Vec<u32> = sync
        .iter()
        .enumerate()
        .filter_map(|(index, &is_sync)| {
            is_sync
                .then(|| u32::try_from(index.checked_add(1)?).ok())
                .flatten()
        })
        .collect();
    Some(SyncSampleBox {
        version: 0,
        flags: 0,
        entries,
    })
}

/// Run-length code a per-chunk sample count into `stsc` runs (§8.7.4): one
/// entry per distinct count, keyed by the one-based first chunk it applies to.
pub fn build_stsc(samples_per_chunk: &[u32]) -> Result<SampleToChunkBox> {
    let mut entries: Vec<StscEntry> = Vec::new();
    for (index, &count) in samples_per_chunk.iter().enumerate() {
        let first_chunk = broadcast_common::len::fit_u32(index.saturating_add(1), "first_chunk")?;
        match entries.last_mut() {
            Some(last) if last.samples_per_chunk == count => {}
            _ => entries.push(StscEntry {
                first_chunk,
                samples_per_chunk: count,
                sample_description_index: SAMPLE_DESCRIPTION_INDEX,
            }),
        }
    }
    Ok(SampleToChunkBox {
        version: 0,
        flags: 0,
        entries,
    })
}

#[cfg(test)]
mod builder_tests {
    use super::*;

    #[test]
    fn build_stts_run_length_codes_consecutive_equal_deltas() {
        let stts = build_stts(&[10, 10, 20, 10]);
        let entries: alloc::vec::Vec<(u32, u32)> = stts
            .entries
            .iter()
            .map(|e| (e.sample_count, e.sample_delta))
            .collect();
        assert_eq!(entries, alloc::vec![(2, 10), (1, 20), (1, 10)]);
    }

    #[test]
    fn build_ctts_omits_an_all_zero_table() {
        assert!(build_ctts(&[0, 0, 0]).is_none());
        let ctts = build_ctts(&[0, 5, 5, -3]).expect("a non-zero offset keeps the box");
        assert_eq!(ctts.version, 1);
        let entries: alloc::vec::Vec<(u32, i32)> = ctts
            .entries
            .iter()
            .map(|e| (e.sample_count, e.sample_offset))
            .collect();
        assert_eq!(entries, alloc::vec![(1, 0), (2, 5), (1, -3)]);
    }

    #[test]
    fn build_stss_omits_an_all_sync_table() {
        assert!(build_stss(&[true, true, true]).is_none());
        let stss = build_stss(&[true, false, true]).expect("a non-sync sample keeps the box");
        assert_eq!(stss.entries, alloc::vec![1, 3]);
    }

    #[test]
    fn build_stsc_run_length_codes_equal_chunk_sizes() {
        let stsc = build_stsc(&[5, 5, 3, 5]).expect("chunk counts fit");
        let entries: alloc::vec::Vec<(u32, u32, u32)> = stsc
            .entries
            .iter()
            .map(|e| {
                (
                    e.first_chunk,
                    e.samples_per_chunk,
                    e.sample_description_index,
                )
            })
            .collect();
        assert_eq!(entries, alloc::vec![(1, 5, 1), (3, 3, 1), (4, 5, 1)]);
    }
}
