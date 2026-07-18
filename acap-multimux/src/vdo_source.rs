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

/// How many VDO buffers to read, at most, while searching for the codec's
/// full in-band parameter-set run (SPS/PPS for H.264; VPS/SPS/PPS for H.265)
/// needed to build the `TrackSpec`/`avcC`/`hvcC`. Axis encoders resend
/// parameter sets in-band with every IDR, so the params are expected within
/// the first handful of buffers; a bound avoids blocking forever on a
/// misconfigured stream.
const PARAM_SET_SCAN_LIMIT: usize = 150;

/// A VDO access unit read while scanning for parameter sets, held onto so it
/// can be delivered as a real sample instead of being silently dropped.
///
/// # Why buffer instead of discard
///
/// [`VdoSource::new`] must return with a complete [`TrackSpec`] already built
/// (before `multimux::pipeline::run_pipeline` calls `track_specs()`), so it
/// has to read ahead into the live buffer stream to find the parameter sets.
/// The buffer that *satisfies* [`convert::extract_param_sets`] is, in
/// practice, the first IDR access unit (Axis encoders emit SPS/PPS/VPS
/// in-band with the IDR slice in the same buffer) — i.e. exactly the sample
/// an LL-HLS segmenter needs as its first pushed sample (a sync sample).
/// Discarding it and pulling a fresh buffer for the first `next_samples()`
/// call would silently drop the IDR and could hand the segmenter a
/// non-sync first sample. Buffers read *before* the successful one are
/// genuinely discardable: without the parameter sets yet resolved, whatever
/// they contain isn't decodable relative to any init segment `VdoSource` will
/// ever publish for this stream start.
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

        let stream = StreamBuilder::new()
            .channel(channel)
            .format(format)
            .resolution(Resolution::Exact { width, height })
            .framerate(framerate)
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

/// Read buffers from `running` until `crate::convert::extract_param_sets`
/// finds a complete parameter-set run, returning the parameter sets plus the
/// successful buffer (preserved as a [`PendingAu`] — see its doc comment).
fn scan_for_param_sets(running: &RunningStream, codec: Codec) -> Result<(ParamSets, PendingAu)> {
    for i in 0..PARAM_SET_SCAN_LIMIT {
        let buf = running.next_buffer()?;
        let data = buf.data_copy()?;
        // Hardware-verify diagnostic (#669): log the framing of the first few
        // VDO buffers so we can confirm what `data_copy()` actually delivers on
        // this camera — Annex-B start codes (00 00 00 01) vs AVCC length-prefix,
        // and whether SPS/PPS are in-band. Only the leading buffers, to keep the
        // log readable on a live stream.
        if i < 8 {
            let head: Vec<String> = data.iter().take(24).map(|b| format!("{b:02x}")).collect();
            log::info!(
                "vdo scan buf[{i}]: frame_type={:?} len={} head=[{}]",
                buf.frame_type(),
                data.len(),
                head.join(" "),
            );
        }
        if let Some(params) = convert::extract_param_sets(codec, &data) {
            log::info!("vdo scan: found parameter sets at buf[{i}]");
            let pending = PendingAu {
                timestamp_us: buf.timestamp(),
                frame_type: buf.frame_type(),
                data,
            };
            return Ok((params, pending));
        }
    }
    Err(AcapError::Convert(format!(
        "no complete {codec:?} parameter-set run found in the first {PARAM_SET_SCAN_LIMIT} VDO buffers"
    )))
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
        let (data, timestamp_us, frame_type) = if let Some(pending) = self.pending_first.take() {
            (pending.data, pending.timestamp_us, pending.frame_type)
        } else {
            let buf = self.running.next_buffer().map_err(source_err)?;
            let data = buf.data_copy().map_err(source_err)?;
            (data, buf.timestamp(), buf.frame_type())
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
