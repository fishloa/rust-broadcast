//! The one input and output vocabulary every muxer [`Stage`]
//! speaks: a muxer is `Stage<In<'a> = MuxInput, Out = MuxOutput>`.
//!
//! A muxer holds at most one output unit (a fragment, a TS PES, an FLV tag,
//! a Matroska cluster, an RTP packet group). Manifest packagers never see
//! samples: they render from the [`SegmentSummary`] each [`MuxOutput::Segment`]
//! carries.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use broadcast_common::stage::{Stage, Timestamp};
use bytes::Bytes;

use crate::ir::{Media, Sample, SampleTiming, Track, TrackId, TrackSpec};

/// What a muxer stage is fed.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum MuxInput {
    /// A track is announced, or an announced track's setup changed.
    Track(Box<TrackSpec>),
    /// One coded sample of an announced track, in decode order per track.
    Sample {
        /// The announced track this sample belongs to.
        track: TrackId,
        /// The sample.
        sample: Sample,
    },
    /// The media timeline is discontinuous before the next sample of
    /// `track`, or of every track when `None`.
    Discontinuity {
        /// The track the discontinuity applies to; `None` for all tracks.
        track: Option<TrackId>,
    },
    /// Close the unit being built now, without waiting for its natural cut.
    Flush,
}

/// What a muxer stage produces.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MuxOutput {
    /// Bytes appended to the output stream.
    Bytes(Bytes),
    /// Bytes written over an earlier part of the output at `offset`; only a
    /// muxer writing to a seekable output emits this.
    BytesAt {
        /// Absolute byte offset in the output.
        offset: u64,
        /// The bytes to write there.
        bytes: Bytes,
    },
    /// An initialisation segment.
    Init(Bytes),
    /// One finished media segment and its summary.
    Segment(SegmentOut),
    /// A rendered playlist or manifest.
    Text(String),
}

/// One finished media segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentOut {
    /// The segment's bytes.
    pub bytes: Bytes,
    /// What a manifest needs to describe the segment as a whole.
    pub summary: SegmentSummary,
    /// One entry per track the segment carries, in the segment's track order:
    /// what a per-representation manifest needs (its own start, duration, byte
    /// count and sample count), which the segment-level summary cannot express.
    pub tracks: Vec<TrackPart>,
}

/// One track's share of a segment: the per-representation view a DASH or Smooth
/// manifest describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackPart {
    /// The track this part belongs to.
    pub track: TrackId,
    /// Decode time of this track's first sample in the segment, in
    /// [`Self::timescale`] ticks.
    pub start: u64,
    /// This track's duration in the segment, in [`Self::timescale`] ticks.
    pub duration: u64,
    /// Ticks per second of [`Self::start`] and [`Self::duration`].
    pub timescale: u32,
    /// This track's bytes in the segment.
    pub bytes: u64,
    /// The samples this track contributed to the segment.
    pub samples: u32,
}

/// What a manifest needs to know about one segment, without its samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentSummary {
    /// The track whose timeline cut the segment (the anchor track of a
    /// multiplexed segment).
    pub track: TrackId,
    /// 0-based, ever-increasing segment number for this output.
    pub sequence: u64,
    /// Decode time of the segment's first sample, in `timescale` ticks.
    pub start: u64,
    /// Segment duration, in `timescale` ticks.
    pub duration: u64,
    /// Ticks per second of `start` and `duration`.
    pub timescale: u32,
    /// Size of the segment's bytes.
    pub bytes: u64,
    /// The segment starts with a sync sample.
    pub independent: bool,
    /// The segment follows a timeline discontinuity.
    pub discontinuous: bool,
}

/// Feeds a whole [`Media`] to a muxer stage and collects what it produces.
///
/// Every track is announced first, then samples go in decode-time order
/// across tracks (ties to the earlier track), then the stage is finished.
/// This is the bridge for callers that still hold a whole [`Media`]; it goes
/// with [`Media`].
///
/// # Errors
///
/// Whatever the stage returns from `feed` or `finish`.
pub fn drive_media<S>(
    stage: &mut S,
    media: &Media,
    now: Timestamp,
) -> Result<Vec<MuxOutput>, S::Error>
where
    S: for<'a> Stage<In<'a> = MuxInput, Out = MuxOutput>,
{
    let mut out = Vec::new();
    for track in &media.tracks {
        stage.feed(MuxInput::Track(Box::new(track.spec.clone())), now)?;
        drain(stage, &mut out);
    }
    let mut cursors: Vec<DecodeCursor> = media.tracks.iter().map(DecodeCursor::new).collect();
    while let Some(index) = earliest(&cursors) {
        let cursor = &mut cursors[index];
        let track = &media.tracks[index];
        let sample = track.samples[cursor.next].clone();
        cursor.advance(&sample);
        stage.feed(
            MuxInput::Sample {
                track: track.spec.track_id,
                sample,
            },
            now,
        )?;
        drain(stage, &mut out);
    }
    stage.finish()?;
    drain(stage, &mut out);
    Ok(out)
}

fn drain<S>(stage: &mut S, out: &mut Vec<MuxOutput>)
where
    S: for<'a> Stage<In<'a> = MuxInput, Out = MuxOutput>,
{
    while let Some(item) = stage.poll() {
        out.push(item);
    }
}

struct DecodeCursor<'m> {
    samples: &'m [Sample],
    timescale: i128,
    next: usize,
    decode: i64,
}

impl<'m> DecodeCursor<'m> {
    fn new(track: &'m Track) -> Self {
        Self {
            samples: &track.samples,
            timescale: i128::from(track.spec.timescale),
            next: 0,
            decode: i64::try_from(track.start_decode_time).unwrap_or(i64::MAX),
        }
    }

    fn decode_time(&self) -> Option<i64> {
        let sample = self.samples.get(self.next)?;
        Some(match sample.timing {
            Some(SampleTiming::Timed { dts, .. }) => dts,
            _ => self.decode,
        })
    }

    fn advance(&mut self, sample: &Sample) {
        let start = self.decode_time().unwrap_or(self.decode);
        let duration = match sample.timing {
            Some(SampleTiming::Timed { duration, .. }) => {
                duration.map_or(0, |d| i64::from(d.get()))
            }
            Some(SampleTiming::DurationOnly { duration }) => i64::from(duration.get()),
            _ => 0,
        };
        self.decode = start.saturating_add(duration);
        self.next += 1;
    }
}

fn earliest(cursors: &[DecodeCursor<'_>]) -> Option<usize> {
    let mut best: Option<(usize, i128, i128)> = None;
    for (index, cursor) in cursors.iter().enumerate() {
        let Some(time) = cursor.decode_time() else {
            continue;
        };
        let time = i128::from(time);
        let earlier = best.is_none_or(|(_, best_time, best_scale)| {
            time * best_scale < best_time * cursor.timescale
        });
        if earlier {
            best = Some((index, time, cursor.timescale));
        }
    }
    best.map(|(index, ..)| index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{CodecConfig, DataCarriage, DataIdentity, SampleFlags};
    use alloc::vec;
    use broadcast_common::timescale::Timescale;

    #[derive(Default)]
    struct Recorder {
        fed: Vec<(TrackId, i64)>,
        announced: usize,
        finished: bool,
        out: Vec<MuxOutput>,
    }

    impl Stage for Recorder {
        type In<'a> = MuxInput;
        type Out = MuxOutput;
        type Error = core::convert::Infallible;

        fn feed(&mut self, input: MuxInput, _now: Timestamp) -> Result<(), Self::Error> {
            match input {
                MuxInput::Track(_) => self.announced += 1,
                MuxInput::Sample { track, sample } => {
                    let Some(SampleTiming::Timed { dts, .. }) = sample.timing else {
                        unreachable!("test samples are timed");
                    };
                    self.fed.push((track, dts));
                    self.out.push(MuxOutput::Bytes(sample.data));
                }
                _ => {}
            }
            Ok(())
        }

        fn poll(&mut self) -> Option<MuxOutput> {
            self.out.pop()
        }

        fn finish(&mut self) -> Result<(), Self::Error> {
            self.finished = true;
            Ok(())
        }
    }

    fn track(id: u32, timescale: u32, dts: &[i64]) -> Track {
        let spec = TrackSpec::new(
            TrackId::new(id),
            Timescale::try_new(timescale).expect("non-zero"),
            CodecConfig::Data {
                identity: DataIdentity::Ts {
                    stream_type: 6,
                    descriptors: Vec::new(),
                    carriage: DataCarriage::Pes,
                },
            },
        );
        let samples = dts
            .iter()
            .map(|&d| Sample {
                data: vec![0u8; 1].into(),
                timing: Some(SampleTiming::timed(d, d)),
                flags: SampleFlags::SYNC,
                provenance: None,
                display: None,
                protection: None,
            })
            .collect();
        Track::new(spec, samples)
    }

    #[test]
    fn drive_media_interleaves_by_decode_time_across_timescales() {
        let video = track(1, 90_000, &[0, 3_000, 6_000]);
        let audio = track(2, 48_000, &[0, 1_024, 2_048, 3_072]);
        let media = Media::new(
            vec![video, audio],
            Timescale::try_new(1_000).expect("non-zero"),
        );
        let mut stage = Recorder::default();
        let out = drive_media(&mut stage, &media, Timestamp::default()).expect("infallible");
        let v = TrackId::new(1);
        let a = TrackId::new(2);
        assert_eq!(stage.announced, 2);
        assert!(stage.finished);
        assert_eq!(out.len(), 7);
        assert_eq!(
            stage.fed,
            vec![
                (v, 0),
                (a, 0),
                (a, 1_024),
                (v, 3_000),
                (a, 2_048),
                (a, 3_072),
                (v, 6_000)
            ]
        );
    }
}
