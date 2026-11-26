//! The `sidx` and `mfra` a fragmented writer regenerates from what it wrote.
//!
//! ISO/IEC 14496-12:2026 §8.14.3 (segment index) and §8.8.9-§8.8.11 (random
//! access): one `sidx` reference per written fragment and one `tfra` entry per
//! fragment a track appears in (`private/specs-verified/mpeg/isobmff/sidx.md`,
//! `private/specs-verified/mpeg/isobmff/fragment-boxes.md`).
//!
//! Both boxes address the file by byte offset, so a carried copy would state
//! the *source's* positions. They are rebuilt from the offsets this writer
//! placed its own fragments at, in the form the source used (version,
//! reference type, `tfra` field widths). Nothing but the offsets and per-track
//! durations is kept: no sample bytes, and no second copy of the media.

use crate::error::{Error, Result};
use crate::frag_layout::{FragmentIndex, MfraTrackForm, SegmentIndexForm};
use crate::timing::{
    MovieFragmentRandomAccessBox, MovieFragmentRandomAccessOffsetBox, SegmentIndexBox,
    SidxReference, TrackFragmentRandomAccessBox, TrackFragmentRandomAccessEntry,
};
use crate::track_id::TrackId;
use alloc::vec::Vec;
use broadcast_core::Serialize;
use broadcast_core::timescale::Timescale;

/// The `sap_type` of a subsegment that starts with a closed GOP: every sample
/// of a written fragment is independently decodable from its first sync sample
/// (§8.14.3).
const SIDX_SAP_TYPE_CLOSED_GOP: u8 = 1;
/// The `mfro`: 8 header bytes, 4 version/flags bytes and the 32-bit size.
const MFRO_LEN: usize = 16;

/// One track's contribution to a written fragment.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WrittenTrack {
    /// `tfhd.track_ID`.
    pub track_id: TrackId,
    /// The track's `mdhd.timescale`, which its durations are in.
    pub timescale: Timescale,
    /// One-based position of the `traf` inside its `moof`.
    pub traf_number: u32,
    /// Sum of the track's sample durations in this fragment.
    pub duration: u64,
    /// The fragment's first sample is a sync sample.
    pub starts_with_sync: bool,
    /// The first sync sample: its one-based `trun` number, its one-based
    /// position inside that run, and its presentation time.
    pub first_sync: Option<(u32, u32, u64)>,
}

/// One fragment the writer emitted, by the offsets it landed at.
#[derive(Debug, Clone)]
pub(crate) struct WrittenFragment {
    /// Absolute offset of the fragment's `moof`.
    pub moof_offset: u64,
    /// Absolute offset one past the fragment's media.
    pub end: u64,
    /// Every track the fragment carries, in `traf` order.
    pub tracks: Vec<WrittenTrack>,
}

impl WrittenFragment {
    fn track(&self, track_id: TrackId) -> Option<&WrittenTrack> {
        self.tracks.iter().find(|track| track.track_id == track_id)
    }
}

/// The `sidx` over `written` in `form`'s form, or `None` when the writer was
/// not asked for one.
pub(crate) fn segment_index(
    form: Option<&SegmentIndexForm>,
    written: &[WrittenFragment],
) -> Result<Option<Vec<u8>>> {
    let Some(form) = form else {
        return Ok(None);
    };
    if form.reference_type != 0 {
        return Err(Error::UnsupportedFeature(
            "a segment index of nested sub-segment indexes is not regenerated",
        ));
    }
    let mut references = Vec::with_capacity(written.len());
    for (index, fragment) in written.iter().enumerate() {
        let end = match written.get(index.saturating_add(1)) {
            Some(next) => next.moof_offset,
            None => fragment.end,
        };
        let referenced_size = end.saturating_sub(fragment.moof_offset);
        let track = fragment.track(form.reference_id);
        references.push(SidxReference {
            reference_type: form.reference_type,
            referenced_size: u32::try_from(referenced_size).map_err(|_| {
                Error::InvalidInput("a fragment is larger than a sidx referenced_size can state")
            })?,
            subsegment_duration: match track {
                Some(track) => scaled_duration(track.duration, track.timescale, form.timescale)?,
                None => 0,
            },
            starts_with_sap: u8::from(track.is_some_and(|track| track.starts_with_sync)),
            sap_type: SIDX_SAP_TYPE_CLOSED_GOP,
            sap_delta_time: 0,
        });
    }
    let boxed = SegmentIndexBox {
        version: form.version,
        flags: form.flags,
        reference_id: form.reference_id.value(),
        timescale: form.timescale,
        earliest_presentation_time: form.earliest_presentation_time,
        first_offset: 0,
        references,
    };
    Ok(Some(boxed.try_to_bytes()?))
}

/// `duration` converted from `from` to `to` ticks per second.
fn scaled_duration(duration: u64, from: Timescale, to: Timescale) -> Result<u32> {
    let scaled = u128::from(duration) * u128::from(to.get());
    let ticks = scaled / u128::from(from.get().max(1));
    u32::try_from(ticks).map_err(|_| {
        Error::InvalidInput("a fragment is longer than a sidx subsegment_duration can state")
    })
}

/// The trailing `mfra` over `written` in `form`'s form, or `None` when the
/// writer was not asked for one.
pub(crate) fn random_access(
    form: Option<&crate::frag_layout::MfraForm>,
    written: &[WrittenFragment],
) -> Result<Option<Vec<u8>>> {
    let Some(form) = form else {
        return Ok(None);
    };
    let mut tables = Vec::with_capacity(form.tracks.len());
    for track in &form.tracks {
        let entries: Vec<TrackFragmentRandomAccessEntry> = written
            .iter()
            .filter_map(|fragment| {
                let carried = fragment.track(track.track_id)?;
                let (trun_number, sample_number, time) = carried.first_sync?;
                Some(TrackFragmentRandomAccessEntry {
                    time,
                    moof_offset: fragment.moof_offset,
                    traf_number: carried.traf_number,
                    trun_number,
                    sample_number,
                })
            })
            .collect();
        if entries.is_empty() {
            continue;
        }
        tables.push(tfra(track, entries));
    }
    let size = tables
        .iter()
        .try_fold(
            crate::box_types::BOX_HEADER_MIN_SIZE + MFRO_LEN,
            |total, table| total.checked_add(table.serialized_len()),
        )
        .ok_or(Error::InvalidInput("mfra length overflows"))?;
    let parent_size = broadcast_core::len::fit_u32(size, "mfro parent_size")?;
    let mfra = MovieFragmentRandomAccessBox {
        tfra: tables,
        mfro: MovieFragmentRandomAccessOffsetBox {
            version: 0,
            flags: 0,
            parent_size,
        },
    };
    Ok(Some(mfra.try_to_bytes()?))
}

/// One track's `tfra` in the recorded form.
fn tfra(
    track: &MfraTrackForm,
    entries: Vec<TrackFragmentRandomAccessEntry>,
) -> TrackFragmentRandomAccessBox {
    TrackFragmentRandomAccessBox {
        version: track.version,
        flags: 0,
        track_id: track.track_id,
        length_size_of_traf_num: track.lengths.traf,
        length_size_of_trun_num: track.lengths.trun,
        length_size_of_sample_num: track.lengths.sample,
        entries,
    }
}

/// The `sidx` bytes over `written`, when the index layout sets a form.
pub(crate) fn sidx_bytes(
    index: &FragmentIndex,
    written: &[WrittenFragment],
) -> Result<Option<Vec<u8>>> {
    segment_index(index.sidx.as_ref(), written)
}
