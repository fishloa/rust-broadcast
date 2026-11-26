//! The progressive sample plan: a parsed `moov`'s per-track tables (lent to
//! [`TrackCursor`]), and the owned form a writer can outlive a reader with.
//!
//! `plan()` is the whole-file fact a progressive file already carries in its
//! index (streaming-first design §4.4, P8): sample timing, size and position
//! come from `stts`/`ctts`/`stsc`/`stsz`/`stco`, never from held samples.
//! [`SamplePlan`] borrows the parsed [`MovieBox`] a
//! [`Reader`](crate::reader::Reader) owns, so a cursor created here reads the
//! same tables the reader walks.
//!
//! [`OwnedPlan`] is the same fact with no borrow: per track the sample tables
//! in their compact wire form, the `stsd` entries, the `elst` entries, the
//! media and presentation durations and, when it came from a file, the source
//! chunk layout. [`PlanBuilder`] builds one from pushed sample descriptors,
//! run-length coding as it goes, for a caller that has samples and no index.
//! [`Writer::progressive`](crate::writer::Writer::progressive) consumes it.
//!
//! Spec: ISO/IEC 14496-12:2026 §8.5-§8.7
//! (`private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`).

use crate::box_support::{ChildOrder, OpaqueBox};
use crate::error::{Error, Result};
use crate::init_segment::{
    CompactSampleSizeBox, MovieBox, MovieExtendsBox, MovieHeaderBox, SampleTableBox,
    SampleToChunkBox, StblChild, SyncSampleBox, TrackBox,
};
use crate::limits::Limits;
use crate::reader::TrackInfo;
use crate::sample_table::{CursorState, TrackCursor, append_ctts, append_stss, append_stts};
use crate::timing::{CompositionOffsetBox, EditListEntry, TimeToSampleBox};
use crate::track_id::TrackId;
use crate::writer::{SampleEntry, TrackSetup};
use alloc::vec::Vec;

/// One-based `stsd` index a sample uses when no `stsc` run names its chunk.
pub const DEFAULT_DESCRIPTION_INDEX: u32 = 1;

/// A progressive file's sample tables, one cursor per track on demand.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct SamplePlan<'a> {
    movie: &'a MovieBox,
    tracks: &'a [TrackInfo],
    file_len: Option<u64>,
    top_level: &'a [TopLevelBox],
    progressive_iv_sizes: &'a [Option<u8>],
}

impl<'a> SamplePlan<'a> {
    pub(crate) fn new(
        movie: &'a MovieBox,
        tracks: &'a [TrackInfo],
        file_len: Option<u64>,
        top_level: &'a [TopLevelBox],
        progressive_iv_sizes: &'a [Option<u8>],
    ) -> Self {
        Self {
            movie,
            tracks,
            file_len,
            top_level,
            progressive_iv_sizes,
        }
    }

    /// Every track of the movie, in `moov` order.
    #[must_use]
    pub fn tracks(&self) -> &'a [TrackInfo] {
        self.tracks
    }

    /// A lazy cursor over one track's samples.
    ///
    /// `limits` bounds the sample count and each sample's size before the
    /// cursor walks anything.
    pub fn cursor(&self, track: TrackId, limits: &Limits) -> Result<TrackCursor<'a>> {
        let index = self
            .tracks
            .iter()
            .position(|info| info.track_id == track)
            .ok_or(Error::InvalidInput("unknown track"))?;
        let trak = self
            .movie
            .tracks
            .get(index)
            .ok_or(Error::UnexpectedBox { expected: "trak" })?;
        let stbl = sample_table(trak).ok_or(Error::UnexpectedBox { expected: "stbl" })?;
        let iv_size = self.progressive_iv_sizes.get(index).copied().flatten();
        TrackCursor::new_with_aux(stbl, track, limits, self.file_len, iv_size)
    }

    /// The plan in owned form, ready for
    /// [`Writer::progressive`](crate::writer::Writer::progressive).
    ///
    /// Every table is cloned in its compact wire form (no per-sample
    /// expansion), and the source `stsc`/chunk offsets are kept so
    /// [`Interleave::AsPlanned`](crate::writer::Interleave::AsPlanned) can
    /// reproduce the file's layout.
    pub fn to_owned_plan(&self) -> Result<OwnedPlan> {
        let limits = Limits::default();
        let mut tracks = Vec::with_capacity(self.tracks.len());
        for (index, info) in self.tracks.iter().enumerate() {
            let trak = self
                .movie
                .tracks
                .get(index)
                .ok_or(Error::UnexpectedBox { expected: "trak" })?;
            let stbl = sample_table(trak).ok_or(Error::UnexpectedBox { expected: "stbl" })?;
            let iv_size = self.progressive_iv_sizes.get(index).copied().flatten();
            let state =
                CursorState::new_with_aux(stbl, info.track_id, &limits, self.file_len, iv_size)?;
            let chunks = source_chunks(stbl)?;
            let tables = SampleTables {
                sample_count: state.remaining(),
                stts: stts_of(stbl)?,
                ctts: ctts_of(stbl),
                stss: stss_of(stbl),
                sizes: sizes_of(stbl)?,
                descriptions: description_runs(&chunks),
            };
            let entries = entries_of(info);
            let setup = track_setup(trak, info, &entries)?;
            tracks.push(PlannedTrack {
                setup,
                entries,
                edts_after_mdia: edts_after_mdia(trak),
                edits: info.edits.clone(),
                media_duration: media_duration(trak, info),
                presentation_duration: info.header.duration,
                start_decode_time: 0,
                tables,
                chunks: Some(chunks),
                shell: Some(trak.clone()),
            });
        }
        let shell = MovieShell {
            mvhd: self.movie.mvhd.clone(),
            mvex: self.movie.mvex.clone(),
            opaque: self.movie.opaque.clone(),
            order: self.movie.order.clone(),
            top_level: self.top_level.to_vec(),
        };
        Ok(OwnedPlan::new(tracks).with_shell(shell))
    }
}

/// A track's sample table, when it has one.
pub(crate) fn sample_table(trak: &TrackBox) -> Option<&SampleTableBox> {
    trak.mdia.as_ref()?.minf.as_ref()?.stbl.as_ref()
}

/// A track's typed sample description, when it has one.
pub(crate) fn sample_description(
    stbl: &SampleTableBox,
) -> Option<&crate::init_segment::SampleDescriptionBox> {
    stbl.children.iter().find_map(|child| match child {
        StblChild::Stsd(stsd) => Some(stsd),
        _ => None,
    })
}

/// Where a top-level box the writer does not model sat in the source file,
/// relative to the boxes the writer emits (`ftyp`, `moov`, `mdat`).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxPosition {
    /// Directly after `ftyp`, before the `moov`/`mdat` that follow.
    AfterFtyp,
    /// Directly after `moov`.
    AfterMoov,
    /// After the `mdat` payload.
    AfterMdat,
}

impl BoxPosition {
    /// The position's stable label.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::AfterFtyp => "after-ftyp",
            Self::AfterMoov => "after-moov",
            Self::AfterMdat => "after-mdat",
        }
    }
}

broadcast_core::impl_spec_display!(BoxPosition);

/// One top-level box outside `ftyp`/`moov`/`mdat` (`free`, `wide`, `uuid`,
/// `skip`, a trailing `mfra`), carried with its position so a rewrite
/// reproduces it.
#[derive(Debug, Clone, PartialEq)]
pub struct TopLevelBox {
    /// Where the box sat in the source file.
    pub position: BoxPosition,
    /// The box, kept as its opaque bytes.
    pub data: OpaqueBox,
}

/// The `moov` children a sample plan does not model, and the top-level boxes
/// around the movie.
///
/// A plan read from a file carries one so
/// [`Writer::progressive`](crate::writer::Writer::progressive) and
/// [`Writer::fragmented`](crate::writer::Writer::fragmented) can reproduce the
/// `udta`/`meta`, `mvex`, extra `trak` children and the boxes before or after
/// the movie. A plan a [`PlanBuilder`] made carries none, and the writer emits
/// exactly its own conventional layout.
#[derive(Debug, Clone, PartialEq)]
pub struct MovieShell {
    /// The movie header exactly as the source carried it (its duration and
    /// version are recomputed from the plan when written).
    pub mvhd: MovieHeaderBox,
    /// The movie-extends box (`mvex`), when the source was fragmented.
    pub mvex: Option<MovieExtendsBox>,
    /// `moov` children this crate does not model (`udta`, `meta`, `iods`).
    pub opaque: Vec<OpaqueBox>,
    /// The `moov` wire order the opaque children belong to.
    pub order: ChildOrder,
    /// The top-level boxes outside `ftyp`/`moov`/`mdat`, in source order.
    pub top_level: Vec<TopLevelBox>,
}

impl MovieShell {
    /// The carried top-level boxes at one position, in source order.
    pub fn top_level_at(&self, position: BoxPosition) -> impl Iterator<Item = &OpaqueBox> {
        self.top_level
            .iter()
            .filter(move |carried| carried.position == position)
            .map(|carried| &carried.data)
    }
}

/// One sample as [`PlanBuilder`] takes it (ISO/IEC 14496-12:2026 §8.6-§8.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanSample {
    /// Decode delta written to `stts`: the gap to the next sample's decode
    /// time, which is what the table can express (§8.6.1.2).
    pub duration: u32,
    /// `ctts` composition offset, `pts - dts` (§8.6.1.3).
    pub composition_offset: i32,
    /// `stss` membership (§8.6.2).
    pub sync: bool,
    /// `stsz` size in bytes (§8.7.3).
    pub size: u32,
    /// One-based `stsd` entry (`stsc.sample_description_index`, §8.7.4).
    pub description_index: u32,
}

impl PlanSample {
    /// A sync sample of the first `stsd` entry with no composition offset.
    #[must_use]
    pub const fn new(duration: u32, size: u32) -> Self {
        Self {
            duration,
            composition_offset: 0,
            sync: true,
            size,
            description_index: DEFAULT_DESCRIPTION_INDEX,
        }
    }
}

/// The first sample of a run that shares one `stsd` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescriptionRun {
    /// Zero-based index of the run's first sample.
    pub first_sample: u32,
    /// One-based `stsd` entry the run's samples use.
    pub description_index: u32,
}

/// A track's sample sizes: uniform, per sample, or the compact `stz2` form.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SampleSizes {
    /// Every sample has this size (`stsz.sample_size`, §8.7.3.2).
    Uniform(u32),
    /// One `stsz` entry per sample (§8.7.3.2).
    PerSample(Vec<u32>),
    /// The 4/8/16-bit `stz2` form, kept as parsed so a table round-trips.
    Compact(CompactSampleSizeBox),
}

/// One track's sample tables in compact wire form (no per-sample expansion of
/// a run-length table).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SampleTables {
    /// Number of samples the tables describe (`stsz.sample_count`).
    pub sample_count: u32,
    /// Decode timing (§8.6.1.2).
    pub stts: TimeToSampleBox,
    /// Composition offsets (§8.6.1.3); `None` when the box is omitted.
    pub ctts: Option<CompositionOffsetBox>,
    /// Sync samples (§8.6.2); `None` when every sample is one.
    pub stss: Option<SyncSampleBox>,
    /// Per-sample sizes (§8.7.3).
    pub sizes: SampleSizes,
    /// Which `stsd` entry each sample uses; the first run starts at sample 0.
    pub descriptions: Vec<DescriptionRun>,
}

impl SampleTables {
    /// The size of one sample, in bytes.
    pub fn size(&self, index: u32) -> Result<u32> {
        match &self.sizes {
            SampleSizes::Uniform(size) => Ok(*size),
            SampleSizes::PerSample(entries) => {
                entries.get(index as usize).copied().ok_or(Error::NoPlan {
                    what: "sample size",
                })
            }
            SampleSizes::Compact(compact) => {
                compact
                    .entries
                    .get(index as usize)
                    .copied()
                    .ok_or(Error::NoPlan {
                        what: "sample size",
                    })
            }
        }
    }

    /// The one-based `stsd` entry one sample uses.
    pub fn description_index(&self, index: u32) -> u32 {
        self.descriptions
            .iter()
            .rev()
            .find(|run| run.first_sample <= index)
            .map_or(DEFAULT_DESCRIPTION_INDEX, |run| run.description_index)
    }
}

/// The chunk layout a plan was read with, for
/// [`Interleave::AsPlanned`](crate::writer::Interleave::AsPlanned).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SourceChunks {
    /// The file's `stsc` runs, kept verbatim (§8.7.4).
    pub stsc: SampleToChunkBox,
    /// Samples in each chunk, in the track's own chunk order.
    pub samples_per_chunk: Vec<u32>,
    /// One-based `stsd` entry each chunk's samples use.
    pub description_index: Vec<u32>,
    /// Absolute file offset of each chunk (`stco`/`co64`, §8.7.5).
    pub offsets: Vec<u64>,
}

/// One track of an [`OwnedPlan`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PlannedTrack {
    /// Track identity, handler and header values.
    pub setup: TrackSetup,
    /// The `stsd` entries in wire order; `descriptions` indexes this list.
    pub entries: Vec<SampleEntry>,
    /// `elst` entries to write (§8.6.6); empty writes no `edts`.
    pub edits: Vec<EditListEntry>,
    /// Whether `edts` follows `mdia` in the source track's wire order. Kept so
    /// rewriting a file reproduces its child order; a built plan takes this
    /// crate's progressive order (`true`).
    pub edts_after_mdia: bool,
    /// `mdhd.duration`, in the track timescale (§8.4.2).
    pub media_duration: u64,
    /// `tkhd.duration`, on the movie timeline (§8.3.2).
    pub presentation_duration: u64,
    /// Decode time of the first sample relative to the movie timeline origin,
    /// which is what the interleave is ordered by.
    pub start_decode_time: u64,
    /// The sample tables.
    pub tables: SampleTables,
    /// The source chunk layout, when the plan came from a file.
    pub chunks: Option<SourceChunks>,
    /// The parsed `trak` this track was read from, carrying every header field,
    /// extra child and wire order the plan does not model. `None` for a track
    /// a [`PlanBuilder`] built.
    pub shell: Option<TrackBox>,
}

/// A progressive file's whole sample plan, owned.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct OwnedPlan {
    tracks: Vec<PlannedTrack>,
    shell: Option<MovieShell>,
}

impl OwnedPlan {
    /// A plan over `tracks`, in `moov` order.
    #[must_use]
    pub fn new(tracks: Vec<PlannedTrack>) -> Self {
        Self {
            tracks,
            shell: None,
        }
    }

    /// Attach the source movie shell, so the writer reproduces the `moov`
    /// children a plan does not model.
    #[must_use]
    pub fn with_shell(mut self, shell: MovieShell) -> Self {
        self.shell = Some(shell);
        self
    }

    /// The source movie shell, when this plan carries one.
    #[must_use]
    pub fn shell(&self) -> Option<&MovieShell> {
        self.shell.as_ref()
    }

    /// Replace the source movie shell.
    pub fn set_shell(&mut self, shell: Option<MovieShell>) {
        self.shell = shell;
    }

    /// Every track, in `moov` order.
    #[must_use]
    pub fn tracks(&self) -> &[PlannedTrack] {
        &self.tracks
    }

    /// One track by id.
    #[must_use]
    pub fn track(&self, track: TrackId) -> Option<&PlannedTrack> {
        self.tracks.iter().find(|t| t.setup.track_id == track)
    }

    /// The index of one track in [`OwnedPlan::tracks`].
    #[must_use]
    pub fn index_of(&self, track: TrackId) -> Option<usize> {
        self.tracks.iter().position(|t| t.setup.track_id == track)
    }

    /// Total samples over every track.
    #[must_use]
    pub fn sample_count(&self) -> u64 {
        self.tracks
            .iter()
            .map(|t| u64::from(t.tables.sample_count))
            .sum()
    }
}

/// One track's tables while [`PlanBuilder`] fills them.
struct PendingTrack {
    plan: PlannedTrack,
    stts: Vec<crate::timing::SttsEntry>,
    ctts: Vec<crate::timing::CttsEntry>,
    stss: Vec<u32>,
    all_sync: bool,
    sizes: Vec<u32>,
    descriptions: Vec<DescriptionRun>,
}

/// Builds an [`OwnedPlan`] from pushed sample descriptors, run-length coding
/// the tables as each sample arrives.
///
/// Memory is the compact tables plus one `u32` per sample that carries its own
/// size; nothing is expanded to one entry per sample until the caller asks for
/// an [`OwnedPlan`].
pub struct PlanBuilder {
    tracks: Vec<PendingTrack>,
    limits: Limits,
}

impl PlanBuilder {
    /// A builder over `tracks`, in `moov` order.
    ///
    /// `limits` bounds each track's sample count
    /// ([`Limits::max_samples_per_track`]).
    #[must_use]
    pub fn new(tracks: Vec<TrackSetup>, limits: Limits) -> Self {
        let tracks = tracks
            .into_iter()
            .map(|setup| PendingTrack {
                plan: PlannedTrack {
                    entries: alloc::vec![setup.entry.clone()],
                    setup,
                    edits: Vec::new(),
                    edts_after_mdia: true,
                    media_duration: 0,
                    presentation_duration: 0,
                    start_decode_time: 0,
                    tables: SampleTables {
                        sample_count: 0,
                        stts: TimeToSampleBox {
                            version: 0,
                            flags: 0,
                            entries: Vec::new(),
                        },
                        ctts: None,
                        stss: None,
                        sizes: SampleSizes::PerSample(Vec::new()),
                        descriptions: Vec::new(),
                    },
                    chunks: None,
                    shell: None,
                },
                stts: Vec::new(),
                ctts: Vec::new(),
                stss: Vec::new(),
                all_sync: true,
                sizes: Vec::new(),
                descriptions: Vec::new(),
            })
            .collect();
        Self { tracks, limits }
    }

    /// Append one sample to `track`, in decode order.
    pub fn push(&mut self, track: TrackId, sample: PlanSample) -> Result<()> {
        let limits = self.limits;
        let pending = self
            .tracks
            .iter_mut()
            .find(|t| t.plan.setup.track_id == track)
            .ok_or(Error::NoPlan { what: "track" })?;
        let index = u32::try_from(pending.sizes.len()).map_err(|_| Error::LimitExceeded {
            what: "samples per track",
            limit: u64::from(limits.max_samples_per_track()),
        })?;
        if index >= limits.max_samples_per_track() {
            return Err(Error::LimitExceeded {
                what: "samples per track",
                limit: u64::from(limits.max_samples_per_track()),
            });
        }
        append_stts(&mut pending.stts, sample.duration);
        append_ctts(&mut pending.ctts, sample.composition_offset);
        append_stss(&mut pending.stss, &mut pending.all_sync, index, sample.sync);
        if pending.descriptions.last().map(|run| run.description_index)
            != Some(sample.description_index)
        {
            pending.descriptions.push(DescriptionRun {
                first_sample: index,
                description_index: sample.description_index,
            });
        }
        pending.sizes.push(sample.size);
        Ok(())
    }

    /// Set the decode time of `track`'s first sample relative to the movie
    /// timeline origin, which orders the `mdat` interleave.
    pub fn set_start_decode_time(&mut self, track: TrackId, decode_time: u64) -> Result<()> {
        self.track_mut(track)?.plan.start_decode_time = decode_time;
        Ok(())
    }

    /// Set the `elst` entries written for `track` (ISO/IEC 14496-12:2026
    /// §8.6.6).
    pub fn set_edits(&mut self, track: TrackId, edits: Vec<EditListEntry>) -> Result<()> {
        self.track_mut(track)?.plan.edits = edits;
        Ok(())
    }

    /// Set the `mdhd` and `tkhd` durations (§8.4.2, §8.3.2).
    pub fn set_durations(
        &mut self,
        track: TrackId,
        media_duration: u64,
        presentation_duration: u64,
    ) -> Result<()> {
        let pending = self.track_mut(track)?;
        pending.plan.media_duration = media_duration;
        pending.plan.presentation_duration = presentation_duration;
        Ok(())
    }

    /// Finish the plan.
    pub fn finish(self) -> Result<OwnedPlan> {
        let mut tracks = Vec::with_capacity(self.tracks.len());
        let mut seen: Vec<TrackId> = Vec::with_capacity(self.tracks.len());
        for mut pending in self.tracks {
            let id = pending.plan.setup.track_id;
            if seen.contains(&id) {
                return Err(Error::InvalidInput("duplicate track id in a plan builder"));
            }
            seen.push(id);
            let sample_count =
                u32::try_from(pending.sizes.len()).map_err(|_| Error::LimitExceeded {
                    what: "samples per track",
                    limit: u64::from(self.limits.max_samples_per_track()),
                })?;
            let has_ctts = pending.ctts.iter().any(|entry| entry.sample_offset != 0);
            let ctts = has_ctts.then_some(CompositionOffsetBox {
                version: 1,
                flags: 0,
                entries: pending.ctts,
            });
            let stss = (!pending.all_sync).then_some(SyncSampleBox {
                version: 0,
                flags: 0,
                entries: pending.stss,
            });
            pending.plan.tables = SampleTables {
                sample_count,
                stts: TimeToSampleBox {
                    version: 0,
                    flags: 0,
                    entries: pending.stts,
                },
                ctts,
                stss,
                sizes: SampleSizes::PerSample(pending.sizes),
                descriptions: pending.descriptions,
            };
            tracks.push(pending.plan);
        }
        Ok(OwnedPlan::new(tracks))
    }

    fn track_mut(&mut self, track: TrackId) -> Result<&mut PendingTrack> {
        self.tracks
            .iter_mut()
            .find(|t| t.plan.setup.track_id == track)
            .ok_or(Error::NoPlan { what: "track" })
    }
}

fn stts_of(stbl: &SampleTableBox) -> Result<TimeToSampleBox> {
    stbl.children
        .iter()
        .find_map(|child| match child {
            StblChild::Stts(entries) => Some(entries.clone()),
            _ => None,
        })
        .ok_or(Error::UnexpectedBox { expected: "stts" })
}

fn ctts_of(stbl: &SampleTableBox) -> Option<CompositionOffsetBox> {
    stbl.children.iter().find_map(|child| match child {
        StblChild::Ctts(entries) => Some(entries.clone()),
        _ => None,
    })
}

fn stss_of(stbl: &SampleTableBox) -> Option<SyncSampleBox> {
    stbl.children.iter().find_map(|child| match child {
        StblChild::Stss(entries) => Some(entries.clone()),
        _ => None,
    })
}

fn sizes_of(stbl: &SampleTableBox) -> Result<SampleSizes> {
    let uniform = stbl.children.iter().find_map(|child| match child {
        StblChild::Stsz(size) => Some(size),
        _ => None,
    });
    if let Some(size) = uniform {
        return Ok(if size.sample_size == 0 {
            SampleSizes::PerSample(size.entries.clone())
        } else {
            SampleSizes::Uniform(size.sample_size)
        });
    }
    stbl.children
        .iter()
        .find_map(|child| match child {
            StblChild::Stz2(compact) => Some(SampleSizes::Compact(compact.clone())),
            _ => None,
        })
        .ok_or(Error::UnexpectedBox {
            expected: "stsz or stz2",
        })
}

fn chunk_table(stbl: &SampleTableBox) -> Result<(SampleToChunkBox, Vec<u64>)> {
    let stsc = stbl
        .children
        .iter()
        .find_map(|child| match child {
            StblChild::Stsc(runs) => Some(runs.clone()),
            _ => None,
        })
        .ok_or(Error::UnexpectedBox { expected: "stsc" })?;
    if let Some(co64) = stbl.children.iter().find_map(|child| match child {
        StblChild::Co64(offsets) => Some(offsets.entries.clone()),
        _ => None,
    }) {
        return Ok((stsc, co64));
    }
    let stco = stbl
        .children
        .iter()
        .find_map(|child| match child {
            StblChild::Stco(offsets) => Some(
                offsets
                    .entries
                    .iter()
                    .map(|&offset| u64::from(offset))
                    .collect(),
            ),
            _ => None,
        })
        .ok_or(Error::UnexpectedBox {
            expected: "stco or co64",
        })?;
    Ok((stsc, stco))
}

/// The source chunk layout and the per-chunk `stsc` values `AsPlanned` needs.
fn source_chunks(stbl: &SampleTableBox) -> Result<SourceChunks> {
    let (stsc, offsets) = chunk_table(stbl)?;
    let mut samples_per_chunk = Vec::with_capacity(offsets.len());
    let mut description_index = Vec::with_capacity(offsets.len());
    let mut run = 0usize;
    for chunk in 1..=u64::try_from(offsets.len()).unwrap_or(u64::MAX) {
        while let Some(next) = stsc.entries.get(run + 1)
            && u64::from(next.first_chunk) <= chunk
        {
            run += 1;
        }
        let entry = stsc
            .entries
            .get(run)
            .ok_or(Error::NoPlan { what: "stsc run" })?;
        if u64::from(entry.first_chunk) > chunk {
            return Err(Error::InvalidInput(
                "stsc does not start at the first chunk (ISO/IEC 14496-12:2026 §8.7.4)",
            ));
        }
        samples_per_chunk.push(entry.samples_per_chunk);
        description_index.push(entry.sample_description_index);
    }
    Ok(SourceChunks {
        stsc,
        samples_per_chunk,
        description_index,
        offsets,
    })
}

/// The per-sample `stsd`-entry runs implied by a source chunk table.
fn description_runs(chunks: &SourceChunks) -> Vec<DescriptionRun> {
    let mut runs: Vec<DescriptionRun> = Vec::new();
    let mut first_sample = 0u32;
    for (&samples, &description) in chunks
        .samples_per_chunk
        .iter()
        .zip(&chunks.description_index)
    {
        if runs.last().map(|run| run.description_index) != Some(description) {
            runs.push(DescriptionRun {
                first_sample,
                description_index: description,
            });
        }
        first_sample = first_sample.saturating_add(samples);
    }
    runs
}

/// The track's `stsd` entries, as the exact box bytes the source carried.
///
/// A sample entry is a codec description, not a sample table, so the plan keeps
/// it verbatim: re-serializing a typed parse would normalize a box another
/// muxer wrote (a `colr` ahead of `avcC`, a QuickTime `vendor` field) and break
/// the byte-identical round trip. `typed` stays available on
/// [`TrackInfo`](crate::reader::TrackInfo) for a caller that wants the parse.
fn entries_of(info: &TrackInfo) -> Vec<SampleEntry> {
    info.entries
        .iter()
        .map(|entry| SampleEntry::Raw {
            fourcc: entry.fourcc,
            body: entry.body.clone(),
        })
        .collect()
}

fn track_setup(trak: &TrackBox, info: &TrackInfo, entries: &[SampleEntry]) -> Result<TrackSetup> {
    let first = entries.first().ok_or(Error::UnexpectedBox {
        expected: "stsd entry",
    })?;
    let mdia = trak.mdia.as_ref();
    let mut setup = TrackSetup::new(
        info.track_id,
        info.handler,
        mdia.and_then(|media| media.hdlr.as_ref())
            .map_or_else(Vec::new, |hdlr| hdlr.name.clone()),
        info.timescale,
        first.clone(),
    );
    setup.minf_header = media_header_fourcc(mdia.and_then(|media| media.minf.as_ref()));
    setup.gmhd = mdia
        .and_then(|media| media.minf.as_ref())
        .and_then(|minf| minf.gmhd());
    setup.language = mdia
        .and_then(|media| media.mdhd.as_ref())
        .map_or(crate::writer::LANG_UND, |mdhd| mdhd.language);
    setup.width = info.header.width;
    setup.height = info.header.height;
    setup.volume = info.header.volume;
    setup.shell = Some(trak.clone());
    Ok(setup)
}

/// The `minf` header the writer has to emit itself: `None` when the handler
/// implies one (`vide`/`soun`), else the leading opaque header's four-CC.
fn media_header_fourcc(
    minf: Option<&crate::init_segment::MediaInformationBox>,
) -> Option<[u8; crate::box_types::BOX_TYPE_SIZE]> {
    let minf = minf?;
    if minf.vmhd.is_some() || minf.smhd.is_some() || minf.gmhd.is_some() {
        return None;
    }
    minf.opaque.first().map(|header| header.box_type)
}

/// Position of `edts` in a `trak`'s typed children (ISO/IEC 14496-12:2026
/// §8.3.1).
const TRAK_EDTS_SLOT: usize = 1;
/// Position of `mdia` in a `trak`'s typed children.
const TRAK_MDIA_SLOT: usize = 2;

/// Whether the track's `edts` box follows its `mdia` on the wire
/// (ISO/IEC 14496-12:2026 §8.3.1).
fn edts_after_mdia(trak: &TrackBox) -> bool {
    if trak.edts.is_none() {
        return false;
    }
    let order = trak.order.resolve(&[b"tkhd", b"edts", b"mdia"]);
    let edts = order.iter().position(|slot| *slot == Some(TRAK_EDTS_SLOT));
    let mdia = order.iter().position(|slot| *slot == Some(TRAK_MDIA_SLOT));
    matches!((edts, mdia), (Some(edts), Some(mdia)) if edts > mdia)
}

fn media_duration(trak: &TrackBox, info: &TrackInfo) -> u64 {
    trak.mdia
        .as_ref()
        .and_then(|media| media.mdhd.as_ref())
        .map_or(info.duration, |mdhd| mdhd.duration)
}
