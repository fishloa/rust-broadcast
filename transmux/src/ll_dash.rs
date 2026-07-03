//! Low-latency DASH — chunked CMAF packaging + LL-DASH MPD signalling.
//!
//! Whole-segment CMAF ([`crate::segmenter::Segmenter`], [`crate::media::CmafMux`])
//! delivers one `moof`+`mdat` per segment: a client cannot start playing a
//! segment until the whole thing has been produced, so end-to-end latency is at
//! least one segment duration. **Low-latency DASH** (DASH-IF Low-latency Live
//! Interoperability, "LL-DASH IOP") cuts that by splitting each CMAF segment
//! into several **CMAF chunks** — each a self-contained `moof`+`mdat` covering a
//! sub-run of the segment's samples — delivered (over HTTP chunked transfer) as
//! they are produced. The client can begin decoding the first chunk while the
//! rest of the segment is still being generated.
//!
//! # Chunked-CMAF structure (ISO/IEC 14496-12:2015)
//!
//! A CMAF chunk is exactly the fragment structure the batch builder
//! [`crate::build_media_segment`] already emits, but scoped to K samples:
//!
//! - the **first chunk** of a segment carries the segment-type box
//!   `styp` (§8.16.4) followed by `moof` (§8.8.4) + `mdat` (§8.1.1);
//! - **subsequent chunks** are a bare `moof` + `mdat`.
//!
//! Each chunk's `moof` has its own `mfhd.sequence_number` (§8.8.5) — contiguous
//! and increasing across all chunks of all segments — and each track fragment's
//! `tfdt.baseMediaDecodeTime` (§8.8.12) is the decode time of that chunk's first
//! sample. Concatenating a segment's chunks yields the same coded samples, in
//! the same decode order, with the same per-track decode timeline, as the
//! whole-segment [`crate::build_media_segment`] output — chunking splits, it
//! never loses, duplicates, or reorders a sample. Segment boundaries stay
//! keyframe-aligned (the anchor track's first chunk of every segment starts on a
//! sync sample), exactly as [`crate::segmenter::Segmenter`] guarantees.
//!
//! # LL-DASH MPD signalling (ISO/IEC 23009-1 + DASH-IF LL IOP)
//!
//! [`LlDashPackager`] renders an MPD that advertises the chunked availability so
//! a player can request a segment before it is complete:
//!
//! - **`SegmentTemplate@availabilityTimeComplete="false"`** (ISO/IEC 23009-1
//!   §5.3.9.5.3) — a segment becomes available (its first chunk) before the whole
//!   segment is produced.
//! - **`SegmentTemplate@availabilityTimeOffset`** (§5.3.9.5.3) — how many seconds
//!   *before* the nominal segment-complete time the segment first becomes
//!   available. Per DASH-IF LL IOP this is `segment_duration − chunk_duration`
//!   (the first chunk is ready one chunk-duration into the segment).
//! - **`MPD/ServiceDescription/Latency@target`** (§5.13.2) — the target
//!   end-to-end latency in milliseconds, with an optional `<PlaybackRate>` giving
//!   the min/max catch-up rate.
//! - `MPD@type="dynamic"` with `@availabilityStartTime` (§5.3.1.2) — LL-DASH is a
//!   live profile.
//!
//! `<ProducerReferenceTime>` (§5.12) is **optional** and is *not* emitted by this
//! packager: it requires a wall-clock capture timestamp per segment that the
//! samples-in IR does not carry. A caller with that timing can add it out of
//! band; its absence does not affect the availability signalling above.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::dash::DashPackager;
use crate::error::{Error, Result};
use crate::media::Media;
use crate::movie_fragment::{
    MovieFragmentBox, MovieFragmentHeaderBox, TFHD_DEFAULT_BASE_IS_MOOF, TRUN_DATA_OFFSET_PRESENT,
    TRUN_SAMPLE_COMPOSITION_TIME_OFFSET_PRESENT, TRUN_SAMPLE_DURATION_PRESENT,
    TRUN_SAMPLE_FLAGS_PRESENT, TRUN_SAMPLE_SIZE_PRESENT, TrackFragmentBaseMediaDecodeTimeBox,
    TrackFragmentBox, TrackFragmentHeaderBox, TrackFragmentRunBox, TrunSample,
};
use crate::pipeline::{CodecConfig, FragmentTrackData, Sample, TrackSpec, build_init_segment};
use crate::segments::{MediaDataBox, SegmentTypeBox};
use broadcast_common::{Package, Serialize};

// --- sample_flags (ISO/IEC 14496-12:2015 §8.8.3.1) --------------------------
// Mirror the private constants used by `build_media_segment` so a chunk's trun
// carries byte-identical flags (a segment's chunks == the whole segment).
/// Sample flags for a sync sample: `sample_depends_on = 2`, non-sync = 0.
const SAMPLE_FLAGS_SYNC: u32 = 0x0200_0000;
/// Sample flags for a non-sync sample: `sample_depends_on = 1`, non-sync = 1.
const SAMPLE_FLAGS_NON_SYNC: u32 = 0x0101_0000;

/// `styp` major brand for a low-latency CMAF chunk (§8.16.4). Matches the brand
/// `build_media_segment` writes so chunk concat == whole segment.
const STYP_MAJOR_BRAND: [u8; 4] = *b"msdh";
/// `styp` compatible brands.
const STYP_COMPATIBLE_BRANDS: [[u8; 4]; 2] = [*b"msdh", *b"msix"];

// ===========================================================================
// LlSegmenter — chunked-CMAF output packager
// ===========================================================================

/// One finished CMAF chunk: its bytes plus the metadata a caller needs to place
/// it (which segment it belongs to, whether it opens that segment).
#[derive(Debug, Clone)]
pub struct Chunk {
    /// The chunk bytes: `styp`+`moof`+`mdat` for the first chunk of a segment,
    /// or a bare `moof`+`mdat` for a continuation chunk.
    pub data: Vec<u8>,
    /// 1-based index of the segment this chunk belongs to.
    pub segment_number: u64,
    /// `true` if this is the first chunk of its segment (carries `styp`, and its
    /// anchor-track first sample is a keyframe).
    pub is_segment_start: bool,
    /// `mfhd.sequence_number` of this chunk's `moof`.
    pub sequence_number: u32,
}

/// Per-track accumulation state for the segment currently being built.
struct TrackState {
    spec: TrackSpec,
    /// Samples buffered for the current (not-yet-cut) segment, in decode order.
    pending: Vec<Sample>,
    /// Decode time of the first *pending* sample (media-timescale ticks) — the
    /// `tfdt.baseMediaDecodeTime` of this track's next chunk.
    base_decode: u64,
}

/// A stateful **chunked** CMAF segmenter for low-latency DASH.
///
/// Same segmentation state machine as [`crate::segmenter::Segmenter`] — segments
/// are cut on the anchor track's sync samples once the target duration is reached
/// — but each segment is emitted as an ordered list of [`Chunk`]s: one
/// `moof`+`mdat` per `chunk_samples` anchor-track samples (e.g. `1` for per-frame
/// LL). Concatenating a segment's chunks reproduces the whole-segment
/// [`crate::build_media_segment`] coded-sample stream (see the module docs).
///
/// ```
/// use transmux::{CodecConfig, LlSegmenter, Sample, TrackSpec};
/// # fn spec() -> TrackSpec { unimplemented!() }
/// # fn au(sync: bool) -> Sample { Sample::from_raw(vec![0u8; 4], 3000) }
/// # if false {
/// // 2 s target segments, one video frame per chunk (per-frame LL).
/// let mut seg = LlSegmenter::new(vec![spec()], 1000, 2.0, 1).unwrap();
/// let init = seg.init_segment().unwrap();       // ftyp + moov
/// seg.push(1, au(true)).unwrap();               // keyframe
/// seg.push(1, au(false)).unwrap();
/// for chunk in seg.take_ready() { /* write chunk.data over HTTP */ }
/// seg.flush().unwrap();                         // trailing chunks
/// # }
/// ```
pub struct LlSegmenter {
    tracks: Vec<TrackState>,
    movie_timescale: u32,
    /// Index into `tracks` of the segmentation anchor (keyframe cut boundary).
    anchor: usize,
    /// Target segment duration in the *anchor track's* media timescale.
    target_ticks: u64,
    /// Number of anchor-track samples per chunk (>= 1).
    chunk_samples: usize,
    /// Buffered duration of the anchor's `pending` samples (media-timescale ticks).
    anchor_pending_dur: u64,
    /// `mfhd.sequence_number` of the next chunk, 1-based and contiguous.
    next_seq: u32,
    /// 1-based number of the segment currently being built.
    current_segment: u64,
    /// `true` until the first chunk of the current segment has been emitted.
    segment_open: bool,
    /// Chunks finished but not yet taken by the caller.
    ready: Vec<Chunk>,
}

impl LlSegmenter {
    /// Create a chunked segmenter for `tracks`, cutting segments roughly every
    /// `target_duration_secs` on the anchor track's keyframes, and subdividing
    /// each segment into chunks of `chunk_samples` anchor-track samples.
    ///
    /// The anchor is the first video track (falling back to the first track for
    /// audio-only), exactly as [`crate::segmenter::Segmenter`]. `movie_timescale`
    /// matches [`build_init_segment`].
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `tracks` is empty, has duplicate `track_id`s,
    /// `target_duration_secs` is not positive and finite, or `chunk_samples == 0`.
    pub fn new(
        tracks: Vec<TrackSpec>,
        movie_timescale: u32,
        target_duration_secs: f64,
        chunk_samples: usize,
    ) -> Result<Self> {
        if tracks.is_empty() {
            return Err(Error::InvalidInput("ll segmenter needs at least one track"));
        }
        if !(target_duration_secs.is_finite() && target_duration_secs > 0.0) {
            return Err(Error::InvalidInput(
                "target_duration_secs must be positive and finite",
            ));
        }
        if chunk_samples == 0 {
            return Err(Error::InvalidInput("chunk_samples must be >= 1"));
        }
        for (i, a) in tracks.iter().enumerate() {
            if tracks[i + 1..].iter().any(|b| b.track_id == a.track_id) {
                return Err(Error::InvalidInput("duplicate track_id"));
            }
        }

        let anchor = tracks
            .iter()
            .position(|t| matches!(t.config, CodecConfig::Avc { .. }))
            .unwrap_or(0);

        let anchor_timescale = tracks[anchor].timescale as f64;
        let target_ticks = ((target_duration_secs * anchor_timescale) as u64).max(1);

        let tracks = tracks
            .into_iter()
            .map(|spec| TrackState {
                spec,
                pending: Vec::new(),
                base_decode: 0,
            })
            .collect();

        Ok(Self {
            tracks,
            movie_timescale,
            anchor,
            target_ticks,
            chunk_samples,
            anchor_pending_dur: 0,
            next_seq: 1,
            current_segment: 1,
            segment_open: false,
            ready: Vec::new(),
        })
    }

    /// The initialization segment (`ftyp` + fragmented-init `moov`). Stable for
    /// the life of the segmenter; write it once before any chunk.
    pub fn init_segment(&self) -> Result<Vec<u8>> {
        let specs: Vec<TrackSpec> = self.tracks.iter().map(|t| t.spec.clone()).collect();
        build_init_segment(&specs, self.movie_timescale)
    }

    /// Push one coded sample for `track_id`, in decode order.
    ///
    /// When the anchor track reaches a sync sample past the target duration, the
    /// current segment is finalized (all its remaining buffered samples flushed
    /// as a final chunk) *before* this sample is buffered, so the new keyframe
    /// opens the next segment's first chunk on a random-access point.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if `track_id` matches no track, or a chunk build
    /// fails.
    pub fn push(&mut self, track_id: u32, sample: Sample) -> Result<()> {
        let idx = self
            .tracks
            .iter()
            .position(|t| t.spec.track_id == track_id)
            .ok_or(Error::InvalidInput("push: unknown track_id"))?;

        // Segment boundary: anchor keyframe past target → flush the segment.
        if idx == self.anchor
            && sample.is_sync
            && self.anchor_pending_dur >= self.target_ticks
            && !self.tracks[self.anchor].pending.is_empty()
        {
            self.finish_segment()?;
        }

        if idx == self.anchor {
            self.anchor_pending_dur += sample.duration as u64;
        }
        self.tracks[idx].pending.push(sample);

        // Emit a chunk once the anchor has buffered a full chunk's worth of
        // samples (but hold the segment's last chunk until the boundary/flush so
        // audio that trails the last video frame stays in the same segment).
        if idx == self.anchor
            && self.tracks[self.anchor].pending.len() >= self.chunk_samples
            && self.anchor_pending_dur < self.target_ticks
        {
            self.emit_chunk(false)?;
        }
        Ok(())
    }

    /// Finalize the trailing segment at end-of-stream. A no-op if nothing is
    /// buffered. Any remaining chunks are appended to the ready queue.
    ///
    /// # Errors
    /// Propagates a chunk-build failure.
    pub fn flush(&mut self) -> Result<()> {
        if self.tracks.iter().any(|t| !t.pending.is_empty()) {
            self.finish_segment()?;
        }
        Ok(())
    }

    /// Remove and return every chunk finished since the last call, in order.
    pub fn take_ready(&mut self) -> Vec<Chunk> {
        core::mem::take(&mut self.ready)
    }

    /// Flush the whole current segment: emit all remaining buffered anchor
    /// samples (and every non-anchor sample) as chunks, then open the next
    /// segment. The final chunk of a segment carries any trailing samples.
    fn finish_segment(&mut self) -> Result<()> {
        // Emit whole chunks of anchor samples first, then a final chunk for the
        // remainder (which also carries all non-anchor buffered samples).
        while self.tracks[self.anchor].pending.len() > self.chunk_samples {
            self.emit_chunk(false)?;
        }
        // Final chunk of the segment (anchor remainder + all other tracks).
        self.emit_chunk(true)?;

        self.current_segment += 1;
        self.segment_open = false;
        self.anchor_pending_dur = 0;
        Ok(())
    }

    /// Emit one chunk. If `final_chunk`, drain every buffered sample of every
    /// track; otherwise drain `chunk_samples` anchor samples and no other track's
    /// samples (non-anchor samples ride the segment's final chunk, matching the
    /// whole-segment layout's single per-track run).
    fn emit_chunk(&mut self, final_chunk: bool) -> Result<()> {
        let is_start = !self.segment_open;
        let anchor = self.anchor;

        // Decide, per track, how many leading pending samples this chunk drains.
        let take_counts: Vec<usize> = self
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                if final_chunk {
                    t.pending.len()
                } else if i == anchor {
                    self.chunk_samples.min(t.pending.len())
                } else {
                    0
                }
            })
            .collect();

        // Nothing to emit (e.g. flush with only-empty tracks) → skip.
        if take_counts.iter().all(|&n| n == 0) {
            return Ok(());
        }

        let seq = self.next_seq;
        let chunk_bytes = {
            let frags: Vec<FragmentTrackData<'_>> = self
                .tracks
                .iter()
                .zip(&take_counts)
                .filter(|&(_, &n)| n > 0)
                .map(|(t, &n)| FragmentTrackData {
                    track_id: t.spec.track_id,
                    base_media_decode_time: t.base_decode,
                    samples: &t.pending[..n],
                })
                .collect();
            build_chunk(seq, &frags, is_start)?
        };

        // Advance decode times and drop the drained samples.
        for (t, &n) in self.tracks.iter_mut().zip(&take_counts) {
            let dur: u64 = t.pending[..n].iter().map(|s| s.duration as u64).sum();
            t.base_decode += dur;
            t.pending.drain(..n);
        }

        self.next_seq += 1;
        self.segment_open = true;
        self.ready.push(Chunk {
            data: chunk_bytes,
            segment_number: self.current_segment,
            is_segment_start: is_start,
            sequence_number: seq,
        });
        Ok(())
    }
}

impl Package for LlSegmenter {
    type Media = Media;
    /// The ordered chunk list for the whole media (drained via the state machine
    /// in one shot). Streaming callers use [`LlSegmenter::push`] /
    /// [`LlSegmenter::take_ready`] instead.
    type Output = Vec<Chunk>;
    type Error = Error;

    /// Package a complete [`Media`] into its full ordered chunk list.
    ///
    /// Samples are pushed **interleaved by decode time** across tracks so each
    /// segment carries the audio temporally aligned to its video (a per-track
    /// batch push would dump all audio into segment 1). At equal decode times the
    /// non-anchor tracks are pushed *before* the anchor, so a segment-boundary
    /// keyframe cut sees that segment's audio already buffered. Then flush.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] on an empty track list, or a chunk-build failure.
    fn package(&mut self, media: &Media) -> Result<Vec<Chunk>> {
        if media.tracks.is_empty() {
            return Err(Error::InvalidInput("cannot package a Media with no tracks"));
        }
        let anchor_id = self.tracks[self.anchor].spec.track_id;

        // A cursor per input track: next-sample index + its decode time in
        // *seconds* (cross-track comparable despite differing timescales),
        // accumulated from sample durations.
        struct Cursor<'a> {
            track_id: u32,
            timescale: u64,
            samples: &'a [Sample],
            idx: usize,
            dts_ticks: u64,
            is_anchor: bool,
        }
        let mut cursors: Vec<Cursor<'_>> = media
            .tracks
            .iter()
            .map(|t| Cursor {
                track_id: t.spec.track_id,
                timescale: (t.spec.timescale as u64).max(1),
                samples: &t.samples,
                idx: 0,
                dts_ticks: 0,
                is_anchor: t.spec.track_id == anchor_id,
            })
            .collect();

        // Merge: repeatedly push the track whose next sample has the smallest
        // decode time (seconds = dts_ticks / timescale, compared cross-multiplied
        // to stay integer). Anchor loses ties so audio at the same instant is
        // buffered first.
        loop {
            let mut best: Option<usize> = None;
            for (i, c) in cursors.iter().enumerate() {
                if c.idx >= c.samples.len() {
                    continue;
                }
                best = Some(match best {
                    None => i,
                    Some(b) => {
                        // c.dts (sec) < best.dts (sec)  ⇔  c.ticks*best.ts < best.ticks*c.ts
                        let lhs = c.dts_ticks as u128 * cursors[b].timescale as u128;
                        let rhs = cursors[b].dts_ticks as u128 * c.timescale as u128;
                        if lhs < rhs || (lhs == rhs && !c.is_anchor && cursors[b].is_anchor) {
                            i
                        } else {
                            b
                        }
                    }
                });
            }
            let Some(i) = best else { break };
            let (track_id, sample) = {
                let c = &mut cursors[i];
                let s = c.samples[c.idx].clone();
                c.dts_ticks += s.duration as u64;
                c.idx += 1;
                (c.track_id, s)
            };
            self.push(track_id, sample)?;
        }

        self.flush()?;
        Ok(self.take_ready())
    }
}

/// Build one CMAF **chunk** (`[styp] moof mdat`) over a sub-run of samples,
/// mirroring [`crate::build_media_segment`] exactly (same `trun` flags, same
/// `default-base-is-moof` offset resolution) so a segment's chunks concatenate to
/// the whole-segment coded-sample stream. `with_styp` gates the leading `styp`
/// (present on the first chunk of a segment only).
///
/// Shared with the LL-HLS **partial-segment** builder
/// ([`crate::ll_hls`]): an LL-HLS part is exactly this fragment structure scoped
/// to a sub-duration, so both layers emit byte-compatible `moof`+`mdat`.
pub(crate) fn build_chunk(
    sequence_number: u32,
    tracks: &[FragmentTrackData<'_>],
    with_styp: bool,
) -> Result<Vec<u8>> {
    let styp = with_styp.then(|| SegmentTypeBox {
        major_brand: STYP_MAJOR_BRAND,
        minor_version: 0,
        compatible_brands: STYP_COMPATIBLE_BRANDS.to_vec(),
    });

    let mut traf_boxes = Vec::with_capacity(tracks.len());
    for ft in tracks {
        let any_cts = ft.samples.iter().any(|s| s.composition_offset != 0);
        let samples: Vec<TrunSample> = ft
            .samples
            .iter()
            .map(|s| TrunSample {
                sample_duration: Some(s.duration),
                sample_size: Some(s.data.len() as u32),
                sample_flags: Some(if s.is_sync {
                    SAMPLE_FLAGS_SYNC
                } else {
                    SAMPLE_FLAGS_NON_SYNC
                }),
                sample_composition_time_offset: if any_cts {
                    Some(s.composition_offset)
                } else {
                    None
                },
            })
            .collect();

        let mut tr_flags = TRUN_DATA_OFFSET_PRESENT
            | TRUN_SAMPLE_DURATION_PRESENT
            | TRUN_SAMPLE_SIZE_PRESENT
            | TRUN_SAMPLE_FLAGS_PRESENT;
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
            samples,
        };
        let tfhd = TrackFragmentHeaderBox {
            flags: TFHD_DEFAULT_BASE_IS_MOOF,
            track_id: ft.track_id,
            base_data_offset: None,
            sample_description_index: None,
            default_sample_duration: None,
            default_sample_size: None,
            default_sample_flags: None,
        };
        let tfdt = TrackFragmentBaseMediaDecodeTimeBox::new_v1(ft.base_media_decode_time);
        traf_boxes.push(TrackFragmentBox {
            tfhd,
            tfdt: Some(tfdt),
            trun: vec![trun],
        });
    }

    let mut moof = MovieFragmentBox {
        mfhd: MovieFragmentHeaderBox::new(sequence_number),
        traf: traf_boxes,
    };

    // default-base-is-moof: data_offset measured from the moof start; the mdat
    // payload begins at moof_size + 8 (the mdat header).
    let moof_size = moof.serialized_len();
    let mut cursor = moof_size + 8;
    let mut mdat_data = Vec::new();
    for (i, ft) in tracks.iter().enumerate() {
        moof.traf[i].trun[0].data_offset = Some(cursor as i32);
        for s in ft.samples {
            mdat_data.extend_from_slice(&s.data);
            cursor += s.data.len();
        }
    }
    let mdat = MediaDataBox { data: mdat_data };

    let styp_len = styp.as_ref().map_or(0, |s| s.serialized_len());
    let total = styp_len + moof.serialized_len() + mdat.serialized_len();
    let mut out = vec![0u8; total];
    let mut c = 0usize;
    if let Some(styp) = &styp {
        c += styp.serialize_into(&mut out[c..])?;
    }
    c += moof.serialize_into(&mut out[c..])?;
    c += mdat.serialize_into(&mut out[c..])?;
    out.truncate(c);
    Ok(out)
}

// ===========================================================================
// LlDashPackager — low-latency DASH MPD
// ===========================================================================

/// Render a **low-latency** MPEG-DASH MPD (ISO/IEC 23009-1 + DASH-IF LL IOP).
///
/// Wraps the whole-segment [`DashPackager`] and post-processes its XML to add the
/// LL-DASH availability signalling: `availabilityTimeComplete="false"` +
/// `availabilityTimeOffset` on each `SegmentTemplate`, and a top-level
/// `<ServiceDescription>` with a `<Latency>` target (+ optional `<PlaybackRate>`).
/// Always emits `type="dynamic"` with an `availabilityStartTime`.
///
/// See the module docs for the exact spec clauses and why
/// `<ProducerReferenceTime>` is omitted.
#[derive(Debug, Clone)]
pub struct LlDashPackager {
    /// The underlying whole-segment packager (forced `dynamic` on package).
    pub base: DashPackager,
    /// Nominal segment duration in seconds (the `availabilityTimeOffset` base).
    pub segment_duration_secs: f64,
    /// Chunk duration in seconds. `availabilityTimeOffset = segment − chunk`.
    pub chunk_duration_secs: f64,
    /// Target end-to-end latency in **milliseconds** (`Latency@target`).
    pub latency_target_ms: u32,
    /// Optional catch-up playback rate bounds (`PlaybackRate@min`/`@max`).
    pub playback_rate: Option<(f64, f64)>,
}

impl LlDashPackager {
    /// Build an LL-DASH packager. `availability_start_time` is the wall-clock
    /// `MPD@availabilityStartTime` (ISO-8601 UTC). The `availabilityTimeOffset`
    /// is derived as `segment_duration_secs − chunk_duration_secs`.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if the durations are not positive/finite, or the
    /// chunk duration exceeds the segment duration (no LL benefit / negative ATO).
    pub fn new(
        segment_duration_secs: f64,
        chunk_duration_secs: f64,
        latency_target_ms: u32,
        availability_start_time: impl Into<String>,
    ) -> Result<Self> {
        if !(segment_duration_secs.is_finite() && segment_duration_secs > 0.0) {
            return Err(Error::InvalidInput(
                "segment_duration_secs must be positive and finite",
            ));
        }
        if !(chunk_duration_secs.is_finite() && chunk_duration_secs > 0.0) {
            return Err(Error::InvalidInput(
                "chunk_duration_secs must be positive and finite",
            ));
        }
        if chunk_duration_secs > segment_duration_secs {
            return Err(Error::InvalidInput(
                "chunk_duration_secs must not exceed segment_duration_secs",
            ));
        }
        let base = DashPackager {
            dynamic: true,
            availability_start_time: Some(availability_start_time.into()),
            ..DashPackager::default()
        };
        Ok(Self {
            base,
            segment_duration_secs,
            chunk_duration_secs,
            latency_target_ms,
            playback_rate: None,
        })
    }

    /// Set the optional catch-up `<PlaybackRate min max>` (DASH-IF LL IOP).
    pub fn with_playback_rate(mut self, min: f64, max: f64) -> Self {
        self.playback_rate = Some((min, max));
        self
    }

    /// The `availabilityTimeOffset` in seconds (`segment − chunk`, DASH-IF LL IOP
    /// §4.3). Always `>= 0` by construction.
    pub fn availability_time_offset(&self) -> f64 {
        (self.segment_duration_secs - self.chunk_duration_secs).max(0.0)
    }
}

impl Package for LlDashPackager {
    type Media = Media;
    type Output = String;
    type Error = Error;

    /// Render the LL-DASH MPD for `media`.
    ///
    /// # Errors
    /// Propagates [`DashPackager`] errors (e.g. empty track list).
    fn package(&mut self, media: &Media) -> Result<String> {
        let base_xml = self.base.package(media)?;
        Ok(self.inject_ll(&base_xml))
    }
}

impl LlDashPackager {
    /// Post-process the base MPD XML: add `<ServiceDescription>` as the first
    /// child of `<MPD>`, and the LL attributes onto every `<SegmentTemplate>`.
    fn inject_ll(&self, xml: &str) -> String {
        let ato = self.availability_time_offset();
        // Format ATO with up to three decimals (fixed math, no float-format
        // intrinsics beyond core's Display, which is available in `alloc`).
        let ato_str = format_secs(ato);

        // 1. Add availabilityTimeComplete + availabilityTimeOffset to every
        //    self-closing <SegmentTemplate .../>.
        let mut out = String::with_capacity(xml.len() + 256);
        for line in xml.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("<SegmentTemplate") && line.trim_end().ends_with("/>") {
                let end = line.trim_end();
                let head = &end[..end.len() - 2]; // strip "/>"
                out.push_str(head);
                out.push_str(" availabilityTimeOffset=\"");
                out.push_str(&ato_str);
                out.push_str("\" availabilityTimeComplete=\"false\"/>\n");
            } else {
                out.push_str(line);
                out.push('\n');
            }
        }

        // 2. Insert <ServiceDescription> immediately after the <MPD ...> open tag.
        let service = self.service_description();
        if let Some(pos) = find_mpd_open_end(&out) {
            let mut with_sd = String::with_capacity(out.len() + service.len());
            with_sd.push_str(&out[..pos]);
            with_sd.push_str(&service);
            with_sd.push_str(&out[pos..]);
            with_sd
        } else {
            out
        }
    }

    /// Render the `<ServiceDescription>` block (indented one level under `<MPD>`).
    fn service_description(&self) -> String {
        let mut s = String::new();
        s.push_str("  <ServiceDescription id=\"0\">\n");
        s.push_str("    <Latency target=\"");
        s.push_str(&self.latency_target_ms.to_string());
        s.push_str("\"/>\n");
        if let Some((min, max)) = self.playback_rate {
            s.push_str("    <PlaybackRate min=\"");
            s.push_str(&format_secs(min));
            s.push_str("\" max=\"");
            s.push_str(&format_secs(max));
            s.push_str("\"/>\n");
        }
        s.push_str("  </ServiceDescription>\n");
        s
    }
}

/// Byte offset just after the `>` that closes the `<MPD ...>` open tag.
fn find_mpd_open_end(xml: &str) -> Option<usize> {
    let start = xml.find("<MPD")?;
    let rel = xml[start..].find('>')?;
    Some(start + rel + 1 + 1) // +1 past '>', +1 past the trailing '\n'
}

/// Format a non-negative seconds value with up to three decimal places, trailing
/// zeros trimmed (e.g. `1.5`, `2`, `0.033`). Integer math only — no `std` float
/// formatting intrinsic beyond core `Display`.
fn format_secs(v: f64) -> String {
    // Round to milliseconds.
    let millis = (v * 1000.0 + 0.5) as u64;
    let whole = millis / 1000;
    let frac = millis % 1000;
    if frac == 0 {
        return whole.to_string();
    }
    // Trim trailing zeros from the 3-digit fraction.
    let mut f = format!("{frac:03}");
    while f.ends_with('0') {
        f.pop();
    }
    format!("{whole}.{f}")
}
