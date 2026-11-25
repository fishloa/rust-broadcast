//! Progressive (single-file, non-fragmented) MP4 muxer — ISO/IEC 14496-12:2026
//! §8.
//!
//! [`ProgressiveMuxStage`] writes one non-fragmented `.mp4`: an `ftyp`, a
//! single `moov` carrying per-track `trak` boxes with the *full* sample tables
//! (`stbl`: `stsd`/`stts`/`ctts`/`stsc`/`stsz`/`stco`|`co64`/`stss`,
//! ISO/IEC 14496-12:2026 §8.5-§8.7), and a single `mdat` holding every track's
//! sample data. It is the VOD/download counterpart to the fragmented
//! [`crate::media::CmafMuxStage`].
//!
//! The whole layout is fixed up front from an
//! `OwnedPlan`, which
//! [`crate::plan_pass::PlanPass`] records from a first pass over the same
//! input: chunk boundaries, the cross-track interleave, `stco`/`co64`, the
//! `mdat` header and every sample's absolute offset. So no sample is held: the
//! muxer emits `ftyp` plus `moov` (fast start) or `ftyp` plus the `mdat` header
//! (mdat first) before the first sample, then each sample's bytes as it
//! arrives, then (mdat first) `moov` at [`Stage::finish`].
//!
//! A sample whose size or position disagrees with the plan is
//! [`Error::PlanMismatch`], naming the track and sample index, before any of
//! its bytes are written.
//!
//! Spec: ISO/IEC 14496-12:2026 §8.5-§8.7
//! (`private/specs-verified/mpeg/isobmff/iso-iec-14496-12-2026.md`).

use alloc::borrow::Cow;
use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use broadcast_common::stage::{Stage, Timestamp};
use bytes::Bytes;

use crate::error::{Error, Result};
use crate::ir::{Sample, TrackId};
use crate::mux_stage::{MuxInput, MuxOutput};
use isobmff::plan::OwnedPlan;
use isobmff::writer::{FileType, Layout, MovieHeader, ProgressiveWriter, Writer};
use isobmff::{Limits, TrackId as IsoTrackId};

/// `ftyp` major brand for a progressive (non-fragmented) MP4.
const FTYP_MAJOR_BRAND: [u8; 4] = *b"isom";
/// `ftyp` minor version.
const FTYP_MINOR_VERSION: u32 = 512;

/// Mux a sample plan into one progressive (non-fragmented) `.mp4` as a
/// streaming [`Stage`].
///
/// `plan` is the [`OwnedPlan`] a [`PlanPass`](crate::plan_pass::PlanPass)
/// recorded from the same input; `layout` carries the caller's box order, chunk
/// length and sink capability; `movie` is the `mvhd` the plan's tables do not
/// state. Announce the plan's tracks, feed each track's samples in decode
/// order, and write every [`MuxOutput::Bytes`] appended and every
/// [`MuxOutput::BytesAt`] at its offset (a sample may arrive out of file order
/// under a seekable sink).
pub struct ProgressiveMuxStage {
    plan: Option<OwnedPlan>,
    layout: Layout,
    movie: MovieHeader,
    writer: Option<ProgressiveWriter>,
    /// Samples pushed per plan track, in plan order: the index the next sample
    /// of that track must carry.
    counts: Vec<u32>,
    /// Bytes the output stream holds so far, so a write at exactly this offset
    /// is an append.
    appended: u64,
    finished: bool,
    ready: VecDeque<MuxOutput>,
}

impl core::fmt::Debug for ProgressiveMuxStage {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProgressiveMuxStage")
            .field("layout", &self.layout)
            .field("movie", &self.movie)
            .field("started", &self.writer.is_some())
            .field("appended", &self.appended)
            .finish_non_exhaustive()
    }
}

impl ProgressiveMuxStage {
    /// A muxer over `plan`, laid out as `layout` states and headed by `movie`.
    #[must_use]
    pub fn new(plan: OwnedPlan, layout: Layout, movie: MovieHeader) -> Self {
        Self {
            plan: Some(plan),
            layout,
            movie,
            writer: None,
            counts: Vec::new(),
            appended: 0,
            finished: false,
            ready: VecDeque::new(),
        }
    }

    fn ensure_writer(&mut self) -> Result<()> {
        if self.writer.is_some() {
            return Ok(());
        }
        let plan = self
            .plan
            .take()
            .ok_or(Error::InvalidInput("progressive: the plan is gone"))?;
        if plan.tracks().is_empty() {
            return Err(Error::InvalidInput("cannot package a Media with no tracks"));
        }
        self.counts = vec![0; plan.tracks().len()];
        let writer = Writer::progressive(
            plan,
            self.layout,
            file_type(),
            self.movie,
            Limits::default(),
        )?;
        for write in writer.header() {
            let bytes = Bytes::copy_from_slice(&write.bytes);
            self.emit(write.offset, bytes);
        }
        self.writer = Some(writer);
        Ok(())
    }

    fn sample(&mut self, track: TrackId, sample: Sample) -> Result<()> {
        self.ensure_writer()?;
        // The writer is moved out for the push so the emitted writes can be
        // handed to `emit` (which borrows the stage) while the writer's own
        // borrow of the sample is live, and put back whatever the outcome.
        let mut writer = self
            .writer
            .take()
            .ok_or(Error::InvalidInput("progressive: no writer"))?;
        let result = self.push_sample(&mut writer, track, &sample);
        self.writer = Some(writer);
        result
    }

    /// Place one sample through `writer`, sharing its bytes rather than copying
    /// them: the sample's `Bytes` already owns the payload.
    fn push_sample(
        &mut self,
        writer: &mut ProgressiveWriter,
        track: TrackId,
        sample: &Sample,
    ) -> Result<()> {
        let iso_track = IsoTrackId::new(track.value());
        let (track_index, index, expected) = {
            let plan = writer.plan();
            let track_index = plan.index_of(iso_track).ok_or(Error::InvalidInput(
                "progressive: sample for a track the plan does not have",
            ))?;
            let planned = plan
                .tracks()
                .get(track_index)
                .ok_or(Error::InvalidInput("progressive: plan track is missing"))?;
            let index = self
                .counts
                .get(track_index)
                .copied()
                .ok_or(Error::InvalidInput("progressive: plan track is missing"))?;
            if index >= planned.tables.sample_count {
                return Err(Error::PlanMismatch {
                    track,
                    index,
                    what: "sample count",
                    planned: u64::from(planned.tables.sample_count),
                    got: u64::from(index) + 1,
                });
            }
            let expected = planned.tables.size(index)?;
            (track_index, index, expected)
        };
        let got = u64::try_from(sample.data.len()).unwrap_or(u64::MAX);
        if got != u64::from(expected) {
            return Err(Error::PlanMismatch {
                track,
                index,
                what: "size",
                planned: u64::from(expected),
                got,
            });
        }
        if let Some(count) = self.counts.get_mut(track_index) {
            *count = index.saturating_add(1);
        }
        let writes = writer.push(iso_track, index, &sample.data)?;
        let source = sample.data.as_ptr();
        let len = sample.data.len();
        for write in writes {
            // A seekable sink (and an in-turn append) hands the sample back
            // borrowing the very buffer the stage was given, so the output can
            // share it. Only a reordered or buffered write needs a copy.
            let shared = matches!(
                &write.bytes,
                Cow::Borrowed(bytes) if bytes.len() == len && core::ptr::eq(bytes.as_ptr(), source)
            );
            let bytes = if shared {
                sample.data.clone()
            } else {
                Bytes::copy_from_slice(&write.bytes)
            };
            self.emit(write.offset, bytes);
        }
        Ok(())
    }

    fn emit(&mut self, offset: u64, bytes: Bytes) {
        let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if offset == self.appended {
            self.appended = self.appended.saturating_add(len);
            self.ready.push_back(MuxOutput::Bytes(bytes));
        } else {
            self.appended = self.appended.max(offset.saturating_add(len));
            self.ready.push_back(MuxOutput::BytesAt { offset, bytes });
        }
    }
}

impl Stage for ProgressiveMuxStage {
    type In<'a> = MuxInput;
    type Out = MuxOutput;
    type Error = Error;

    fn feed(&mut self, input: MuxInput, _now: Timestamp) -> Result<()> {
        match input {
            MuxInput::Track(_) => {
                self.ensure_writer()?;
                Ok(())
            }
            MuxInput::Sample { track, sample } => self.sample(track, sample),
            MuxInput::Discontinuity { .. } => Err(Error::UnsupportedFeature(
                "progressive: a single-file MP4 cannot signal a discontinuity",
            )),
            MuxInput::Flush => Ok(()),
        }
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.ensure_writer()?;
        let mut writer = self
            .writer
            .take()
            .ok_or(Error::InvalidInput("progressive: no writer"))?;
        let mut missing = None;
        for (index, planned) in writer.plan().tracks().iter().enumerate() {
            let got = self.counts.get(index).copied().unwrap_or(0);
            if got < planned.tables.sample_count {
                missing = Some((
                    TrackId::from(planned.setup.track_id),
                    got,
                    planned.tables.sample_count,
                ));
                break;
            }
        }
        if let Some((track, index, planned)) = missing {
            self.writer = Some(writer);
            return Err(Error::PlanMismatch {
                track,
                index,
                what: "missing sample",
                planned: u64::from(planned),
                got: u64::from(index),
            });
        }
        let trailer = match writer.finish() {
            Ok(trailer) => trailer,
            Err(error) => {
                self.writer = Some(writer);
                return Err(error.into());
            }
        };
        for write in trailer {
            let bytes = Bytes::copy_from_slice(&write.bytes);
            self.emit(write.offset, bytes);
        }
        self.writer = Some(writer);
        Ok(())
    }
}

fn file_type() -> FileType {
    FileType {
        major_brand: FTYP_MAJOR_BRAND,
        minor_version: FTYP_MINOR_VERSION,
        compatible_brands: vec![*b"isom", *b"iso2", *b"mp41", *b"avc1"],
        emit_ftyp: true,
    }
}
