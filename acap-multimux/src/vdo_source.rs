//! VDO capture source (`device`-gated): drives the acap-rs `vdo` crate
//! (Axis VDO — Video Capture API) to pull hardware-encoded H.264/H.265
//! access units off a camera channel, and exposes them as a
//! [`multimux::pipeline::SampleSource`] so [`multimux::pipeline::run_pipeline`]
//! can segment them straight into LL-HLS. Conversion of an Annex B access
//! unit into a [`transmux::pipeline::Sample`]/[`transmux::pipeline::TrackSpec`]
//! is delegated entirely to the pure [`crate::convert`] module — this module's
//! only job is driving VDO and doing the timestamp/frame-type bookkeeping VDO
//! itself doesn't do for us.
//!
//! # Blocking I/O — run on a dedicated thread or task
//!
//! [`vdo::RunningStream::next_buffer`] **blocks the calling thread** until the
//! camera produces the next frame (it is a synchronous FFI call into
//! `libvdo.so`, not a `poll`-based async I/O source). `VdoSource` does not
//! wrap it in `spawn_blocking` itself, since [`multimux::pipeline::SampleSource`]
//! is a plain (non-blocking-aware) trait and doing so here would require a
//! runtime handle this module has no business owning. **Whoever drives
//! `run_pipeline(VdoSource)` (Task 4's ACAP `main`) must ensure this blocking
//! read does not stall other work on the same thread**: either give the VDO
//! pipeline task its own OS thread (e.g. `tokio::task::spawn_blocking` bridging
//! into an async channel, or a plain `std::thread::spawn` + a `tokio::sync::mpsc`
//! feed) or run it on a `rt-multi-thread` runtime where the axum origin has
//! worker threads free while this task's thread blocks in VDO. Note that
//! `RunningStream`/`Stream` are `unsafe impl Send` in the `vdo` crate (verified
//! against acap-rs rev `8e58acb8f0617253ad21fb71ac319fea19454a38`), so
//! `VdoSource` itself is `Send` and *can* be `tokio::spawn`'d — the risk is
//! purely the blocking call starving a single-threaded (`current_thread`)
//! executor, not a `Send` bound failure.
//!
//! # Spec / API grounding
//!
//! Against acap-rs rev `8e58acb8f0617253ad21fb71ac319fea19454a38`:
//! - `vdo::StreamBuilder::{channel, format, resolution, framerate}` + `.build()`
//!   (`crates/vdo/src/lib.rs`).
//! - `vdo::Stream::start() -> RunningStream` (consumes `self`).
//! - `vdo::RunningStream::next_buffer(&self) -> Result<StreamBuffer<'_>, vdo::Error>`
//!   (blocking, see above).
//! - `vdo::StreamBuffer::{data_copy, frame_type, timestamp}` — `data_copy()`
//!   returns the coded access unit with VDO's own header stripped (not part of
//!   the Annex B bytes), which is exactly the access unit `crate::convert`
//!   expects.
//! - `vdo::VdoFrameType::{VDO_FRAME_TYPE_H264_IDR, VDO_FRAME_TYPE_H265_IDR}` are
//!   the sync-sample frame types for H.264/H.265 respectively (confirmed
//!   against `vdo`'s own `capture_h264_frames` hardware test, which matches on
//!   `VDO_FRAME_TYPE_H264_IDR | VDO_FRAME_TYPE_H264_I` for "got a key frame");
//!   `VdoSource` treats only the IDR variant as a sync sample (`is_sync`) —
//!   the non-IDR `_I` type is an intra frame that need not reset the decoder's
//!   reference-picture state, so it is not a safe CMAF random-access point.

use transmux::pipeline::{Sample, TrackSpec};
use vdo::{Resolution, RunningStream, StreamBuilder, VdoFormat, VdoFrameType};

use crate::Result;
use crate::convert::{self, Codec, ParamSets};
use crate::error::AcapError;

/// Fixed single (video) track id — `VdoSource` carries exactly one video
/// track per stream.
const TRACK_ID: u32 = 1;

/// Media/track timescale for both H.264 and H.265 (90 kHz video clock).
const CLOCK_RATE: u32 = 90_000;

/// How many VDO buffers to read, at most, while collecting the codec's full
/// parameter-set run (SPS/PPS for H.264; VPS/SPS/PPS for H.265) needed to
/// build the `TrackSpec`/`avcC`/`hvcC`. Axis VDO delivers parameter sets as
/// their own dedicated buffers (frame types `VDO_FRAME_TYPE_H264_SPS`/`_PPS`,
/// `_H265_VPS`/`_SPS`/`_PPS`) — **not** in-band with the IDR slice — and
/// resends them ahead of each IDR, so the full set arrives within one GOP.
/// Starting capture mid-GOP means the first IDR seen can precede the next
/// parameter-set run; the bound spans a generous multi-GOP window so that
/// case still resolves, while avoiding blocking forever on a stream that
/// never emits parameter sets.
const PARAM_SET_SCAN_LIMIT: usize = 150;

/// The first IDR access unit found while collecting parameter sets, held onto
/// so it can be delivered as the first real sample instead of being dropped.
///
/// # Why buffer instead of discard
///
/// [`VdoSource::new`] must return with a complete [`TrackSpec`] already built
/// (before `multimux::pipeline::run_pipeline` calls `track_specs()`), so it
/// reads ahead into the live buffer stream, gathering the parameter-set
/// buffers (SPS/PPS/VPS, each its own VDO buffer) until it has the full set.
/// The IDR buffer that immediately follows a complete parameter-set run is the
/// first decodable sync sample for this stream start — exactly the sample an
/// LL-HLS segmenter needs as its first pushed sample. Discarding it and
/// pulling a fresh buffer for the first `next_samples()` call would drop that
/// IDR and could hand the segmenter a non-sync first sample. The parameter-set
/// and SEI buffers consumed while scanning are *not* samples (SPS/PPS/VPS are
/// carried in the `avcC`/`hvcC` init segment, not the coded samples), and any
/// picture buffer read before the parameter sets resolve isn't decodable
/// relative to any init segment this source will publish — both are dropped.
struct PendingAu {
    data: Vec<u8>,
    timestamp_us: u64,
    frame_type: VdoFrameType,
}

/// A live VDO stream adapted into a [`multimux::pipeline::SampleSource`].
///
/// Built by [`VdoSource::new`], which starts the VDO stream, scans forward
/// for in-band parameter sets, and pre-builds the single-track [`TrackSpec`].
/// Every subsequent [`next_samples`](multimux::pipeline::SampleSource::next_samples)
/// call blocks (see the module doc) on [`RunningStream::next_buffer`].
pub struct VdoSource {
    running: RunningStream,
    track_id: u32,
    codec: Codec,
    clock_rate: u32,
    specs: Vec<TrackSpec>,
    prev_ts_us: Option<u64>,
    /// The parameter-set-bearing access unit found by `new()`, replayed as
    /// the very first `next_samples()` batch (see [`PendingAu`]).
    pending_first: Option<PendingAu>,
}

impl VdoSource {
    /// Open VDO `channel` at `width`x`height`/`framerate`, encoding `codec`,
    /// start it, and scan forward for the in-band parameter sets needed to
    /// build the track's `avcC`/`hvcC`.
    ///
    /// # Errors
    /// Returns [`AcapError::Vdo`] if the stream can't be built/started or a
    /// buffer read fails while scanning for parameter sets, and
    /// [`AcapError::Convert`] if no complete parameter-set run turns up
    /// within [`PARAM_SET_SCAN_LIMIT`] buffers, or the parameter sets found
    /// don't decode into a valid `TrackSpec` (propagated from
    /// [`convert::track_spec`]).
    pub fn new(
        codec: Codec,
        channel: u32,
        width: u32,
        height: u32,
        framerate: u32,
    ) -> Result<Self> {
        let format = match codec {
            Codec::H264 => VdoFormat::VDO_FORMAT_H264,
            Codec::H265 => VdoFormat::VDO_FORMAT_H265,
        };

        // Force a ~1-second GOP so key frames — and the SPS/PPS/VPS parameter
        // sets VDO emits ahead of each one, as their own buffers — recur
        // predictably. Without this, a camera in dynamic-GOP / Zipstream mode
        // can go many seconds between key frames, so `scan_for_param_sets`
        // finds no parameter-set run within its bounded window (observed on
        // ARTPEC-6 / firmware 11, #669). Falls back to 30 if the caller left
        // framerate at 0 (camera default) rather than forcing a key frame every
        // frame.
        let gop_length = if framerate > 0 { framerate } else { 30 };

        let stream = StreamBuilder::new()
            .channel(channel)
            .format(format)
            .resolution(Resolution::Exact { width, height })
            .framerate(framerate)
            .gop_length(gop_length)
            .build()?;

        let running = stream.start()?;

        let (params, pending_first) = scan_for_param_sets(&running, codec)?;
        let spec = convert::track_spec(codec, &params, TRACK_ID, CLOCK_RATE)?;

        Ok(Self {
            running,
            track_id: TRACK_ID,
            codec,
            clock_rate: CLOCK_RATE,
            specs: vec![spec],
            prev_ts_us: None,
            pending_first: Some(pending_first),
        })
    }
}

/// Read buffers from `running` until the parameter sets (SPS/PPS for H.264;
/// VPS/SPS/PPS for H.265) can be resolved, returning them plus the key-frame
/// access unit as a [`PendingAu`].
///
/// Where VDO puts the parameter sets (verified on hardware, ARTPEC-6/H.264,
/// #669): they are carried in the **key-frame buffer's header** — the bytes
/// `data_copy()` strips off the front (`header_size`) to leave just the coded
/// slice. So the full key-frame buffer is `[SPS][PPS][…][IDR slice]` in Annex
/// B, and `extract_param_sets` finds the run in `as_slice()` even though
/// `data_copy()` (the sample bytes) does not contain it. As a fallback for
/// cameras/configs that instead deliver each parameter set as its own buffer
/// (frame types `VDO_FRAME_TYPE_H264_SPS`/`_PPS`, …), those are also collected
/// and tried. The sample handed on for the key frame is the header-stripped
/// `data_copy()` (parameter sets ride in the `avcC`/`hvcC` init, not samples).
fn scan_for_param_sets(running: &RunningStream, codec: Codec) -> Result<(ParamSets, PendingAu)> {
    // Fallback path: latest Annex B bytes for each separately-delivered
    // parameter-set NAL, kept individually so a resent run replaces it.
    let mut vps: Option<Vec<u8>> = None; // H.265 only
    let mut sps: Option<Vec<u8>> = None;
    let mut pps: Option<Vec<u8>> = None;

    for i in 0..PARAM_SET_SCAN_LIMIT {
        let buf = running.next_buffer()?;
        let ft = buf.frame_type();
        let data = buf.data_copy()?;
        // Hardware-verify diagnostic (#669): frame type, header size (where the
        // key-frame parameter sets live), and the leading bytes of the full
        // frame — for the first few buffers only, to keep a live log short.
        if i < 12 {
            let full = buf.as_slice().ok();
            let head: Vec<String> = full
                .map(|s| &s[..buf.size().min(s.len())])
                .unwrap_or(data.as_slice())
                .iter()
                .take(16)
                .map(|b| format!("{b:02x}"))
                .collect();
            log::info!(
                "vdo scan buf[{i}]: frame_type={ft:?} size={} header_size={:?} full_head=[{}]",
                buf.size(),
                buf.header_size(),
                head.join(" "),
            );
        }
        if let Some(kind) = param_set_kind(codec, ft) {
            // Separately-delivered parameter-set buffer (fallback path).
            match kind {
                ParamSetKind::Vps => vps = Some(data),
                ParamSetKind::Sps => sps = Some(data),
                ParamSetKind::Pps => pps = Some(data),
            }
            continue;
        }
        if is_idr(codec, ft) {
            // Primary path: the key frame's own buffer carries the parameter
            // sets in the header that `data_copy()` strips — parse the *full*
            // frame (`as_slice()` up to `size()`).
            let full = buf.as_slice()?;
            let full_au = &full[..buf.size().min(full.len())];
            if let Some(params) = convert::extract_param_sets(codec, full_au) {
                log::info!("vdo scan: parameter sets from key-frame header at buf[{i}]");
                return Ok((
                    params,
                    PendingAu {
                        timestamp_us: buf.timestamp(),
                        frame_type: ft,
                        data,
                    },
                ));
            }
            // Fallback path: pair the separately-collected parameter sets with
            // this key frame. Their concatenation is valid Annex B (each is a
            // whole Annex-B NAL buffer).
            let mut blob = Vec::new();
            if let Some(v) = &vps {
                blob.extend_from_slice(v);
            }
            if let (Some(s), Some(p)) = (&sps, &pps) {
                blob.extend_from_slice(s);
                blob.extend_from_slice(p);
            }
            if let Some(params) = convert::extract_param_sets(codec, &blob) {
                log::info!("vdo scan: parameter sets from separate buffers, IDR at buf[{i}]");
                return Ok((
                    params,
                    PendingAu {
                        timestamp_us: buf.timestamp(),
                        frame_type: ft,
                        data,
                    },
                ));
            }
            // Key frame before parameter sets resolve (mid-GOP start): drop and
            // keep scanning.
        }
        // non-key pictures / SEI while scanning: dropped
    }
    Err(AcapError::Convert(format!(
        "no complete {codec:?} parameter-set run found in the first {PARAM_SET_SCAN_LIMIT} VDO buffers"
    )))
}

/// Which parameter-set NAL a VDO parameter-set frame type carries.
enum ParamSetKind {
    /// H.265 video parameter set (no H.264 equivalent).
    Vps,
    /// Sequence parameter set.
    Sps,
    /// Picture parameter set.
    Pps,
}

/// Classify a VDO `frame_type` as a parameter-set buffer, if it is one. VDO
/// delivers SPS/PPS (and, for H.265, VPS) as dedicated buffers rather than
/// in-band with the coded picture (see [`scan_for_param_sets`]).
fn param_set_kind(codec: Codec, ft: VdoFrameType) -> Option<ParamSetKind> {
    // VDO frame-type values are associated consts, not enum variants usable in
    // patterns, so classify with `==` comparisons (as `is_idr` does).
    match codec {
        Codec::H264 => {
            if ft == VdoFrameType::VDO_FRAME_TYPE_H264_SPS {
                Some(ParamSetKind::Sps)
            } else if ft == VdoFrameType::VDO_FRAME_TYPE_H264_PPS {
                Some(ParamSetKind::Pps)
            } else {
                None
            }
        }
        Codec::H265 => {
            if ft == VdoFrameType::VDO_FRAME_TYPE_H265_VPS {
                Some(ParamSetKind::Vps)
            } else if ft == VdoFrameType::VDO_FRAME_TYPE_H265_SPS {
                Some(ParamSetKind::Sps)
            } else if ft == VdoFrameType::VDO_FRAME_TYPE_H265_PPS {
                Some(ParamSetKind::Pps)
            } else {
                None
            }
        }
    }
}

/// Whether `frame_type` is a coded-picture buffer (IDR/I/P/B) — the buffers
/// that become CMAF samples. Parameter-set (SPS/PPS/VPS) and SEI buffers are
/// **not** samples: parameter sets live in the `avcC`/`hvcC` init segment, and
/// a standalone SEI buffer is not a coded picture. `next_samples` skips
/// everything that is not a picture.
fn is_picture(codec: Codec, ft: VdoFrameType) -> bool {
    // `==` chains rather than `matches!` — VDO frame types are associated
    // consts, not pattern-usable enum variants.
    match codec {
        Codec::H264 => {
            ft == VdoFrameType::VDO_FRAME_TYPE_H264_IDR
                || ft == VdoFrameType::VDO_FRAME_TYPE_H264_I
                || ft == VdoFrameType::VDO_FRAME_TYPE_H264_P
                || ft == VdoFrameType::VDO_FRAME_TYPE_H264_B
        }
        Codec::H265 => {
            ft == VdoFrameType::VDO_FRAME_TYPE_H265_IDR
                || ft == VdoFrameType::VDO_FRAME_TYPE_H265_I
                || ft == VdoFrameType::VDO_FRAME_TYPE_H265_P
                || ft == VdoFrameType::VDO_FRAME_TYPE_H265_B
        }
    }
}

/// Whether `frame_type` is the codec's IDR (instantaneous decoder refresh)
/// frame type — the only VDO frame type this module treats as a CMAF sync
/// sample (see the module doc for why the non-IDR `_I` type is excluded).
fn is_idr(codec: Codec, frame_type: VdoFrameType) -> bool {
    match codec {
        Codec::H264 => frame_type == VdoFrameType::VDO_FRAME_TYPE_H264_IDR,
        Codec::H265 => frame_type == VdoFrameType::VDO_FRAME_TYPE_H265_IDR,
    }
}

/// Map a `vdo::Error` into a `multimux::MultimuxError::Source` — multimux is
/// an external crate and can't itself define `From<vdo::Error>`.
fn source_err(err: vdo::Error) -> multimux::MultimuxError {
    multimux::MultimuxError::Source(format!("vdo: {err}"))
}

impl multimux::pipeline::SampleSource for VdoSource {
    fn track_specs(&self) -> Vec<TrackSpec> {
        self.specs.clone()
    }

    /// Pull the next access unit and convert it to a [`Sample`].
    ///
    /// The very first call replays the buffer [`VdoSource::new`] already
    /// consumed while scanning for parameter sets (see [`PendingAu`]);
    /// every call after that blocks on [`RunningStream::next_buffer`] (see
    /// the module doc for the blocking-thread implication). A live camera
    /// channel has no natural end-of-stream, so a VDO read/convert failure
    /// is reported as `Err` rather than `Ok(None)`.
    ///
    /// Duration is the delta between this buffer's VDO timestamp (µs since
    /// boot) and the previous one's, converted to the track's 90 kHz
    /// timescale via [`convert::duration_ticks`] — VDO delivers one complete
    /// access unit per buffer with its own timestamp, so (unlike an RTP
    /// depacketizer, which only learns a sample's duration from the *next*
    /// packet's timestamp) there is no extra frame of latency here: the
    /// first sample's duration is `0` (no previous timestamp to diff
    /// against), and every sample after that carries the correct duration
    /// for the frame just read.
    async fn next_samples(&mut self) -> multimux::Result<Option<Vec<(u32, Sample)>>> {
        // The pending IDR from `new()` is always a coded picture; live reads
        // skip the parameter-set (SPS/PPS/VPS) and SEI buffers VDO interleaves
        // ahead of each IDR — those are carried in the init segment or are not
        // coded pictures, so they must not be emitted as samples.
        let (data, timestamp_us, frame_type) = if let Some(pending) = self.pending_first.take() {
            (pending.data, pending.timestamp_us, pending.frame_type)
        } else {
            loop {
                let buf = self.running.next_buffer().map_err(source_err)?;
                let ft = buf.frame_type();
                if is_picture(self.codec, ft) {
                    let data = buf.data_copy().map_err(source_err)?;
                    break (data, buf.timestamp(), ft);
                }
            }
        };

        let is_sync = is_idr(self.codec, frame_type);
        let duration = convert::duration_ticks(
            self.prev_ts_us.unwrap_or(timestamp_us),
            timestamp_us,
            self.clock_rate,
        );
        self.prev_ts_us = Some(timestamp_us);

        let sample = convert::au_to_sample(self.codec, &data, duration, is_sync);
        Ok(Some(vec![(self.track_id, sample)]))
    }
}
