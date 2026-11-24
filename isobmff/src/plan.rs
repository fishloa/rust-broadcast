//! The progressive read plan: a parsed `moov`'s per-track tables, lent to
//! [`TrackCursor`] without any per-sample storage.
//!
//! `plan()` is the whole-file fact a progressive file already carries in its
//! index (streaming-first design §4.4, P8): sample timing, size and position
//! come from `stts`/`ctts`/`stsc`/`stsz`/`stco`, never from held samples.
//! [`SamplePlan`] borrows the parsed [`MovieBox`] a
//! [`Reader`](crate::reader::Reader) owns, so a cursor created here reads the
//! same tables the reader walks.
//!
//! Spec: ISO/IEC 14496-12:2026 §8.5-§8.7
//! (`private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`).

use crate::error::{Error, Result};
use crate::init_segment::{MovieBox, SampleDescriptionBox, SampleTableBox, StblChild, TrackBox};
use crate::limits::Limits;
use crate::reader::TrackInfo;
use crate::sample_table::TrackCursor;
use crate::track_id::TrackId;

/// A progressive file's sample tables, one cursor per track on demand.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct SamplePlan<'a> {
    movie: &'a MovieBox,
    tracks: &'a [TrackInfo],
    file_len: Option<u64>,
}

impl<'a> SamplePlan<'a> {
    pub(crate) fn new(movie: &'a MovieBox, tracks: &'a [TrackInfo], file_len: Option<u64>) -> Self {
        Self {
            movie,
            tracks,
            file_len,
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
        TrackCursor::new(stbl, track, limits, self.file_len)
    }
}

/// A track's sample table, when it has one.
pub(crate) fn sample_table(trak: &TrackBox) -> Option<&SampleTableBox> {
    trak.mdia.as_ref()?.minf.as_ref()?.stbl.as_ref()
}

/// A track's typed sample description, when it has one.
pub(crate) fn sample_description(stbl: &SampleTableBox) -> Option<&SampleDescriptionBox> {
    stbl.children.iter().find_map(|child| match child {
        StblChild::Stsd(stsd) => Some(stsd),
        _ => None,
    })
}
