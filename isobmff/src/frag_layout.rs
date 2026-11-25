//! The write-back layout of a movie fragment: which `tfhd`/`trun` fields a
//! source states, how its runs split, the child order, and the forms of the
//! index boxes around the fragments.
//!
//! ISO/IEC 14496-12:2026 §8.8.3 (`traf`), §8.8.7 (`tfhd`), §8.8.8 (`trun`),
//! §8.8.9-§8.8.11 (`mfra`/`tfra`/`mfro`) and §8.14.3 (`sidx`)
//! (`private/specs-verified/mpeg/isobmff/fragment-boxes.md`,
//! `private/specs-verified/mpeg/isobmff/sidx.md`).
//!
//! A fragment's *content* (which samples, when, how big) is the plan's
//! business; its *layout* — whether a duration lives in every `trun` record or
//! once in the `tfhd`, whether the run is addressed from the `moof` or from an
//! explicit `base_data_offset`, where the opaque children sit — is the
//! muxer's. Files written by ffmpeg, GPAC, Bento4 and Shaka Packager disagree
//! on all of it, so [`crate::reader::FragmentInfo`] carries one of these per
//! fragment and the writer reproduces it byte for byte.
//!
//! Every offset a layout holds is **relative** (`base_data_offset` to the
//! `moof`'s first byte, `trun.data_offset` and `sidx`/`tfra` entries are
//! recomputed from the written positions), so a fragment moved to a different
//! file offset still describes itself correctly.

use crate::box_support::OpaqueBox;
use crate::error::{Error, Result};
use crate::movie_fragment::{
    MoofChild, MovieFragmentBox, SAMPLE_FLAGS_NON_SYNC, SAMPLE_FLAGS_SYNC,
    TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT, TRUN_SAMPLE_DURATION_PRESENT,
    TRUN_SAMPLE_FLAGS_PRESENT, TRUN_SAMPLE_SIZE_PRESENT, TrackFragmentHeaderBox, TrafChild,
};
use crate::track_id::TrackId;
use alloc::vec::Vec;
use broadcast_common::timescale::Timescale;

/// The sample flags a sample's sync flag stands for (§8.8.8.3): a sync sample
/// states `sample_depends_on = 2` (an I frame), a non-sync one the
/// `sample_is_non_sync_sample` bit.
#[must_use]
pub(crate) fn sample_flags(sync: bool) -> u32 {
    if sync {
        SAMPLE_FLAGS_SYNC
    } else {
        SAMPLE_FLAGS_NON_SYNC
    }
}

/// Which per-sample fields a `trun`'s records carry (§8.8.8.2).
///
/// `trun` is flag-driven and fixed-width: a field is either present in every
/// record or in none, and a field that is absent takes its value from the
/// `tfhd` default (then `trex`), never from the previous record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RunFields {
    /// `sample_duration` present.
    pub duration: bool,
    /// `sample_size` present.
    pub size: bool,
    /// `sample_flags` present.
    pub flags: bool,
    /// `sample_composition_time_offset` present.
    pub composition_offset: bool,
}

impl RunFields {
    /// The fields a `tr_flags` word states.
    #[must_use]
    pub fn from_flags(tr_flags: u32) -> Self {
        Self {
            duration: tr_flags & TRUN_SAMPLE_DURATION_PRESENT != 0,
            size: tr_flags & TRUN_SAMPLE_SIZE_PRESENT != 0,
            flags: tr_flags & TRUN_SAMPLE_FLAGS_PRESENT != 0,
            composition_offset: tr_flags & TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT != 0,
        }
    }

    /// The `tr_flags` bits these fields stand for.
    #[must_use]
    pub fn flags(self) -> u32 {
        let mut bits = 0;
        if self.duration {
            bits |= TRUN_SAMPLE_DURATION_PRESENT;
        }
        if self.size {
            bits |= TRUN_SAMPLE_SIZE_PRESENT;
        }
        if self.flags {
            bits |= TRUN_SAMPLE_FLAGS_PRESENT;
        }
        if self.composition_offset {
            bits |= TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT;
        }
        bits
    }
}

/// One `trun` of a recorded fragment: its version, the fields its records
/// carry, whether it states its own `data_offset`/`first_sample_flags`, and
/// how many samples it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct RunLayout {
    /// `trun` version (§8.8.8.2): 0 makes the composition offset unsigned.
    pub version: u8,
    /// The per-sample fields every record carries.
    pub fields: RunFields,
    /// Whether the run states a `data_offset` (recomputed on write).
    pub data_offset: bool,
    /// `first_sample_flags`: the flags of the run's first sample, when the
    /// `tfhd` default does not describe it.
    pub first_sample_flags: Option<u32>,
    /// `sample_count`.
    pub sample_count: usize,
}

/// One `tfhd` of a recorded fragment: its non-presence flags, which optional
/// fields it states, and its `base_data_offset` measured from the `moof`'s
/// first byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackFragmentHeaderLayout {
    /// The flags other than the five field-presence bits, which are derived
    /// from the `Option`s below (`default-base-is-moof`, `duration-is-empty`).
    pub flags: u32,
    /// `base_data_offset` relative to the `moof`'s first byte.
    pub base_data_offset: Option<i64>,
    /// `sample_description_index` (one-based).
    pub sample_description_index: Option<u32>,
    /// `default_sample_duration`.
    pub default_sample_duration: Option<u32>,
    /// `default_sample_size`.
    pub default_sample_size: Option<u32>,
    /// `default_sample_flags`.
    pub default_sample_flags: Option<u32>,
}

impl TrackFragmentHeaderLayout {
    /// The layout `tfhd` states, with its absolute base moved to a `moof`
    /// relative one.
    pub fn from_tfhd(tfhd: &TrackFragmentHeaderBox, moof_offset: u64) -> Result<Self> {
        let base_data_offset = match tfhd.base_data_offset {
            None => None,
            Some(absolute) => {
                let relative = base_offset_delta(absolute, moof_offset)?;
                Some(relative)
            }
        };
        Ok(Self {
            flags: tfhd.flags & !TFHD_FIELD_PRESENCE_BITS,
            base_data_offset,
            sample_description_index: tfhd.sample_description_index,
            default_sample_duration: tfhd.default_sample_duration,
            default_sample_size: tfhd.default_sample_size,
            default_sample_flags: tfhd.default_sample_flags,
        })
    }
}

/// The five `tfhd` flag bits whose value lives in a field rather than in the
/// flags word (§8.8.7.1).
pub(crate) const TFHD_FIELD_PRESENCE_BITS: u32 =
    crate::movie_fragment::TFHD_BASE_DATA_OFFSET_PRESENT
        | crate::movie_fragment::TFHD_SAMPLE_DESCRIPTION_INDEX_PRESENT
        | crate::movie_fragment::TFHD_DEFAULT_SAMPLE_DURATION_PRESENT
        | crate::movie_fragment::TFHD_DEFAULT_SAMPLE_SIZE_PRESENT
        | crate::movie_fragment::TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT;

/// An absolute `base_data_offset` as its distance from the `moof`'s first
/// byte.
fn base_offset_delta(absolute: u64, moof_offset: u64) -> Result<i64> {
    let absolute = i64::try_from(absolute).map_err(|_| {
        Error::InvalidInput("tfhd.base_data_offset is past the signed 64-bit range")
    })?;
    let moof = i64::try_from(moof_offset)
        .map_err(|_| Error::InvalidInput("moof offset is past the signed 64-bit range"))?;
    absolute
        .checked_sub(moof)
        .ok_or(Error::InvalidInput("tfhd.base_data_offset delta overflows"))
}

/// One `traf` of a recorded fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TrackFragmentLayout {
    /// `tfhd.track_ID`.
    pub track_id: TrackId,
    /// The `tfhd`'s field choices.
    pub tfhd: TrackFragmentHeaderLayout,
    /// `tfdt` version (§8.8.12), meaningful only when `children` lists a
    /// [`TrafChild::Tfdt`].
    pub tfdt_version: u8,
    /// The `trun`s, in the order they appear in `children`.
    pub runs: Vec<RunLayout>,
    /// Every `traf` child in wire order, opaque ones (`sdtp`, `sbgp`, `sgpd`,
    /// `senc`, `saiz`, `saio`, `subs`, a `uuid`) included.
    pub children: Vec<TrafChild>,
}

impl TrackFragmentLayout {
    /// The layout one parsed `traf` states.
    fn from_traf(traf: &crate::movie_fragment::TrackFragmentBox, moof_offset: u64) -> Result<Self> {
        let runs = traf
            .trun
            .iter()
            .map(|trun| RunLayout {
                version: trun.version,
                fields: RunFields::from_flags(trun.effective_flags()),
                data_offset: trun.data_offset.is_some(),
                first_sample_flags: trun.first_sample_flags,
                sample_count: trun.samples.len(),
            })
            .collect();
        Ok(Self {
            track_id: traf.tfhd.track_id,
            tfhd: TrackFragmentHeaderLayout::from_tfhd(&traf.tfhd, moof_offset)?,
            tfdt_version: traf.tfdt.map_or(0, |tfdt| tfdt.version()),
            runs,
            children: traf.order.clone(),
        })
    }
}

/// One fragment's layout: what [`crate::reader::Step::Fragment`] reports and
/// what the writer reproduces.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FragmentLayout {
    /// `mfhd.sequence_number` (§8.8.2).
    pub sequence_number: u32,
    /// The `traf`s, in the order they appear in `children`.
    pub tracks: Vec<TrackFragmentLayout>,
    /// Every `moof` child in wire order.
    pub children: Vec<MoofChild>,
    /// The top-level boxes between the previous fragment's media and this
    /// `moof` (`styp`, `prft`, a `free` pad): everything the caller's own
    /// event/`styp` arguments do not already state.
    pub before: Vec<OpaqueBox>,
}

impl FragmentLayout {
    /// The layout a parsed `moof` states, recorded at the absolute offset it
    /// was read from so an explicit `base_data_offset` becomes relative.
    pub fn from_moof(moof: &MovieFragmentBox, moof_offset: u64) -> Result<Self> {
        let mut tracks = Vec::with_capacity(moof.traf.len());
        for traf in &moof.traf {
            tracks.push(TrackFragmentLayout::from_traf(traf, moof_offset)?);
        }
        Ok(Self {
            sequence_number: moof.mfhd.sequence_number,
            tracks,
            children: moof.order.clone(),
            before: Vec::new(),
        })
    }

    /// The `traf` a `track_id` states in this fragment.
    #[must_use]
    pub fn track(&self, track_id: TrackId) -> Option<&TrackFragmentLayout> {
        self.tracks.iter().find(|track| track.track_id == track_id)
    }

    /// The compact layout for `tracks`: `tfhd` sample defaults where every
    /// sample of a track agrees, `first_sample_flags` when only the first
    /// differs, and no per-sample field those make redundant.
    ///
    /// This is a caller's policy, not a default: a writer that is handed no
    /// layout keeps its canonical per-sample form.
    pub fn compact(tracks: &[crate::writer::FragmentTrack<'_>]) -> Result<Self> {
        let mut laid_out = Vec::with_capacity(tracks.len());
        for track in tracks {
            laid_out.push(TrackFragmentLayout::compact(track)?);
        }
        Ok(Self {
            sequence_number: 0,
            tracks: laid_out,
            children: Vec::new(),
            before: Vec::new(),
        })
    }
}

/// The width of each variable-width `tfra` field, as the wire states it: the
/// byte count **minus one** (§8.8.10.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TfraLengthSizes {
    /// `length_size_of_traf_num`.
    pub traf: u8,
    /// `length_size_of_trun_num`.
    pub trun: u8,
    /// `length_size_of_sample_num`.
    pub sample: u8,
}

/// One track's `tfra` entry in a recorded `mfra`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MfraTrackForm {
    /// `tfra.track_ID`.
    pub track_id: TrackId,
    /// The `tfra` version (§8.8.10.2): 0 makes `time`/`moof_offset` 32-bit.
    pub version: u8,
    /// The variable-field widths.
    pub lengths: TfraLengthSizes,
}

/// The form of a trailing `mfra`: one [`MfraTrackForm`] per `tfra`, in the
/// source's order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MfraForm {
    /// The `tfra`s to emit, in order.
    pub tracks: Vec<MfraTrackForm>,
}

/// The form of a `sidx` (§8.14.3) over the fragments that follow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SegmentIndexForm {
    /// `sidx` version: 0 makes the presentation time and first offset 32-bit.
    pub version: u8,
    /// `sidx` flags.
    pub flags: u32,
    /// `reference_ID`: the track whose timeline the durations are in.
    pub reference_id: TrackId,
    /// `timescale` of that timeline.
    pub timescale: Timescale,
    /// `earliest_presentation_time`.
    pub earliest_presentation_time: u64,
    /// `reference_type`: 0 for a media segment. A nested sub-segment index (1)
    /// is not reproduced.
    pub reference_type: u8,
}

impl SegmentIndexForm {
    /// The form an author's `sidx` takes over `reference_id`'s timeline.
    #[must_use]
    pub fn new(reference_id: TrackId, timescale: Timescale) -> Self {
        Self {
            version: 0,
            flags: 0,
            reference_id,
            timescale,
            earliest_presentation_time: 0,
            reference_type: 0,
        }
    }
}

/// The index boxes a fragmented file carries around its fragments: the
/// `sidx` that precedes them and the `mfra` that follows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FragmentIndex {
    /// The segment index form, when the file carries a `sidx`.
    pub sidx: Option<SegmentIndexForm>,
    /// The random-access form, when the file ends with an `mfra`.
    pub mfra: Option<MfraForm>,
}

impl TrackFragmentLayout {
    /// The compact layout for one track's samples: defaults in the `tfhd`
    /// wherever every sample agrees, `first_sample_flags` where only the first
    /// sample differs, and a composition offset only when one is non-zero.
    fn compact(track: &crate::writer::FragmentTrack<'_>) -> Result<Self> {
        let samples = track.samples;
        let mut sizes = Vec::with_capacity(samples.len());
        for sample in samples {
            sizes.push(u32::try_from(sample.data.len()).map_err(|_| {
                Error::InvalidInput("sample larger than the 32-bit trun sample_size field")
            })?);
        }
        let duration = uniform(samples.iter().map(|sample| sample.meta.duration));
        let size = uniform(sizes.iter().copied());
        let flags = uniform(samples.iter().map(|sample| sample_flags(sample.meta.sync)));
        // A run that states no per-sample flags takes its first sample's from
        // `first_sample_flags` and every later one from the `tfhd` default, so
        // "only the first differs" is still fully described.
        let first_flags = samples.first().map(|sample| sample_flags(sample.meta.sync));
        let later_flags = uniform(samples.iter().skip(1).map(|s| sample_flags(s.meta.sync)));
        let any_offset = samples
            .iter()
            .any(|sample| sample.meta.composition_offset != 0);
        let negative_offset = samples
            .iter()
            .any(|sample| sample.meta.composition_offset < 0);

        let mut tfhd = TrackFragmentHeaderLayout {
            flags: crate::movie_fragment::TFHD_DEFAULT_BASE_IS_MOOF,
            ..TrackFragmentHeaderLayout::default()
        };
        let mut fields = RunFields::default();
        match duration {
            Some(value) => tfhd.default_sample_duration = Some(value),
            None => fields.duration = true,
        }
        match size {
            Some(value) => tfhd.default_sample_size = Some(value),
            None => fields.size = true,
        }
        let mut first_sample_flags = None;
        match flags {
            Some(value) => tfhd.default_sample_flags = Some(value),
            None => match (first_flags, later_flags) {
                (Some(first), Some(later)) if first != later => {
                    tfhd.default_sample_flags = Some(later);
                    first_sample_flags = Some(first);
                }
                _ => fields.flags = true,
            },
        }
        fields.composition_offset = any_offset;
        Ok(Self {
            track_id: track.track_id,
            tfhd,
            tfdt_version: 1,
            runs: alloc::vec![RunLayout {
                version: if negative_offset { 1 } else { 0 },
                fields,
                data_offset: true,
                first_sample_flags,
                sample_count: samples.len(),
            }],
            children: Vec::new(),
        })
    }
}

/// The single value every entry of `values` has, or `None` when they differ
/// or there are none.
fn uniform<T: PartialEq + Copy>(mut values: impl Iterator<Item = T>) -> Option<T> {
    let first = values.next()?;
    values.all(|value| value == first).then_some(first)
}
