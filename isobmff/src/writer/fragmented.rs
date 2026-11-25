//! The fragmented writer's buffers and the one `moof`+`mdat` builder.
//!
//! ISO/IEC 14496-12:2026 §8.8 (movie fragments): `mfhd`, `traf`, `tfhd`,
//! `tfdt`, `trun` and the `mdat` each `trun.data_offset` resolves against
//! under `default-base-is-moof`. The CENC `senc`/`saiz`/`saio` emission is
//! ISO/IEC 23001-7 §12.2/§12.3.

use super::index::{WrittenFragment, WrittenTrack};
use super::{
    Fragment, FragmentOptions, FragmentTrack, SampleData, SampleMeta, SegmentBrands,
    TREX_DEFAULT_DESCRIPTION_INDEX, TrackSetup, TrexDefaults,
};
use crate::box_types::{BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE};
use crate::cenc::{SAIO, SAIZ, SENC, SampleEncryptionEntry};
use crate::emsg::EmsgBox;
use crate::error::{Error, Result};
use crate::frag_layout::{
    FragmentIndex, FragmentLayout, MfraForm, MfraTrackForm, SegmentIndexForm, TfraLengthSizes,
    sample_flags,
};
use crate::limits::Limits;
use crate::movie_fragment::{
    CENC_AUX_INFO_TYPES, MoofChild, MovieFragmentBox, MovieFragmentHeaderBox,
    SAIZ_SUBSAMPLE_COUNT_SIZE, SAIZ_SUBSAMPLE_ENTRY_SIZE, SAMPLE_FLAGS_NON_SYNC, SAMPLE_FLAGS_SYNC,
    SENC_ENTRIES_OFFSET, TFHD_DEFAULT_BASE_IS_MOOF, TRUN_DATA_OFFSET_PRESENT,
    TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT, TRUN_SAMPLE_DURATION_PRESENT,
    TRUN_SAMPLE_FLAGS_PRESENT, TRUN_SAMPLE_SIZE_PRESENT, TrackFragmentBaseMediaDecodeTimeBox,
    TrackFragmentBox, TrackFragmentHeaderBox, TrackFragmentRunBox, TrafChild, TrunSample,
    aux_info_type_of,
};
use crate::segments::{MOOF, MediaDataBox, SegmentTypeBox};
use crate::track_id::TrackId;
use alloc::vec;
use alloc::vec::Vec;
use broadcast_common::Serialize;
use broadcast_common::timescale::Timescale;

/// The per-sample overhead a buffered fragment reserves for its `trun` record
/// and bookkeeping, so the fragment cap bounds the whole buffer rather than
/// only the coded bytes.
const SAMPLE_RECORD_BYTES: u64 = core::mem::size_of::<TrunSample>() as u64;

/// One buffered sample: its metadata and its coded bytes.
struct BufferedSample {
    meta: SampleMeta,
    data: Vec<u8>,
}

/// One track's buffer between two flushes.
struct TrackState {
    setup: TrackSetup,
    base_decode_time: u64,
    active: bool,
    samples: Vec<BufferedSample>,
}

impl TrackState {
    fn new(setup: TrackSetup) -> Self {
        Self {
            setup,
            base_decode_time: 0,
            active: false,
            samples: Vec::new(),
        }
    }
}

/// The sans-IO fragmented writer returned by
/// [`Writer::fragmented`](super::Writer::fragmented).
pub struct FragmentedWriter {
    tracks: Vec<TrackState>,
    movie_timescale: Timescale,
    limits: Limits,
    file_type: super::FileType,
    shell: Option<crate::plan::MovieShell>,
    sequence_number: u32,
    buffered_bytes: u64,
    layout: Option<FragmentLayout>,
    compact: bool,
    position: u64,
    index: FragmentIndex,
    written: Vec<WrittenFragment>,
}

impl FragmentedWriter {
    pub(crate) fn new(
        tracks: Vec<TrackSetup>,
        movie_timescale: Timescale,
        limits: Limits,
    ) -> Result<Self> {
        let mut seen = Vec::with_capacity(tracks.len());
        for track in &tracks {
            if seen.contains(&track.track_id) {
                return Err(Error::InvalidInput(
                    "duplicate track id in a fragmented writer",
                ));
            }
            seen.push(track.track_id);
        }
        Ok(Self {
            tracks: tracks.into_iter().map(TrackState::new).collect(),
            movie_timescale,
            limits,
            file_type: super::FileType::default(),
            shell: None,
            sequence_number: 1,
            buffered_bytes: 0,
            layout: None,
            compact: false,
            position: 0,
            index: FragmentIndex::default(),
            written: Vec::new(),
        })
    }

    /// Replace the `ftyp` brand set of [`FragmentedWriter::init`].
    pub fn set_file_type(&mut self, file_type: super::FileType) {
        self.file_type = file_type;
    }

    /// Attach the source movie shell, so [`FragmentedWriter::init`] reproduces
    /// the `moov` children a [`TrackSetup`] does not model.
    pub fn set_movie_shell(&mut self, shell: crate::plan::MovieShell) {
        self.shell = Some(shell);
    }

    /// The attached source movie shell, when one was set.
    #[must_use]
    pub fn movie_shell(&self) -> Option<&crate::plan::MovieShell> {
        self.shell.as_ref()
    }

    /// Set the `mfhd.sequence_number` the next flush writes.
    pub fn set_sequence_number(&mut self, sequence_number: u32) {
        self.sequence_number = sequence_number;
    }

    /// The fragmented initialization segment (`ftyp` + `moov`).
    pub fn init(&self) -> Result<Vec<u8>> {
        let setups: Vec<TrackSetup> = self.tracks.iter().map(|t| t.setup.clone()).collect();
        super::build_init_segment(
            &setups,
            self.movie_timescale,
            &self.file_type,
            self.shell.as_ref(),
        )
    }

    /// The trailing boxes the shell carries after the last fragment's media
    /// (`free`, `skip`, a `uuid` box), then the regenerated `mfra` when the
    /// index layout asks for one.
    pub fn trailer(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        if let Some(shell) = &self.shell {
            for box_ in shell.top_level_at(crate::plan::BoxPosition::AfterMdat) {
                out.extend_from_slice(&box_.try_to_bytes()?);
            }
        }
        if let Some(mfra) =
            crate::writer::index::random_access(self.index.mfra.as_ref(), &self.written)?
        {
            out.extend_from_slice(&mfra);
        }
        Ok(out)
    }

    /// Reproduce `layout` on the next flush, and only that flush.
    ///
    /// Every `tfhd`/`trun` field choice, run split and child order the layout
    /// states is written as it states it; a sample value the layout leaves to
    /// a default is checked against that default and fails with
    /// [`Error::LayoutMismatch`] when it disagrees.
    pub fn set_fragment_layout(&mut self, layout: FragmentLayout) {
        self.layout = Some(layout);
        self.compact = false;
    }

    /// Write every following flush in [`FragmentLayout::compact`]'s form:
    /// `tfhd` sample defaults where the track's samples agree and
    /// `first_sample_flags` where only the first differs. Off by default, so a
    /// writer that is not asked writes this crate's canonical per-sample form.
    pub fn set_compact_layout(&mut self, compact: bool) {
        self.compact = compact;
    }

    /// State the absolute file offset at which the next flush's bytes will be
    /// placed, so a layout's `base_data_offset` and the regenerated
    /// `tfra`/`sidx` entries state real positions. Every flush advances it by
    /// the bytes it returns.
    pub fn set_position(&mut self, position: u64) {
        self.position = position;
    }

    /// The absolute file offset the next flush's bytes will be placed at.
    #[must_use]
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Regenerate the index boxes around the fragments from `index`'s forms.
    pub fn set_index_layout(&mut self, index: FragmentIndex) {
        self.index = index;
    }

    /// The index layout this writer regenerates.
    #[must_use]
    pub fn index_layout(&self) -> &FragmentIndex {
        &self.index
    }

    /// Emit a trailing `mfra` over the flushed fragments, in one `tfra` per
    /// track with one-byte variable fields and 64-bit times.
    pub fn with_mfra(&mut self, enabled: bool) -> &mut Self {
        self.index.mfra = enabled.then(|| {
            let tracks = self
                .tracks
                .iter()
                .map(|track| MfraTrackForm {
                    track_id: track.setup.track_id,
                    version: 1,
                    lengths: TfraLengthSizes::default(),
                })
                .collect();
            MfraForm { tracks }
        });
        self
    }

    /// Emit a leading segment index over the flushed fragments, on the first
    /// track's timeline.
    pub fn with_sidx(&mut self, enabled: bool) -> &mut Self {
        self.index.sidx = if enabled {
            self.tracks
                .first()
                .map(|track| SegmentIndexForm::new(track.setup.track_id, track.setup.timescale))
        } else {
            None
        };
        self
    }

    /// The segment index over the flushed fragments, for a caller to place
    /// before the first of them (streaming output cannot insert it once the
    /// fragments are gone).
    pub fn segment_index(&self) -> Result<Option<Vec<u8>>> {
        crate::writer::index::sidx_bytes(&self.index, &self.written)
    }
}

impl FragmentedWriter {
    /// Anchor a track's fragment at an absolute decode time (`tfdt`, §8.8.12).
    ///
    /// Also marks the track as present in the next flush, so a track with no
    /// samples still gets its (empty) `traf`, matching the caller's track set.
    pub fn set_base_decode_time(&mut self, track: TrackId, decode_time: u64) -> Result<()> {
        let state = self.track_mut(track)?;
        state.base_decode_time = decode_time;
        state.active = true;
        Ok(())
    }

    /// Buffer one sample for the next fragment.
    ///
    /// Fails with [`Error::FragmentTooLarge`] before the buffer grows past
    /// [`Limits::max_fragment_bytes`].
    pub fn push(&mut self, track: TrackId, meta: SampleMeta, data: &[u8]) -> Result<()> {
        let limit = self.limits.max_fragment_bytes();
        let data_len = u64::try_from(data.len()).map_err(|_| Error::FragmentTooLarge { limit })?;
        let aux_len = meta.aux.as_ref().map_or(0u64, |aux| {
            (aux.initialization_vector.len()
                + aux.subsamples.len() * core::mem::size_of::<crate::cenc::SubSampleEntry>())
                as u64
        });
        let add = data_len
            .checked_add(aux_len)
            .and_then(|n| n.checked_add(SAMPLE_RECORD_BYTES))
            .ok_or(Error::FragmentTooLarge { limit })?;
        let projected = self
            .buffered_bytes
            .checked_add(add)
            .ok_or(Error::FragmentTooLarge { limit })?;
        if projected > limit {
            return Err(Error::FragmentTooLarge { limit });
        }
        let state = self.track_mut(track)?;
        state.samples.push(BufferedSample {
            meta,
            data: data.to_vec(),
        });
        state.active = true;
        self.buffered_bytes = projected;
        Ok(())
    }

    /// Flush one fragment with no `styp` and no events.
    pub fn flush(&mut self) -> Result<Option<Fragment>> {
        self.flush_with(None, &[])
    }

    /// Flush one `[styp] [emsg]* moof mdat` fragment.
    ///
    /// `None` when no track is active. The buffer is cleared only after the
    /// fragment has been built, so a failed build leaves it intact.
    pub fn flush_with(
        &mut self,
        styp: Option<&SegmentBrands>,
        events: &[EmsgBox<'_>],
    ) -> Result<Option<Fragment>> {
        let active: Vec<usize> = (0..self.tracks.len())
            .filter(|&i| self.tracks[i].active)
            .collect();
        if active.is_empty() {
            return Ok(None);
        }
        let mut sample_sets: Vec<Vec<SampleData<'_>>> = Vec::with_capacity(active.len());
        let mut anchors: Vec<(TrackId, u64, TrexDefaults)> = Vec::with_capacity(active.len());
        for &i in &active {
            sample_sets.push(
                self.tracks[i]
                    .samples
                    .iter()
                    .map(|s| SampleData {
                        meta: &s.meta,
                        data: &s.data,
                    })
                    .collect(),
            );
            anchors.push((
                self.tracks[i].setup.track_id,
                self.tracks[i].base_decode_time,
                self.tracks[i].setup.trex.unwrap_or_default(),
            ));
        }
        let fragment_tracks: Vec<FragmentTrack<'_>> = anchors
            .iter()
            .enumerate()
            .map(
                |(slot, &(track_id, base_decode_time, trex))| FragmentTrack {
                    track_id,
                    base_decode_time,
                    samples: &sample_sets[slot],
                    trex,
                },
            )
            .collect();
        let pending = self.layout.take();
        let compact = self.compact;
        let layout = match pending {
            Some(layout) => Some(layout),
            None if compact => Some(FragmentLayout::compact(&fragment_tracks)?),
            None => None,
        };
        let options = FragmentOptions {
            sequence_number: self.sequence_number,
            styp,
            events,
            layout: layout.as_ref(),
            position: self.position,
        };
        let start = self.position;
        let fragment = fragmented_segment(&fragment_tracks, &options)?;
        let written = self.written_fragment(&fragment_tracks, layout.as_ref(), start, &fragment)?;
        self.written.push(written);
        self.position =
            self.position
                .saturating_add(u64::try_from(fragment.bytes.len()).map_err(|_| {
                    Error::InvalidInput("fragment is past the 64-bit offset range")
                })?);
        for state in &mut self.tracks {
            state.samples.clear();
            state.active = false;
        }
        self.buffered_bytes = 0;
        self.sequence_number = self.sequence_number.wrapping_add(1);
        Ok(Some(fragment))
    }

    /// The track order the fragment's `traf`s take: the layout's own when it
    /// states one, else the flush's track order.
    fn traf_order(layout: Option<&FragmentLayout>, tracks: &[FragmentTrack<'_>]) -> Vec<TrackId> {
        let Some(layout) = layout else {
            return tracks.iter().map(|track| track.track_id).collect();
        };
        let mut order = Vec::with_capacity(layout.tracks.len());
        for child in &layout.children {
            if let MoofChild::Traf(index) = child
                && let Some(track) = layout.tracks.get(*index)
            {
                order.push(track.track_id);
            }
        }
        if order.len() != layout.tracks.len() {
            return layout.tracks.iter().map(|track| track.track_id).collect();
        }
        order
    }

    /// The offsets and per-track facts of one emitted fragment.
    fn written_fragment(
        &self,
        tracks: &[FragmentTrack<'_>],
        layout: Option<&FragmentLayout>,
        start: u64,
        fragment: &Fragment,
    ) -> Result<WrittenFragment> {
        let length = u64::try_from(fragment.bytes.len())
            .map_err(|_| Error::InvalidInput("fragment is past the 64-bit offset range"))?;
        let mut written = Vec::with_capacity(layout.map_or(tracks.len(), |l| l.tracks.len()));
        for (slot, track_id) in Self::traf_order(layout, tracks).iter().enumerate() {
            let ft = tracks
                .iter()
                .find(|track| track.track_id == *track_id)
                .ok_or(Error::InvalidInput(
                    "a fragment traf names a track this flush does not carry",
                ))?;
            let timescale = self
                .tracks
                .iter()
                .find(|state| state.setup.track_id == *track_id)
                .ok_or(Error::InvalidInput(
                    "a fragment traf names a track this writer does not hold",
                ))?
                .setup
                .timescale;
            let runs: Vec<usize> = match layout.and_then(|layout| layout.track(*track_id)) {
                Some(track) => track.runs.iter().map(|run| run.sample_count).collect(),
                None => vec![ft.samples.len()],
            };
            let traf_number = u32::try_from(slot.saturating_add(1)).unwrap_or(u32::MAX);
            written.push(Self::written_track(ft, timescale, &runs, traf_number));
        }
        Ok(WrittenFragment {
            moof_offset: fragment.moof_offset,
            end: start.saturating_add(length),
            tracks: written,
        })
    }

    /// One track's facts for the index boxes: its durations, and where its
    /// first sync sample sits.
    fn written_track(
        ft: &FragmentTrack<'_>,
        timescale: Timescale,
        runs: &[usize],
        traf_number: u32,
    ) -> WrittenTrack {
        let mut decode_time = ft.base_decode_time;
        let mut duration = 0u64;
        let mut first_sync = None;
        let mut run = 0usize;
        let mut left = runs.first().copied().unwrap_or(0);
        let mut in_run = 0u32;
        for sample in ft.samples {
            if left == 0
                && let Some(next) = runs.get(run.saturating_add(1))
            {
                run = run.saturating_add(1);
                left = *next;
                in_run = 0;
            }
            if first_sync.is_none() && sample.meta.sync {
                let time = i128::from(decode_time) + i128::from(sample.meta.composition_offset);
                first_sync = Some((
                    u32::try_from(run.saturating_add(1)).unwrap_or(u32::MAX),
                    in_run.saturating_add(1),
                    u64::try_from(time).unwrap_or(0),
                ));
            }
            duration = duration.saturating_add(u64::from(sample.meta.duration));
            decode_time = decode_time.saturating_add(u64::from(sample.meta.duration));
            in_run = in_run.saturating_add(1);
            left = left.saturating_sub(1);
        }
        WrittenTrack {
            track_id: ft.track_id,
            timescale,
            traf_number,
            duration,
            starts_with_sync: ft.samples.first().is_some_and(|sample| sample.meta.sync),
            first_sync,
        }
    }

    fn track_mut(&mut self, track: TrackId) -> Result<&mut TrackState> {
        self.tracks
            .iter_mut()
            .find(|t| t.setup.track_id == track)
            .ok_or(Error::InvalidInput(
                "track id is not part of this fragmented writer",
            ))
    }
}

/// Build one `[before]* [styp] [emsg]* moof mdat` fragment over
/// already-collected samples.
///
/// The single fragment builder: [`FragmentedWriter::flush`] prepares its
/// buffers and calls this, and a caller that already holds its samples calls it
/// directly. Without a [`FragmentOptions::layout`] every sample's duration,
/// size, flags and composition offset sits in `trun` and each run is addressed
/// from the `moof` (`default-base-is-moof`, §8.8.7); with one, the `tfhd`/
/// `trun` field choices, the run splits, the child order and the boxes between
/// fragments are the source's, and `trun.data_offset`/`tfhd.base_data_offset`
/// are recomputed from where the fragment is written.
pub fn fragmented_segment(
    tracks: &[FragmentTrack<'_>],
    options: &FragmentOptions<'_>,
) -> Result<Fragment> {
    let styp = options.styp.map(|brands| SegmentTypeBox {
        major_brand: brands.major_brand,
        minor_version: brands.minor_version,
        compatible_brands: brands.compatible_brands.clone(),
    });

    let mut moof = match options.layout {
        Some(layout) => layout_moof(tracks, layout, options.sequence_number)?,
        None => canonical_moof(tracks, options.sequence_number)?,
    };

    let emsg_bytes = event_bytes(options.events)?;
    let mut prefix = Vec::new();
    if let Some(layout) = options.layout {
        for box_ in &layout.before {
            prefix.extend_from_slice(&box_.try_to_bytes()?);
        }
    }
    if let Some(styp) = &styp {
        let mut bytes = vec![0u8; styp.serialized_len()];
        let written = styp.serialize_into(&mut bytes)?;
        bytes.truncate(written);
        prefix.extend_from_slice(&bytes);
    }
    prefix.extend_from_slice(&emsg_bytes);
    let moof_offset = options
        .position
        .checked_add(u64::try_from(prefix.len()).map_err(|_| {
            Error::InvalidInput("fragment prefix is past the 64-bit file offset range")
        })?)
        .ok_or(Error::InvalidInput("fragment offset overflows the file"))?;

    place_payload(&mut moof, tracks, moof_offset, options.layout)?;
    let mut mdat_data = Vec::new();
    fill_mdat(&moof, tracks, &mut mdat_data)?;

    let mdat = MediaDataBox { data: mdat_data };
    let mut body = vec![0u8; moof.serialized_len()];
    let written = moof.serialize_into(&mut body)?;
    body.truncate(written);
    let mut mdat_bytes = vec![0u8; mdat.serialized_len()];
    let n = mdat.serialize_into(&mut mdat_bytes)?;
    body.extend_from_slice(&mdat_bytes[..n]);

    if options.layout.is_some() {
        // A recorded layout reproduces the source's own protection boxes; the
        // samples' aux info is not re-derived on top of them.
        if tracks
            .iter()
            .any(|track| track.samples.iter().any(|sample| sample.meta.aux.is_some()))
        {
            return Err(Error::InvalidInput(
                "a recorded fragment layout cannot carry new CENC aux",
            ));
        }
    }
    let protections = ProtectionSets::build(tracks)?;
    let body = if protections.is_empty() {
        body
    } else {
        protect_media_segment(&body, &protections.protections())?
    };
    let moof_len = first_box_len(&body, &MOOF)?;

    let mut bytes = prefix;
    bytes.extend_from_slice(&body);
    Ok(Fragment {
        bytes,
        moof_len,
        moof_offset,
    })
}

/// One `moof` in this crate's canonical form: per-sample `trun` fields,
/// `trun`/`tfdt` versions derived from the samples, one run per track and
/// `default-base-is-moof` addressing.
fn canonical_moof(tracks: &[FragmentTrack<'_>], sequence_number: u32) -> Result<MovieFragmentBox> {
    let mut traf_boxes = Vec::with_capacity(tracks.len());
    for ft in tracks {
        let description = fragment_description(ft)?;
        let runs = ft
            .samples
            .iter()
            .map(|s| {
                Ok(RunSample {
                    duration: s.meta.duration,
                    size: s.data.len(),
                    sync: s.meta.sync,
                    composition_offset: s.meta.composition_offset,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let (tfhd, trun) = media_fragment_run(ft.track_id, runs.into_iter(), description)?;
        let tfdt = TrackFragmentBaseMediaDecodeTimeBox::new_v1(ft.base_decode_time);
        traf_boxes.push(TrackFragmentBox::new(tfhd, Some(tfdt), vec![trun]));
    }
    Ok(MovieFragmentBox::new(
        MovieFragmentHeaderBox::new(sequence_number),
        traf_boxes,
    ))
}

/// One `moof` reproducing `layout`, with every sample value its omitted
/// fields leave to a default checked against that default.
fn layout_moof(
    tracks: &[FragmentTrack<'_>],
    layout: &crate::frag_layout::FragmentLayout,
    sequence_number: u32,
) -> Result<MovieFragmentBox> {
    if layout.tracks.len() != tracks.len() {
        return Err(Error::InvalidInput(
            "the recorded fragment layout does not cover this flush's track set",
        ));
    }
    check_listed(
        layout.children.iter().filter_map(|child| match child {
            MoofChild::Traf(index) => Some(*index),
            _ => None,
        }),
        layout.tracks.len(),
        "the recorded moof child order does not list every traf",
    )?;
    let mut traf_boxes = Vec::with_capacity(layout.tracks.len());
    for track_layout in &layout.tracks {
        let ft = tracks
            .iter()
            .find(|track| track.track_id == track_layout.track_id)
            .ok_or(Error::InvalidInput(
                "the recorded fragment layout names a track this flush does not carry",
            ))?;
        traf_boxes.push(layout_track_fragment(ft, track_layout)?);
    }
    Ok(MovieFragmentBox {
        mfhd: MovieFragmentHeaderBox::new(sequence_number),
        traf: traf_boxes,
        order: layout.children.clone(),
    })
}

/// One `traf` as `track_layout` states it.
fn layout_track_fragment(
    ft: &FragmentTrack<'_>,
    track_layout: &crate::frag_layout::TrackFragmentLayout,
) -> Result<TrackFragmentBox> {
    check_listed(
        track_layout
            .children
            .iter()
            .filter_map(|child| match child {
                TrafChild::Trun(index) => Some(*index),
                _ => None,
            }),
        track_layout.runs.len(),
        "the recorded traf child order does not list every run",
    )?;
    check_description(ft, &track_layout.tfhd)?;
    let declared: usize = track_layout.runs.iter().map(|run| run.sample_count).sum();
    if declared != ft.samples.len() {
        return Err(Error::LayoutMismatch {
            field: "sample_count",
            track: ft.track_id,
            sample: u32::try_from(declared).unwrap_or(u32::MAX),
            expected: u64::try_from(declared).unwrap_or(u64::MAX),
            actual: u64::try_from(ft.samples.len()).unwrap_or(u64::MAX),
        });
    }
    let tfhd = TrackFragmentHeaderBox {
        flags: track_layout.tfhd.flags,
        track_id: ft.track_id,
        base_data_offset: track_layout.tfhd.base_data_offset.map(|_| 0),
        sample_description_index: track_layout.tfhd.sample_description_index,
        default_sample_duration: track_layout.tfhd.default_sample_duration,
        default_sample_size: track_layout.tfhd.default_sample_size,
        default_sample_flags: track_layout.tfhd.default_sample_flags,
    };
    let mut runs = Vec::with_capacity(track_layout.runs.len());
    let mut start = 0usize;
    for run in &track_layout.runs {
        let end = start.saturating_add(run.sample_count);
        let samples = ft.samples.get(start..end).ok_or(Error::LayoutMismatch {
            field: "sample_count",
            track: ft.track_id,
            sample: u32::try_from(start).unwrap_or(u32::MAX),
            expected: u64::try_from(end).unwrap_or(u64::MAX),
            actual: u64::try_from(ft.samples.len()).unwrap_or(u64::MAX),
        })?;
        runs.push(layout_run(ft, &tfhd, run, samples, start)?);
        start = end;
    }
    let has_tfdt = track_layout.children.is_empty()
        || track_layout
            .children
            .iter()
            .any(|child| matches!(child, TrafChild::Tfdt));
    let tfdt = if has_tfdt {
        Some(decode_time_box(ft, track_layout.tfdt_version)?)
    } else {
        None
    };
    Ok(TrackFragmentBox {
        tfhd,
        tfdt,
        trun: runs,
        order: track_layout.children.clone(),
    })
}

/// The `tfdt` for `ft` in the version the layout states.
fn decode_time_box(
    ft: &FragmentTrack<'_>,
    version: u8,
) -> Result<TrackFragmentBaseMediaDecodeTimeBox> {
    if version == 0 {
        let time = u32::try_from(ft.base_decode_time).map_err(|_| {
            Error::InvalidInput(
                "a version-0 tfdt cannot state this fragment's base media decode time",
            )
        })?;
        return Ok(TrackFragmentBaseMediaDecodeTimeBox::new_v0(time));
    }
    Ok(TrackFragmentBaseMediaDecodeTimeBox::new_v1(
        ft.base_decode_time,
    ))
}

/// One `trun` as `run` states it, with every sample value the run leaves out
/// checked against the default it inherits.
fn layout_run(
    ft: &FragmentTrack<'_>,
    tfhd: &TrackFragmentHeaderBox,
    run: &crate::frag_layout::RunLayout,
    samples: &[SampleData<'_>],
    first_index: usize,
) -> Result<TrackFragmentRunBox> {
    let default_duration = tfhd
        .default_sample_duration
        .unwrap_or(ft.trex.default_sample_duration);
    let default_size = tfhd
        .default_sample_size
        .unwrap_or(ft.trex.default_sample_size);
    let default_flags = tfhd
        .default_sample_flags
        .unwrap_or(ft.trex.default_sample_flags);
    let mut records = Vec::with_capacity(samples.len());
    for (offset, sample) in samples.iter().enumerate() {
        let index = first_index.saturating_add(offset);
        let duration = sample.meta.duration;
        if !run.fields.duration {
            check_mismatch(ft, "sample_duration", index, default_duration, duration)?;
        }
        let size = u32::try_from(sample.data.len()).map_err(|_| {
            Error::InvalidInput("sample larger than the 32-bit trun sample_size field")
        })?;
        if !run.fields.size {
            check_mismatch(ft, "sample_size", index, default_size, size)?;
        }
        let flags = sample_flags(sample.meta.sync);
        if !run.fields.flags {
            let expected = if offset == 0 {
                run.first_sample_flags.unwrap_or(default_flags)
            } else {
                default_flags
            };
            check_mismatch(ft, "sample_flags", index, expected, flags)?;
        }
        let composition_offset = sample.meta.composition_offset;
        if !run.fields.composition_offset {
            check_mismatch(
                ft,
                "sample_composition_time_offset",
                index,
                0,
                composition_offset.unsigned_abs(),
            )?;
        }
        records.push(TrunSample {
            sample_duration: run.fields.duration.then_some(duration),
            sample_size: run.fields.size.then_some(size),
            sample_flags: run.fields.flags.then_some(flags),
            sample_composition_time_offset: run
                .fields
                .composition_offset
                .then_some(i64::from(composition_offset)),
        });
    }
    if run.version == 0
        && let Some((offset, _)) = samples
            .iter()
            .enumerate()
            .find(|(_, sample)| sample.meta.composition_offset < 0)
    {
        return Err(Error::LayoutMismatch {
            field: "trun version",
            track: ft.track_id,
            sample: u32::try_from(first_index.saturating_add(offset)).unwrap_or(u32::MAX),
            expected: 1,
            actual: u64::from(run.version),
        });
    }
    Ok(TrackFragmentRunBox {
        version: run.version,
        tr_flags: if records.is_empty() {
            run.fields.flags()
        } else {
            0
        },
        data_offset: run.data_offset.then_some(0),
        first_sample_flags: run.first_sample_flags,
        samples: records,
    })
}

/// A recorded `tfhd`/`trex` default against the value a sample states.
fn check_mismatch(
    ft: &FragmentTrack<'_>,
    field: &'static str,
    index: usize,
    expected: u32,
    actual: u32,
) -> Result<()> {
    if expected == actual {
        return Ok(());
    }
    Err(Error::LayoutMismatch {
        field,
        track: ft.track_id,
        sample: u32::try_from(index).unwrap_or(u32::MAX),
        expected: u64::from(expected),
        actual: u64::from(actual),
    })
}

/// A recorded child order must list every child its container holds: one the
/// order leaves out would be written without its payload, so its samples would
/// go missing from the `mdat`.
fn check_listed(
    indices: impl Iterator<Item = usize>,
    count: usize,
    missing: &'static str,
) -> Result<()> {
    let listed: Vec<usize> = indices.collect();
    if listed.is_empty() {
        return Ok(());
    }
    let mut seen = vec![false; count];
    for index in listed {
        if let Some(slot) = seen.get_mut(index) {
            *slot = true;
        }
    }
    if seen.iter().any(|listed| !listed) {
        return Err(Error::InvalidInput(missing));
    }
    Ok(())
}

/// The `tfhd.sample_description_index` a recorded layout states against the
/// one every sample uses.
fn check_description(
    ft: &FragmentTrack<'_>,
    tfhd: &crate::frag_layout::TrackFragmentHeaderLayout,
) -> Result<()> {
    let stated = tfhd
        .sample_description_index
        .unwrap_or(ft.trex.sample_description_index);
    for (index, sample) in ft.samples.iter().enumerate() {
        if sample.meta.description_index != stated {
            return Err(Error::LayoutMismatch {
                field: "sample_description_index",
                track: ft.track_id,
                sample: u32::try_from(index).unwrap_or(u32::MAX),
                expected: u64::from(stated),
                actual: u64::from(sample.meta.description_index),
            });
        }
    }
    Ok(())
}

/// State each `traf`'s absolute `base_data_offset` and each run's
/// `data_offset` for where the fragment's media will land, in `moof` order.
fn place_payload(
    moof: &mut MovieFragmentBox,
    tracks: &[FragmentTrack<'_>],
    moof_offset: u64,
    layout: Option<&crate::frag_layout::FragmentLayout>,
) -> Result<()> {
    let mut cursor = moof.serialized_len() + BOX_HEADER_MIN_SIZE;
    for child in moof.child_order_for_rewrite() {
        let MoofChild::Traf(index) = child else {
            continue;
        };
        let traf = moof
            .traf
            .get_mut(index)
            .ok_or(Error::InvalidInput("moof child index out of range"))?;
        let track_id = traf.tfhd.track_id;
        let ft = tracks
            .iter()
            .find(|track| track.track_id == track_id)
            .ok_or(Error::InvalidInput(
                "a moof traf names a track this fragment has no samples for",
            ))?;
        let delta = layout
            .and_then(|layout| layout.track(track_id))
            .and_then(|track| track.tfhd.base_data_offset)
            .unwrap_or(0);
        if traf.tfhd.base_data_offset.is_some() {
            traf.tfhd.base_data_offset = Some(absolute_base(moof_offset, delta)?);
        }
        let mut next = 0usize;
        for run in &mut traf.trun {
            if run.data_offset.is_some() {
                run.data_offset = Some(offset_from_base(cursor, delta)?);
            }
            for _ in 0..run.samples.len() {
                let sample = ft.samples.get(next).ok_or(Error::InvalidInput(
                    "a trun states more samples than the fragment carries",
                ))?;
                cursor = cursor.saturating_add(sample.data.len());
                next = next.saturating_add(1);
            }
        }
    }
    Ok(())
}

/// The fragment's media, in the `moof`'s `traf` order.
fn fill_mdat(
    moof: &MovieFragmentBox,
    tracks: &[FragmentTrack<'_>],
    out: &mut Vec<u8>,
) -> Result<()> {
    for child in moof.child_order_for_rewrite() {
        let MoofChild::Traf(index) = child else {
            continue;
        };
        let track_id = moof
            .traf
            .get(index)
            .ok_or(Error::InvalidInput("moof child index out of range"))?
            .tfhd
            .track_id;
        let ft = tracks
            .iter()
            .find(|track| track.track_id == track_id)
            .ok_or(Error::InvalidInput(
                "a moof traf names a track this fragment has no samples for",
            ))?;
        for sample in ft.samples {
            out.extend_from_slice(sample.data);
        }
    }
    Ok(())
}

/// An absolute `tfhd.base_data_offset` from the `moof`'s offset and a
/// relative delta.
fn absolute_base(moof_offset: u64, delta: i64) -> Result<u64> {
    let base = i128::from(moof_offset) + i128::from(delta);
    u64::try_from(base)
        .map_err(|_| Error::InvalidInput("tfhd.base_data_offset leaves the 64-bit offset range"))
}

/// A `trun.data_offset` from a payload position inside the fragment and the
/// `traf`'s own base delta.
fn offset_from_base(cursor: usize, delta: i64) -> Result<i32> {
    let position = i64::try_from(cursor)
        .map_err(|_| Error::InvalidInput("fragment offset is past the signed 64-bit range"))?;
    let value = position
        .checked_sub(delta)
        .ok_or(Error::InvalidInput("trun data_offset overflows"))?;
    i32::try_from(value).map_err(|_| {
        Error::InvalidValue(broadcast_common::error::InvalidValue {
            field: "trun data_offset",
            value: u64::try_from(value).unwrap_or(u64::MAX),
            reason: "fragment too large for the signed 32-bit trun data_offset",
        })
    })
}
/// The one-based `stsd` entry a fragment's samples use, when it is not the
/// default one.
///
/// `tfhd.sample_description_index` is stated once per track fragment, so a run
/// whose samples disagree cannot be expressed and is refused rather than
/// silently rewritten with the first sample's entry.
fn fragment_description(ft: &FragmentTrack<'_>) -> Result<Option<u32>> {
    let mut entries = ft
        .samples
        .iter()
        .map(|sample| sample.meta.description_index);
    let Some(first) = entries.next() else {
        return Ok(None);
    };
    if entries.any(|index| index != first) {
        return Err(Error::InvalidInput(
            "one track fragment mixes sample_description_index values",
        ));
    }
    Ok((first != TREX_DEFAULT_DESCRIPTION_INDEX).then_some(first))
}

/// The first box of `bytes` whose type is `fourcc`, as its declared length.
fn first_box_len(bytes: &[u8], fourcc: &[u8; BOX_TYPE_SIZE]) -> Result<usize> {
    for step in crate::BoxIter::new(bytes) {
        let item = step?;
        if item.header.box_type.is(fourcc) {
            return Ok(item.end - item.start);
        }
    }
    Err(Error::UnexpectedBox { expected: "moof" })
}

/// Serialize the caller's event boxes, in order.
fn event_bytes(events: &[EmsgBox<'_>]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for event in events {
        out.extend_from_slice(&event.to_vec()?);
    }
    Ok(out)
}

/// One sample of a fragment's `trun`, as the box builder describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunSample {
    /// `sample_duration`, in the track timescale.
    pub duration: u32,
    /// Coded size in bytes.
    pub size: usize,
    /// Sync sample (random-access point).
    pub sync: bool,
    /// `pts - dts` composition offset.
    pub composition_offset: i32,
}

/// The `tfhd` + `trun` every media-fragment writer emits for one track:
/// per-sample duration/size/flags, with a v1 signed composition offset only
/// when some sample has a non-zero one (B-frames), `default-base-is-moof`
/// addressing and a placeholder `data_offset` of 0 for the caller to patch
/// once the `moof` size is known.
pub fn media_fragment_run(
    track_id: TrackId,
    samples: impl Iterator<Item = RunSample> + Clone,
    description_index: Option<u32>,
) -> Result<(TrackFragmentHeaderBox, TrackFragmentRunBox)> {
    let any_cts = samples.clone().any(|s| s.composition_offset != 0);
    let mut run_samples = Vec::new();
    for s in samples {
        run_samples.push(TrunSample {
            sample_duration: Some(s.duration),
            sample_size: Some(u32::try_from(s.size).map_err(|_| {
                Error::InvalidInput("sample larger than the 32-bit trun sample_size field")
            })?),
            sample_flags: Some(if s.sync {
                SAMPLE_FLAGS_SYNC
            } else {
                SAMPLE_FLAGS_NON_SYNC
            }),
            sample_composition_time_offset: any_cts.then_some(i64::from(s.composition_offset)),
        });
    }
    let mut tr_flags = TRUN_DATA_OFFSET_PRESENT
        | TRUN_SAMPLE_DURATION_PRESENT
        | TRUN_SAMPLE_SIZE_PRESENT
        | TRUN_SAMPLE_FLAGS_PRESENT;
    // Version 1 carries a signed composition offset (needed for B-frames).
    let version = if any_cts {
        tr_flags |= TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT;
        1u8
    } else {
        0u8
    };
    let trun = TrackFragmentRunBox {
        version,
        tr_flags,
        data_offset: Some(0),
        first_sample_flags: None,
        samples: run_samples,
    };
    let tfhd = TrackFragmentHeaderBox {
        flags: TFHD_DEFAULT_BASE_IS_MOOF,
        track_id,
        base_data_offset: None,
        sample_description_index: description_index,
        default_sample_duration: None,
        default_sample_size: None,
        default_sample_flags: None,
    };
    Ok((tfhd, trun))
}

/// The owned `senc` entry lists for one fragment's protected tracks.
struct ProtectionSets {
    entries: Vec<Vec<SampleEncryptionEntry>>,
    track_ids: Vec<TrackId>,
    per_sample_iv_sizes: Vec<u8>,
}

impl ProtectionSets {
    /// Collect each track that carries CENC aux. Every sample of such a track
    /// must carry an entry, and the initialization vectors must agree in size
    /// (`senc` states one `per_sample_iv_size`, §12.3).
    fn build(tracks: &[FragmentTrack<'_>]) -> Result<Self> {
        let mut entries = Vec::new();
        let mut track_ids = Vec::new();
        let mut per_sample_iv_sizes = Vec::new();
        for ft in tracks {
            if !ft.samples.iter().any(|s| s.meta.aux.is_some()) {
                continue;
            }
            let mut track_entries = Vec::with_capacity(ft.samples.len());
            let mut iv_size: Option<usize> = None;
            for sample in ft.samples {
                let aux = sample.meta.aux.as_ref().ok_or(Error::InvalidInput(
                    "every sample of a protected track must carry CENC aux",
                ))?;
                match iv_size {
                    None => iv_size = Some(aux.initialization_vector.len()),
                    Some(size) if size != aux.initialization_vector.len() => {
                        return Err(Error::InvalidInput(
                            "a protected track's per-sample initialization vectors differ in size",
                        ));
                    }
                    Some(_) => {}
                }
                track_entries.push(SampleEncryptionEntry {
                    initialization_vector: aux.initialization_vector.clone(),
                    subsamples: aux.subsamples.clone(),
                });
            }
            let size = u8::try_from(iv_size.unwrap_or(0)).map_err(|_| {
                Error::InvalidInput(
                    "per-sample initialization vector longer than the 8-bit senc field",
                )
            })?;
            entries.push(track_entries);
            track_ids.push(ft.track_id);
            per_sample_iv_sizes.push(size);
        }
        Ok(Self {
            entries,
            track_ids,
            per_sample_iv_sizes,
        })
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn protections(&self) -> Vec<FragmentProtection<'_>> {
        self.track_ids
            .iter()
            .enumerate()
            .map(|(i, &track_id)| FragmentProtection {
                track_id,
                entries: &self.entries[i],
                per_sample_iv_size: self.per_sample_iv_sizes[i],
            })
            .collect()
    }
}

/// One protected track's per-sample CENC aux info for a single movie
/// fragment, for [`protect_media_segment`] — the movie-fragment half of the
/// muxer's CENC emission.
pub struct FragmentProtection<'a> {
    /// The `tfhd.track_id` of the `traf` to protect.
    pub track_id: TrackId,
    /// Per-sample IV + subsample map for **this fragment's samples only**, in
    /// decode order. Must have exactly as many entries as the matching
    /// `traf`'s total `trun` sample count (checked).
    pub entries: &'a [SampleEncryptionEntry],
    /// [`crate::cenc::TrackEncryptionBox::default_per_sample_iv_size`] for
    /// this track — the fixed IV length written into `senc`.
    pub per_sample_iv_size: u8,
}

/// The `senc`/`saiz`/`saio` triple built for one protected `traf`.
struct CencFragmentBoxes {
    senc: crate::cenc::SampleEncryptionBox,
    saiz: crate::cenc::SampleAuxInfoSizesBox,
    saio: crate::cenc::SampleAuxInfoOffsetsBox,
}

impl CencFragmentBoxes {
    fn added_len(&self) -> usize {
        self.senc.serialized_len() + self.saiz.serialized_len() + self.saio.serialized_len()
    }
}

/// Build the `senc`/`saiz`/`saio` boxes for one protected track's fragment,
/// or `None` when this track's fragment has nothing meaningful for them to
/// carry (ISO/IEC 23001-7 §12.2/§12.3).
///
/// `senc`'s syntax writes, per sample, `InitializationVector[Per_Sample_IV_Size]`
/// then — only when `UseSubSampleEncryption` is set — a subsample map. When a
/// track uses a constant IV (`per_sample_iv_size == 0`, the per-sample IV lives
/// once in `tenc.default_constant_IV`) *and* whole-sample protection (no
/// subsample split), every one of `senc`'s entries is zero bytes wide: exactly
/// the shape [`crate::cenc::SampleEncryptionBox::parse_body`] rejects. There is
/// nothing such a fragment needs the triple for, so it is omitted rather than
/// emitted unparseable.
///
/// `saio.offsets[0]` is a placeholder (`0`) — [`protect_media_segment`]
/// back-patches it once every box's final position in the rebuilt `moof` is
/// known.
fn build_cenc_fragment_boxes(p: &FragmentProtection<'_>) -> Result<Option<CencFragmentBoxes>> {
    let use_subsamples = p.entries.iter().any(|e| !e.subsamples.is_empty());
    if p.per_sample_iv_size == 0 && !use_subsamples {
        return Ok(None);
    }
    let flags = if use_subsamples {
        crate::cenc::SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION
    } else {
        0
    };
    let senc = crate::cenc::SampleEncryptionBox {
        version: 0,
        flags,
        per_sample_iv_size: p.per_sample_iv_size,
        entries: p.entries.to_vec(),
    };

    let mut sizes = Vec::with_capacity(p.entries.len());
    for e in p.entries {
        let mut sz = usize::from(p.per_sample_iv_size);
        if use_subsamples {
            sz += SAIZ_SUBSAMPLE_COUNT_SIZE + e.subsamples.len() * SAIZ_SUBSAMPLE_ENTRY_SIZE;
        }
        if sz > usize::from(u8::MAX) {
            return Err(Error::InvalidInput(
                "per-sample aux info size exceeds the 255-byte saiz field",
            ));
        }
        sizes.push(sz as u32);
    }
    let uniform = sizes
        .first()
        .copied()
        .filter(|first| sizes.iter().all(|s| s == first));
    let sample_count = broadcast_common::len::fit_u32(sizes.len(), "saiz sample_count")?;
    let saiz = crate::cenc::SampleAuxInfoSizesBox {
        version: 0,
        flags: 0,
        aux_info_type: None,
        aux_info_type_parameter: None,
        default_sample_info_size: uniform.unwrap_or(0),
        sample_count,
        sample_info_sizes: if uniform.is_some() { Vec::new() } else { sizes },
    };
    let saio = crate::cenc::SampleAuxInfoOffsetsBox {
        version: 0,
        flags: 0,
        aux_info_type: None,
        aux_info_type_parameter: None,
        offsets: vec![0u64],
    };
    Ok(Some(CencFragmentBoxes { senc, saiz, saio }))
}

/// Rewrite an already-built **single-fragment** media segment (`styp`
/// [+ `emsg`*] + one `moof` + `mdat`) so each track named in `protections`
/// gets CENC `senc`/`saiz`/`saio` boxes appended to its `traf.
///
/// A track whose fragment has nothing for those boxes to carry (constant IV +
/// whole-sample protection) is left unmodified entirely; its samples are still
/// encrypted, decryptable from `tenc`'s `default_constant_IV` alone.
///
/// Only the buffer's *first* `moof` is located and rewritten — a buffer
/// containing more than one `moof` is not supported; any `moof`s beyond the
/// first are copied through untouched as part of the verbatim suffix.
///
/// Every `trun.data_offset` in the (possibly-grown) `moof` — protected track
/// or not — is shifted by the exact number of bytes the new boxes add, so
/// every sample in the unchanged `mdat` still resolves against the
/// `default-base-is-moof` base. `media_segment` may be a bare
/// `styp`+`moof`+`mdat` triple or a larger buffer with more boxes before/after
/// (e.g. a whole init-segment-prefixed output) — only the (first) `moof` span
/// is touched; everything before and after it is copied through verbatim.
/// Returns the input unchanged when `protections` is empty.
pub fn protect_media_segment(
    media_segment: &[u8],
    protections: &[FragmentProtection<'_>],
) -> Result<Vec<u8>> {
    if protections.is_empty() {
        return Ok(media_segment.to_vec());
    }

    let mut prefix_len = 0usize;
    let mut moof_len = None;
    for step in crate::BoxIter::new(media_segment) {
        let box_ref = step?;
        let size = box_ref.end - box_ref.start;
        if box_ref.header.box_type.is(&MOOF) {
            moof_len = Some(size);
            break;
        }
        prefix_len += size;
    }
    let moof_len = moof_len.ok_or(Error::UnexpectedBox { expected: "moof" })?;
    let moof_bytes = &media_segment[prefix_len..prefix_len + moof_len];
    let suffix = &media_segment[prefix_len + moof_len..];

    let mut moof = MovieFragmentBox::parse_body(crate::box_types::parse_box(moof_bytes)?.0.body)?;

    let mut built: Vec<Option<CencFragmentBoxes>> = (0..moof.traf.len()).map(|_| None).collect();
    for p in protections {
        let idx = moof
            .traf
            .iter()
            .position(|t| t.tfhd.track_id == p.track_id)
            .ok_or(Error::InvalidInput(
                "protect_media_segment: track_id not present in moof",
            ))?;
        let sample_count: usize = moof.traf[idx].trun.iter().map(|r| r.samples.len()).sum();
        if sample_count != p.entries.len() {
            return Err(Error::InvalidInput(
                "protect_media_segment: entries.len() must equal the traf's total trun sample count",
            ));
        }
        // A `traf` that already carries a CENC `senc`/`saiz`/`saio` would end
        // up with a *second* set, and a decryptor has no way to tell which
        // pair applies (§12.3 defines one `senc` per `traf`). Protection is
        // idempotent-by-rejection: the caller must strip the existing triple.
        // `senc` is CENC by definition; `saiz`/`saio` are generic
        // auxiliary-information boxes (§8.7.8/§8.7.9) identified by
        // `aux_info_type`, so only a CENC-typed pair (or one with the field
        // absent, which §8.7.8.2 defines as `cenc`) collides.
        for child in &moof.traf[idx].order {
            let TrafChild::Opaque(o) = child else {
                continue;
            };
            let collides = match o.box_type {
                SENC => true,
                SAIZ | SAIO => aux_info_type_of(&o.data, &o.box_type)
                    .is_none_or(|ty| CENC_AUX_INFO_TYPES.contains(&ty)),
                _ => false,
            };
            if collides {
                return Err(Error::InvalidInput(
                    "protect_media_segment: traf already carries a CENC senc/saiz/saio",
                ));
            }
        }
        built[idx] = build_cenc_fragment_boxes(p)?;
    }

    // Pass A: each traf's "base" (tfhd/tfdt/trun plus its own opaque children)
    // length, and its final length once its senc/saiz/saio are appended.
    let base_lens: Vec<usize> = moof.traf.iter().map(|t| t.serialized_len()).collect();
    let final_lens: Vec<usize> = base_lens
        .iter()
        .zip(&built)
        .map(|(&base, b)| base + b.as_ref().map(CencFragmentBoxes::added_len).unwrap_or(0))
        .collect();

    // The rebuilt `moof` in wire order. A `moof` may carry opaque children
    // (`meta`, `pssh`, a `uuid`) at any position; the rebuilt length includes
    // those bytes.
    let order = moof.child_order_for_rewrite();
    let new_moof_len = BOX_HEADER_MIN_SIZE
        + order
            .iter()
            .map(|child| match child {
                MoofChild::Mfhd => moof.mfhd.serialized_len(),
                MoofChild::Traf(i) => final_lens.get(*i).copied().unwrap_or(0),
                MoofChild::Opaque(o) => o.serialized_len(),
            })
            .sum::<usize>();
    let delta = new_moof_len as i64 - moof_len as i64;

    // Pass B: compute each protected traf's saio.offset[0] (moof-relative) in
    // the same wire order pass C writes, and shift every trun.data_offset by
    // `delta`.
    let mut running = BOX_HEADER_MIN_SIZE as u64;
    for child in &order {
        let traf_index = match child {
            MoofChild::Mfhd => {
                running += moof.mfhd.serialized_len() as u64;
                continue;
            }
            MoofChild::Opaque(o) => {
                running += o.serialized_len() as u64;
                continue;
            }
            MoofChild::Traf(i) => *i,
        };
        let Some(traf) = moof.traf.get_mut(traf_index) else {
            return Err(Error::InvalidInput(
                "protect_media_segment: moof child index out of range",
            ));
        };
        if let Some(b) = built[traf_index].as_mut() {
            let senc_start = running + base_lens[traf_index] as u64;
            b.saio.offsets[0] = senc_start + SENC_ENTRIES_OFFSET;
        }
        running += final_lens[traf_index] as u64;

        for run in &mut traf.trun {
            if run.tr_flags & TRUN_DATA_OFFSET_PRESENT != 0 {
                let current = run.data_offset.ok_or(Error::InvalidInput(
                    "protect_media_segment: trun declares data_offset but carries none",
                ))?;
                let shifted = i64::from(current)
                    .checked_add(delta)
                    .and_then(|v| i32::try_from(v).ok())
                    .ok_or(Error::InvalidInput(
                        "protect_media_segment: trun.data_offset leaves the 32-bit field",
                    ))?;
                run.data_offset = Some(shifted);
            }
        }
    }
    // Pass C: serialize the rebuilt moof. A protected traf's own leading size
    // field is back-patched to the final length once senc/saiz/saio have been
    // appended; every child keeps its parsed order.
    let mut moof_out = vec![0u8; new_moof_len];
    let moof_size = broadcast_common::len::fit_u32(new_moof_len, "moof size")?;
    moof_out[0..4].copy_from_slice(&moof_size.to_be_bytes());
    moof_out[4..8].copy_from_slice(&MOOF);
    let mut c = BOX_HEADER_MIN_SIZE;
    for child in moof.child_order_for_rewrite() {
        match child {
            MoofChild::Mfhd => c += moof.mfhd.serialize_into(&mut moof_out[c..])?,
            MoofChild::Traf(i) => {
                let start = c;
                let traf = moof.traf.get(i).ok_or(Error::InvalidInput(
                    "protect_media_segment: moof child index out of range",
                ))?;
                c += traf.serialize_into(&mut moof_out[c..])?;
                if let Some(b) = built.get(i).and_then(|b| b.as_ref()) {
                    c += b.senc.serialize_into(&mut moof_out[c..])?;
                    c += b.saiz.serialize_into(&mut moof_out[c..])?;
                    c += b.saio.serialize_into(&mut moof_out[c..])?;
                    let final_len = broadcast_common::len::fit_u32(c - start, "traf size")?;
                    moof_out[start..start + core::mem::size_of::<u32>()]
                        .copy_from_slice(&final_len.to_be_bytes());
                }
            }
            MoofChild::Opaque(ref o) => c += o.serialize_into(&mut moof_out[c..])?,
        }
    }
    if c != new_moof_len {
        return Err(Error::InvalidInput(
            "moof length/senc offset consistency check failed",
        ));
    }

    let mut out = Vec::with_capacity(prefix_len + new_moof_len + suffix.len());
    out.extend_from_slice(&media_segment[..prefix_len]);
    out.extend_from_slice(&moof_out);
    out.extend_from_slice(suffix);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cenc::{SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION, SampleEncryptionBox, SubSampleEntry};
    use crate::movie_fragment::{TRUN_SAMPLE_SIZE_PRESENT, TrunSample};
    use alloc::vec;

    fn sample(composition_offset: i32) -> RunSample {
        RunSample {
            duration: 1_000,
            size: 40,
            sync: true,
            composition_offset,
        }
    }

    #[test]
    fn media_fragment_run_serializes_the_derived_trun_flags() {
        let flags_of = |offset: i32| {
            let (_, trun) = media_fragment_run(
                TrackId::new(1),
                [sample(offset), sample(offset)].into_iter(),
                None,
            )
            .expect("two-sample run");
            let bytes = trun.try_to_bytes().expect("serialize trun");
            u32::from_be_bytes([0, bytes[9], bytes[10], bytes[11]])
        };
        // data-offset-present (0x000001) | sample-duration (0x000100) |
        // sample-size (0x000200) | sample-flags (0x000400).
        assert_eq!(flags_of(0), 0x0000_0701);
        // plus sample-composition-time-offset-present (0x000800).
        assert_eq!(flags_of(1), 0x0000_0F01);
    }

    /// A constant-IV (`per_sample_iv_size == 0`), whole-sample-protected
    /// (`subsamples` empty) track has nothing for `senc` to carry: every one
    /// of `sample_count`'s entries would be zero bytes wide. That degenerate
    /// shape is exactly what the `senc` parser rejects, so the builder must
    /// return `None`.
    #[test]
    fn build_cenc_fragment_boxes_omits_senc_for_constant_iv_whole_sample() {
        let degenerate_senc_body: [u8; 4] = 2u32.to_be_bytes();
        let rejected = SampleEncryptionBox::parse_body(&degenerate_senc_body, 0, 0, 0);
        assert!(
            matches!(rejected, Err(Error::InvalidInput(_))),
            "this crate's own senc parser must reject a sample_count with no backing per-sample \
             bytes, got {rejected:?}"
        );

        let entries = vec![
            SampleEncryptionEntry {
                initialization_vector: Vec::new(),
                subsamples: Vec::new(),
            },
            SampleEncryptionEntry {
                initialization_vector: Vec::new(),
                subsamples: Vec::new(),
            },
        ];
        let protection = FragmentProtection {
            track_id: TrackId::new(1),
            entries: &entries,
            per_sample_iv_size: 0,
        };
        assert!(
            build_cenc_fragment_boxes(&protection)
                .expect("must not error")
                .is_none()
        );
    }

    /// End-to-end: [`protect_media_segment`] over a constant-IV,
    /// whole-sample-protected track must leave the `moof` byte-identical to
    /// the unprotected input.
    #[test]
    fn protect_media_segment_constant_iv_whole_sample_round_trips_with_no_senc() {
        const TRACK_ID: u32 = 1;

        let tfhd = TrackFragmentHeaderBox {
            flags: 0,
            track_id: TrackId::new(TRACK_ID),
            base_data_offset: None,
            sample_description_index: None,
            default_sample_duration: None,
            default_sample_size: None,
            default_sample_flags: None,
        };
        let trun = TrackFragmentRunBox {
            version: 0,
            tr_flags: TRUN_SAMPLE_SIZE_PRESENT,
            data_offset: None,
            first_sample_flags: None,
            samples: vec![
                TrunSample {
                    sample_duration: None,
                    sample_size: Some(4),
                    sample_flags: None,
                    sample_composition_time_offset: None,
                },
                TrunSample {
                    sample_duration: None,
                    sample_size: Some(4),
                    sample_flags: None,
                    sample_composition_time_offset: None,
                },
            ],
        };
        let traf = TrackFragmentBox::new(tfhd, None, vec![trun]);
        let moof = MovieFragmentBox::new(MovieFragmentHeaderBox::new(1), vec![traf]);
        let unprotected = moof.try_to_bytes().expect("serialize moof");

        let entries = vec![
            SampleEncryptionEntry {
                initialization_vector: Vec::new(),
                subsamples: Vec::new(),
            },
            SampleEncryptionEntry {
                initialization_vector: Vec::new(),
                subsamples: Vec::new(),
            },
        ];
        let protection = FragmentProtection {
            track_id: TrackId::new(TRACK_ID),
            entries: &entries,
            per_sample_iv_size: 0,
        };

        let protected = protect_media_segment(&unprotected, &[protection])
            .expect("must not error on a constant-IV whole-sample track");
        assert_eq!(
            protected, unprotected,
            "no senc/saiz/saio bytes must be appended when there is nothing for them to carry"
        );

        let reparsed = MovieFragmentBox::parse_body(&protected[BOX_HEADER_MIN_SIZE..])
            .expect("protected output must still parse as a valid moof");
        assert_eq!(reparsed.traf.len(), 1);
        assert_eq!(reparsed.traf[0].trun[0].samples.len(), 2);
    }

    /// The `saiz` `sample_info_size` is the per-sample IV length plus, under
    /// subsample encryption, a 2-byte count and a 6-byte entry per subsample,
    /// and its `u8` width makes 255 legal and 256 an error.
    #[test]
    fn build_cenc_fragment_boxes_sizes_each_sample_aux_info() {
        let entry = |iv: u8, subsamples: usize| SampleEncryptionEntry {
            initialization_vector: vec![0x11; usize::from(iv)],
            subsamples: (0..subsamples)
                .map(|i| SubSampleEntry {
                    bytes_of_clear_data: i as u16,
                    bytes_of_protected_data: 0,
                })
                .collect(),
        };
        let entries = vec![entry(8, 1), entry(8, 3), entry(8, 0)];
        let protection = FragmentProtection {
            track_id: TrackId::new(1),
            entries: &entries,
            per_sample_iv_size: 8,
        };
        let built = build_cenc_fragment_boxes(&protection)
            .expect("aux info sizes below the ceiling")
            .expect("subsample encryption carries a senc");
        assert_eq!(
            built.saiz.sample_info_sizes,
            vec![8 + 2 + 6, 8 + 2 + 18, 8 + 2],
            "IV + 2-byte count + 6 bytes per subsample"
        );
        assert_eq!(built.saiz.sample_count, 3);
        assert_eq!(built.saiz.default_sample_info_size, 0);
        assert_eq!(
            built.senc.flags & SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION,
            SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION
        );

        // 1 + 2 + 6 * 42 == 255 fits the `u8` field and stays uniform.
        let entries = vec![entry(1, 42)];
        let at_the_ceiling = FragmentProtection {
            track_id: TrackId::new(1),
            entries: &entries,
            per_sample_iv_size: 1,
        };
        let built = build_cenc_fragment_boxes(&at_the_ceiling)
            .expect("255 bytes is a legal sample_info_size")
            .expect("senc present");
        assert_eq!(built.saiz.default_sample_info_size, 255);
        assert!(built.saiz.sample_info_sizes.is_empty());

        // 2 + 2 + 6 * 42 == 256 does not.
        let entries = vec![entry(2, 42)];
        let over_the_ceiling = FragmentProtection {
            track_id: TrackId::new(1),
            entries: &entries,
            per_sample_iv_size: 2,
        };
        let err = build_cenc_fragment_boxes(&over_the_ceiling)
            .err()
            .expect("256 bytes exceeds the u8 sample_info_size");
        assert!(
            matches!(err, Error::InvalidInput(_)),
            "expected InvalidInput, got {err:?}"
        );
    }
}
