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

/// The iteration state, small enough to snapshot for [`CursorState::seek_sync`].
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

/// Where one track's typed sample-table children sit inside its
/// [`SampleTableBox`], resolved once by [`CursorState::new`].
#[derive(Debug, Clone, Copy)]
struct Layout {
    stts: usize,
    ctts: Option<usize>,
    stsc: usize,
    stsz: Option<usize>,
    stz2: Option<usize>,
    stco: Option<usize>,
    co64: Option<usize>,
    stss: Option<usize>,
}

impl Layout {
    fn resolve(children: &[StblChild]) -> Result<Self> {
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
        match (stsz, stz2) {
            (Some(_), Some(_)) => {
                return Err(Error::InvalidInput("both stsz and stz2 are present"));
            }
            (None, None) => {
                return Err(Error::UnexpectedBox {
                    expected: "stsz or stz2",
                });
            }
            _ => {}
        }
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
        if co64.is_none() && stco.is_none() {
            return Err(Error::UnexpectedBox {
                expected: "stco or co64",
            });
        }
        Ok(Self {
            stts,
            ctts,
            stsc,
            stsz,
            stz2,
            stco,
            co64,
            stss,
        })
    }

    fn stts<'a>(&self, stbl: &'a SampleTableBox) -> Result<&'a TimeToSampleBox> {
        match stbl.children.get(self.stts) {
            Some(StblChild::Stts(b)) => Ok(b),
            _ => Err(Error::InvalidInput("sample table changed under its cursor")),
        }
    }

    fn ctts<'a>(&self, stbl: &'a SampleTableBox) -> Result<Option<&'a CompositionOffsetBox>> {
        match self.ctts {
            None => Ok(None),
            Some(index) => match stbl.children.get(index) {
                Some(StblChild::Ctts(b)) => Ok(Some(b)),
                _ => Err(Error::InvalidInput("sample table changed under its cursor")),
            },
        }
    }

    fn stsc<'a>(&self, stbl: &'a SampleTableBox) -> Result<&'a SampleToChunkBox> {
        match stbl.children.get(self.stsc) {
            Some(StblChild::Stsc(b)) => Ok(b),
            _ => Err(Error::InvalidInput("sample table changed under its cursor")),
        }
    }

    fn stss<'a>(&self, stbl: &'a SampleTableBox) -> Result<Option<&'a SyncSampleBox>> {
        match self.stss {
            None => Ok(None),
            Some(index) => match stbl.children.get(index) {
                Some(StblChild::Stss(b)) => Ok(Some(b)),
                _ => Err(Error::InvalidInput("sample table changed under its cursor")),
            },
        }
    }

    fn size_table<'a>(&self, stbl: &'a SampleTableBox) -> Result<SizeTable<'a>> {
        match (self.stsz, self.stz2) {
            (Some(index), None) => match stbl.children.get(index) {
                Some(StblChild::Stsz(b)) => Ok(SizeTable::Stsz(b)),
                _ => Err(Error::InvalidInput("sample table changed under its cursor")),
            },
            (None, Some(index)) => match stbl.children.get(index) {
                Some(StblChild::Stz2(b)) => Ok(SizeTable::Stz2(b)),
                _ => Err(Error::InvalidInput("sample table changed under its cursor")),
            },
            _ => Err(Error::InvalidInput("sample table changed under its cursor")),
        }
    }

    fn chunk_offsets<'a>(&self, stbl: &'a SampleTableBox) -> Result<ChunkOffsets<'a>> {
        if let Some(index) = self.co64 {
            match stbl.children.get(index) {
                Some(StblChild::Co64(b)) => return Ok(ChunkOffsets::Co64(b)),
                _ => return Err(Error::InvalidInput("sample table changed under its cursor")),
            }
        }
        match self.stco {
            Some(index) => match stbl.children.get(index) {
                Some(StblChild::Stco(b)) => Ok(ChunkOffsets::Stco(b)),
                _ => Err(Error::InvalidInput("sample table changed under its cursor")),
            },
            None => Err(Error::InvalidInput("sample table changed under its cursor")),
        }
    }
}

/// One track's position in its sample tables, as plain integers.
///
/// A reader that owns its `moov` bytes keeps one of these per track and lends
/// it the parsed [`SampleTableBox`] on every step, so no cursor borrows from
/// the structure it walks. [`TrackCursor`] is this same state paired with the
/// table it was built from; there is one implementation of the walk.
#[derive(Debug, Clone, Copy)]
pub struct CursorState {
    track: TrackId,
    layout: Layout,
    total_samples: u32,
    file_len: Option<u64>,
    pos: Pos,
    done: bool,
}

impl CursorState {
    /// Resolve and validate one track's sample tables.
    ///
    /// `file_len`, when known, bounds every sample range and the aggregate
    /// sample bytes; pass `Some` for a whole-file read, `None` for a stream
    /// whose length is unknown.
    pub fn new(
        stbl: &SampleTableBox,
        track: TrackId,
        limits: &Limits,
        file_len: Option<u64>,
    ) -> Result<Self> {
        let layout = Layout::resolve(&stbl.children)?;
        let stsc = layout.stsc(stbl)?;
        let size_table = layout.size_table(stbl)?;
        let chunk_offsets = layout.chunk_offsets(stbl)?;
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
        let mut state = Self {
            track,
            layout,
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
        state.pos = state.start(stbl)?;
        Ok(state)
    }

    /// The track this cursor walks.
    pub fn track(&self) -> TrackId {
        self.track
    }

    /// Samples not yet yielded.
    pub fn remaining(&self) -> u32 {
        self.total_samples.saturating_sub(self.pos.sample)
    }

    /// Whether every sample has been yielded.
    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    /// Zero-based index of the next sample.
    pub fn position(&self) -> u32 {
        self.pos.sample
    }

    /// The decode time of the next sample, without consuming it.
    pub fn peek_decode_time(&self) -> Option<u64> {
        (self.pos.sample < self.total_samples).then_some(self.pos.decode_time)
    }

    /// Position on the last sync sample whose decode time is at or before
    /// `decode_time`; the first sample when none is earlier.
    pub fn seek_sync(&mut self, stbl: &SampleTableBox, decode_time: u64) -> Result<()> {
        let start = self.start(stbl)?;
        self.pos = start;
        self.done = false;
        let mut best = None;
        while self.pos.sample < self.total_samples {
            let sample = self.sample_ref(stbl)?;
            if sample.decode_time > decode_time {
                break;
            }
            if sample.sync {
                best = Some(self.pos);
            }
            self.advance_inner(stbl)?;
        }
        self.pos = best.unwrap_or(start);
        self.done = false;
        Ok(())
    }

    /// The next sample descriptor, or `None` when the track is exhausted.
    pub fn peek(&mut self, stbl: &SampleTableBox) -> Result<Option<SampleRef>> {
        match self.peek_inner(stbl) {
            Ok(value) => Ok(value),
            Err(err) => {
                self.done = true;
                Err(err)
            }
        }
    }

    fn peek_inner(&mut self, stbl: &SampleTableBox) -> Result<Option<SampleRef>> {
        if self.done {
            return Ok(None);
        }
        if self.pos.sample >= self.total_samples {
            self.done = true;
            let stts = self.layout.stts(stbl)?;
            if self.stts_remainder(stts) != 0 {
                return Err(Error::InvalidInput(
                    "stts sample count does not match chunk layout",
                ));
            }
            if let Some(ctts) = self.layout.ctts(stbl)?
                && self.ctts_remainder(ctts) != 0
            {
                return Err(Error::InvalidInput(
                    "ctts sample count does not match chunk layout",
                ));
            }
            return Ok(None);
        }
        self.sample_ref(stbl).map(Some)
    }

    /// Consume the next sample, which must have been seen through
    /// [`CursorState::peek`].
    pub fn advance(&mut self, stbl: &SampleTableBox) -> Result<()> {
        match self.advance_inner(stbl) {
            Ok(()) => Ok(()),
            Err(err) => {
                self.done = true;
                Err(err)
            }
        }
    }

    fn start(&self, stbl: &SampleTableBox) -> Result<Pos> {
        let stts = self.layout.stts(stbl)?;
        let ctts = self.layout.ctts(stbl)?;
        let stss = self.layout.stss(stbl)?;
        let mut pos = Pos {
            sample: 0,
            decode_time: 0,
            total_bytes: 0,
            stts_run: 0,
            stts_left: stts.entries.first().map_or(0, |e| e.sample_count),
            ctts_run: 0,
            ctts_left: ctts
                .and_then(|c| c.entries.first())
                .map_or(0, |e| e.sample_count),
            chunk_no: 0,
            chunk_remaining: 0,
            chunk_cursor: 0,
            stsc_run: 0,
            stss_pos: 0,
        };
        self.advance_stts_run(stts, &mut pos);
        if let Some(ctts) = ctts {
            self.advance_ctts_run(ctts, &mut pos);
        }
        if self.total_samples > 0 {
            self.open_next_chunk(stbl, &mut pos)?;
        }
        if let Some(stss) = stss {
            while pos.stss_pos < stss.entries.len() && stss.entries[pos.stss_pos] < SYNC_INDEX_BIAS
            {
                pos.stss_pos += 1;
            }
        }
        Ok(pos)
    }

    fn advance_stts_run(&self, stts: &TimeToSampleBox, pos: &mut Pos) {
        while pos.stts_left == 0 {
            if pos.stts_run >= stts.entries.len() {
                break;
            }
            pos.stts_run += 1;
            pos.stts_left = stts.entries.get(pos.stts_run).map_or(0, |e| e.sample_count);
        }
    }

    fn advance_ctts_run(&self, ctts: &CompositionOffsetBox, pos: &mut Pos) {
        while pos.ctts_left == 0 {
            if pos.ctts_run >= ctts.entries.len() {
                break;
            }
            pos.ctts_run += 1;
            pos.ctts_left = ctts.entries.get(pos.ctts_run).map_or(0, |e| e.sample_count);
        }
    }

    fn open_next_chunk(&self, stbl: &SampleTableBox, pos: &mut Pos) -> Result<()> {
        let stsc = self.layout.stsc(stbl)?;
        let chunk_offsets = self.layout.chunk_offsets(stbl)?;
        loop {
            pos.chunk_no = pos
                .chunk_no
                .checked_add(1)
                .ok_or(Error::InvalidInput("chunk index overflow"))?;
            if pos.chunk_no as usize > chunk_offsets.len() {
                return Err(Error::InvalidInput(
                    "stsc/stco sample-to-chunk mapping did not cover all samples",
                ));
            }
            while pos.stsc_run + 1 < stsc.entries.len()
                && u64::from(stsc.entries[pos.stsc_run + 1].first_chunk) <= u64::from(pos.chunk_no)
            {
                pos.stsc_run += 1;
            }
            let per_chunk = match stsc.entries.get(pos.stsc_run) {
                Some(e) if e.first_chunk <= pos.chunk_no => e.samples_per_chunk,
                _ => 0,
            };
            if per_chunk != 0 {
                pos.chunk_remaining = per_chunk;
                pos.chunk_cursor = chunk_offsets.get(pos.chunk_no - 1);
                return Ok(());
            }
        }
    }

    fn current_duration(&self, stbl: &SampleTableBox) -> Result<u32> {
        self.layout
            .stts(stbl)?
            .entries
            .get(self.pos.stts_run)
            .map(|e| e.sample_delta)
            .ok_or(Error::InvalidInput(
                "stts sample count does not match chunk layout",
            ))
    }

    fn current_composition_offset(&self, stbl: &SampleTableBox) -> Result<i32> {
        let Some(ctts) = self.layout.ctts(stbl)? else {
            return Ok(0);
        };
        ctts.entries
            .get(self.pos.ctts_run)
            .map(|e| e.sample_offset)
            .ok_or(Error::InvalidInput(
                "ctts sample count does not match chunk layout",
            ))
    }

    fn current_description_index(&self, stbl: &SampleTableBox) -> Result<u32> {
        let stsc = self.layout.stsc(stbl)?;
        Ok(match stsc.entries.get(self.pos.stsc_run) {
            Some(e) if e.first_chunk <= self.pos.chunk_no => e.sample_description_index,
            _ => DEFAULT_SAMPLE_DESCRIPTION_INDEX,
        })
    }

    fn current_sync(&self, stbl: &SampleTableBox) -> Result<bool> {
        let Some(stss) = self.layout.stss(stbl)? else {
            return Ok(true);
        };
        let Some(index) = self.pos.sample.checked_add(SYNC_INDEX_BIAS) else {
            return Ok(false);
        };
        Ok(stss.entries.get(self.pos.stss_pos).copied() == Some(index))
    }

    fn sample_ref(&self, stbl: &SampleTableBox) -> Result<SampleRef> {
        let size = self.layout.size_table(stbl)?.size(self.pos.sample)?;
        let offset = self.pos.chunk_cursor;
        Ok(SampleRef {
            track: self.track,
            index: self.pos.sample,
            decode_time: self.pos.decode_time,
            composition_offset: self.current_composition_offset(stbl)?,
            duration: self.current_duration(stbl)?,
            sync: self.current_sync(stbl)?,
            offset,
            size,
            description_index: self.current_description_index(stbl)?,
        })
    }

    fn advance_inner(&mut self, stbl: &SampleTableBox) -> Result<()> {
        let size_table = self.layout.size_table(stbl)?;
        let stts = self.layout.stts(stbl)?;
        let ctts = self.layout.ctts(stbl)?;
        let stss = self.layout.stss(stbl)?;
        let mut pos = self.pos;
        let size = size_table.size(pos.sample)?;
        let duration = self.current_duration(stbl)?;
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
        self.advance_stts_run(stts, &mut pos);
        if let Some(ctts) = ctts {
            if pos.ctts_left > 0 {
                pos.ctts_left -= 1;
            }
            self.advance_ctts_run(ctts, &mut pos);
        }
        pos.chunk_remaining = pos.chunk_remaining.saturating_sub(1);
        pos.chunk_cursor = pos
            .chunk_cursor
            .checked_add(u64::from(size))
            .ok_or(Error::InvalidInput("chunk cursor overflow"))?;
        if let Some(stss) = stss {
            let next_index = pos.sample.saturating_add(SYNC_INDEX_BIAS);
            while pos.stss_pos < stss.entries.len() && stss.entries[pos.stss_pos] < next_index {
                pos.stss_pos += 1;
            }
        }
        if pos.chunk_remaining == 0 && pos.sample < self.total_samples {
            self.open_next_chunk(stbl, &mut pos)?;
        }
        self.pos = pos;
        Ok(())
    }

    fn stts_remainder(&self, stts: &TimeToSampleBox) -> u64 {
        let mut remainder = u64::from(self.pos.stts_left);
        for entry in stts
            .entries
            .iter()
            .skip(self.pos.stts_run.saturating_add(1))
        {
            remainder = remainder.saturating_add(u64::from(entry.sample_count));
        }
        remainder
    }

    fn ctts_remainder(&self, ctts: &CompositionOffsetBox) -> u64 {
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

/// One track's sample tables, walked lazily in decode order.
///
/// Borrows the typed boxes out of [`SampleTableBox`]; it allocates nothing
/// proportional to the sample count. Construct with [`TrackCursor::new`],
/// which validates the tables up front, then iterate it one [`SampleRef`] at a time.
#[derive(Debug)]
pub struct TrackCursor<'a> {
    stbl: &'a SampleTableBox,
    state: CursorState,
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
        Ok(Self {
            stbl,
            state: CursorState::new(stbl, track, limits, file_len)?,
        })
    }

    /// Samples not yet yielded.
    pub fn len(&self) -> u32 {
        self.state.remaining()
    }

    /// Whether every sample has been yielded.
    pub fn is_empty(&self) -> bool {
        self.state.is_empty()
    }

    /// The decode time of the next sample, without consuming it.
    pub fn peek_decode_time(&self) -> Option<u64> {
        self.state.peek_decode_time()
    }

    /// Position on the last sync sample whose decode time is at or before
    /// `decode_time`; the first sample when none is earlier.
    ///
    /// Walks the tables run by run and allocates nothing per sample.
    pub fn seek_sync(&mut self, decode_time: u64) -> Result<()> {
        self.state.seek_sync(self.stbl, decode_time)
    }
}

impl Iterator for TrackCursor<'_> {
    type Item = Result<SampleRef>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.state.peek(self.stbl) {
            Ok(None) => None,
            Ok(Some(sample)) => match self.state.advance(self.stbl) {
                Ok(()) => Some(Ok(sample)),
                Err(err) => Some(Err(err)),
            },
            Err(err) => Some(Err(err)),
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
fn find_child<T>(
    children: &[StblChild],
    fourcc: &[u8; BOX_TYPE_SIZE],
    get: impl Fn(&StblChild) -> Option<&T>,
    parse: impl Fn(&[u8]) -> Result<T>,
) -> Result<Option<usize>> {
    for (index, child) in children.iter().enumerate() {
        if get(child).is_some() {
            return Ok(Some(index));
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

/// Add one sample's decode delta to `stts` entries (ISO/IEC 14496-12:2026
/// §8.6.1.2): consecutive equal deltas collapse into a single
/// `(sample_count, sample_delta)` run.
///
/// The one run-length coder for `stts`: [`build_stts`] and
/// [`crate::plan::PlanBuilder`] both append through it.
pub(crate) fn append_stts(entries: &mut Vec<crate::timing::SttsEntry>, delta: u32) {
    match entries.last_mut() {
        Some(last) if last.sample_delta == delta => last.sample_count += 1,
        _ => entries.push(crate::timing::SttsEntry {
            sample_count: 1,
            sample_delta: delta,
        }),
    }
}

/// Add one sample's composition offset to `ctts` entries (§8.6.1.3), collapsing
/// an equal offset into the previous run.
pub(crate) fn append_ctts(entries: &mut Vec<crate::timing::CttsEntry>, offset: i32) {
    match entries.last_mut() {
        Some(last) if last.sample_offset == offset => last.sample_count += 1,
        _ => entries.push(crate::timing::CttsEntry {
            sample_count: 1,
            sample_offset: offset,
        }),
    }
}

/// Add one sample's sync flag to the `stss` entry list (§8.6.2), updating
/// `all_sync` so a table with no non-sync sample can omit the box.
pub(crate) fn append_stss(entries: &mut Vec<u32>, all_sync: &mut bool, index: u32, sync: bool) {
    if !sync {
        *all_sync = false;
        return;
    }
    if let Some(one_based) = index.checked_add(SYNC_INDEX_BIAS) {
        entries.push(one_based);
    }
}

/// Add one chunk to the `stsc` run list (§8.7.4), collapsing a chunk whose
/// sample count and description index equal the previous run.
pub(crate) fn append_stsc(
    entries: &mut Vec<StscEntry>,
    first_chunk: u32,
    samples_per_chunk: u32,
    sample_description_index: u32,
) {
    match entries.last() {
        Some(last)
            if last.samples_per_chunk == samples_per_chunk
                && last.sample_description_index == sample_description_index => {}
        _ => entries.push(StscEntry {
            first_chunk,
            samples_per_chunk,
            sample_description_index,
        }),
    }
}

/// Run-length code a per-sample decode-delta list into `stts` entries
/// (ISO/IEC 14496-12:2026 §8.6.1.2).
#[must_use]
pub fn build_stts(deltas: &[u32]) -> TimeToSampleBox {
    let mut entries: Vec<crate::timing::SttsEntry> = Vec::new();
    for &delta in deltas {
        append_stts(&mut entries, delta);
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
        append_ctts(&mut entries, offset);
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
    let mut entries: Vec<u32> = Vec::new();
    let mut all_sync = true;
    for (index, &is_sync) in sync.iter().enumerate() {
        if let Ok(index) = u32::try_from(index) {
            append_stss(&mut entries, &mut all_sync, index, is_sync);
        }
    }
    if all_sync {
        return None;
    }
    Some(SyncSampleBox {
        version: 0,
        flags: 0,
        entries,
    })
}

/// Run-length code a per-chunk sample count into `stsc` runs (§8.7.4): one
/// entry per distinct count, keyed by the one-based first chunk it applies to.
/// Every run names the first `stsd` entry.
pub fn build_stsc(samples_per_chunk: &[u32]) -> Result<SampleToChunkBox> {
    let descriptions: Vec<u32> = alloc::vec![SAMPLE_DESCRIPTION_INDEX; samples_per_chunk.len()];
    build_stsc_with_descriptions(samples_per_chunk, &descriptions)
}

/// Run-length code per-chunk sample counts and the `stsd` entry each chunk's
/// samples use into `stsc` runs (§8.7.4).
///
/// A plan read from a file keeps one description index per chunk, so a track
/// with several `stsd` entries writes back the runs it was read with.
pub fn build_stsc_with_descriptions(
    samples_per_chunk: &[u32],
    description_index: &[u32],
) -> Result<SampleToChunkBox> {
    if samples_per_chunk.len() != description_index.len() {
        return Err(Error::InvalidInput(
            "stsc builders need one description index per chunk",
        ));
    }
    let mut entries: Vec<StscEntry> = Vec::new();
    for (index, (&count, &description)) in
        samples_per_chunk.iter().zip(description_index).enumerate()
    {
        let first_chunk = broadcast_common::len::fit_u32(index.saturating_add(1), "first_chunk")?;
        append_stsc(&mut entries, first_chunk, count, description);
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
