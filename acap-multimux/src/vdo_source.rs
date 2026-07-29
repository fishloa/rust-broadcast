//! VDO capture source (`device`-gated): drives the acap-rs `vdo` crate
//! (Axis VDO — Video Capture API) to pull hardware-encoded H.264/H.265
//! access units off a camera channel, and exposes them as a
//! [`media_plane::ingress::IngestSession`] so this crate's own driving loop
//! (`src/bin/acap-multimux.rs`'s `run_vdo_capture`, over
//! [`multimux::supervise_driver`]/[`multimux::source::advance_route`]) can
//! segment them straight into LL-HLS. Conversion of an Annex B access unit
//! into a [`transmux::pipeline::Sample`]/[`transmux::pipeline::TrackSpec`] is
//! delegated entirely to the pure [`crate::convert`] module — this module's
//! only job is driving VDO and doing the timestamp/frame-type bookkeeping VDO
//! itself doesn't do for us.
//!
//! # Why a bare [`IngestSession`], not a [`media_plane::ingress::Dialer`]
//!
//! Every other multimux-ported input (RTSP/RTMP/SRT/TS-*/HLS-DASH-Smooth
//! pull) dials *out* over a network transport, so it fits
//! `Dialer::dial() -> Session` + the sans-IO handshake pump `media-plane`
//! documents (`dial()` performs no I/O; the handshake completes through the
//! ordinary `feed`/`poll_transmit` loop). VDO capture is not a dial-out at
//! all — it is a **local** hardware channel this process already owns, and
//! opening/starting it (`vdo::StreamBuilder::build`/`Stream::start`) plus the
//! in-band-parameter-set scan (see [`scan_for_param_sets`]) are one-shot local
//! setup work, not a multi-round-trip protocol exchange a `Dialer`/handshake
//! pump would buy anything by re-modelling. So `VdoIngestSession` is
//! constructed directly (`VdoIngestSession::new`, doing that local setup
//! synchronously) and driven directly by an
//! [`media_plane::ingress::IngestDriver`] the caller builds itself — no
//! `Dialer`, no [`media_plane::ingress::DialSupervisor`]. The caller's own
//! `attempt` closure (passed to [`multimux::supervise_driver`]) plays the
//! role a `Dialer`'s retry would have: if VDO setup or a later buffer read
//! fails, the closure returns and `supervise_driver` retries the whole
//! `VdoIngestSession::new` from scratch after backoff, exactly like a failed
//! dial would.
//!
//! # `Stage::In` is `()`: there is nothing to feed
//!
//! Every byte-stream `IngestSession` in `multimux` states `type In<'a> = &'a
//! [u8]` because its driving loop reads bytes off a socket and hands them to
//! `feed`. VDO has no bytes for a caller to read and hand in this shape —
//! the *session itself* performs the (blocking) hardware read inside `feed`.
//! So `VdoIngestSession::In<'a> = ()`: the driving loop's contract becomes
//! "call `feed(())` again to advance", and `feed` is where the blocking VDO
//! read (or, on the very first call, replaying the already-resolved
//! parameter-set scan's pending access unit) actually happens. This is
//! exactly the relaxation `media_plane::ingress`'s round-3 docs describe for
//! non-byte-stream sources (a pull source states its own request/response
//! shape; a pure local-capture source states the simplest honest shape it
//! has, which for VDO is nothing at all).
//!
//! # Blocking I/O — run on a dedicated thread or task
//!
//! [`vdo::RunningStream::next_buffer`] **blocks the calling thread** until the
//! camera produces the next frame (it is a synchronous FFI call into
//! `libvdo.so`, not a `poll`-based async I/O source). `VdoIngestSession::feed`
//! calls it directly and therefore blocks too. **Whoever drives the VDO
//! capture loop (`src/bin/acap-multimux.rs`'s `run_vdo_capture`, spawned by
//! `spawn_capture_pipeline`) must ensure this blocking read does not stall
//! other work on the same thread**: that function runs the whole
//! capture/segment/store pipeline on its own `std::thread::spawn`'d OS thread
//! with a dedicated `current_thread` tokio runtime, so the blocking call only
//! ever stalls that one dedicated thread, never axum's worker threads. Note
//! that `RunningStream`/`Stream` are `unsafe impl Send` in the `vdo` crate
//! (verified against acap-rs rev `8e58acb8f0617253ad21fb71ac319fea19454a38`),
//! so `VdoIngestSession` itself is `Send` (required by
//! [`IngestSession: Send`](media_plane::ingress::IngestSession)) — the risk is
//! purely the blocking call starving a single-threaded executor, not a `Send`
//! bound failure.
//!
//! # Spec / API grounding
//!
//! Against acap-rs rev `8e58acb8f0617253ad21fb71ac319fea19454a38`:
//! - `vdo::StreamBuilder::{channel, format, resolution, framerate}` + `.build()`
//!   (`crates/vdo/src/lib.rs`).
//! - `vdo::Stream::start() -> RunningStream` (consumes `self`).
//! - `vdo::RunningStream::next_buffer(&self) -> Result<StreamBuffer<'_>, vdo::Error>`
//!   (blocking, see above).
//! - `vdo::StreamBuffer::{data_copy, as_slice, header_size, frame_type,
//!   timestamp}` — `data_copy()` returns the coded slice with the buffer's
//!   header (`header_size` bytes) stripped, which is exactly the CMAF sample
//!   `crate::convert` expects. On a *key* frame that header is where VDO
//!   carries the SPS/PPS parameter sets, so [`scan_for_param_sets`] reads the
//!   full frame via `as_slice()` (**not** `data_copy()`) to recover them — see
//!   its doc. **This distinction is load-bearing**: reading `data_copy()`
//!   here would silently drop the parameter sets and produce a stream that
//!   looks structurally fine (init segment builds, segments serve) but that
//!   no real decoder can actually decode.
//! - `vdo::VdoFrameType::{VDO_FRAME_TYPE_H264_IDR, VDO_FRAME_TYPE_H265_IDR}` are
//!   the sync-sample frame types for H.264/H.265 respectively (confirmed
//!   against `vdo`'s own `capture_h264_frames` hardware test, which matches on
//!   `VDO_FRAME_TYPE_H264_IDR | VDO_FRAME_TYPE_H264_I` for "got a key frame");
//!   `VdoIngestSession` treats only the IDR variant as a sync sample
//!   (`is_sync`) — the non-IDR `_I` type is an intra frame that need not reset
//!   the decoder's reference-picture state, so it is not a safe CMAF
//!   random-access point.

use std::collections::VecDeque;
use std::convert::Infallible;

use broadcast_common::{Demand, Stage, Timestamp};
use media_plane::ingress::{IngestSession, ProgramId, SessionEvent};
use media_plane::trunk::RetentionClass;
use transmux::pipeline::{Sample, TrackSpec};
use vdo::{Resolution, RunningStream, StreamBuilder, VdoFormat, VdoFrameType};

use crate::Result;
use crate::convert::{self, Codec, ParamSets};
use crate::error::AcapError;

/// Fixed single (video) track id — `VdoIngestSession` carries exactly one
/// video track per stream.
const TRACK_ID: u32 = 1;

/// This app's single program id — VDO captures exactly one camera channel
/// per session, so there is exactly one program, always `ProgramId(0)`.
const PROGRAM: ProgramId = ProgramId(0);

/// Media/track timescale for both H.264 and H.265 (90 kHz video clock).
const CLOCK_RATE: u32 = 90_000;

/// How many VDO buffers to read, at most, while resolving the codec's full
/// parameter-set run (SPS/PPS for H.264; VPS/SPS/PPS for H.265) needed to
/// build the `TrackSpec`/`avcC`/`hvcC`. The parameter sets ride in the
/// key-frame buffer's header (see [`scan_for_param_sets`]), so the very first
/// key frame normally resolves them; the bound spans a generous multi-GOP
/// window to tolerate a mid-GOP start (and the separate-parameter-set-buffer
/// fallback) while avoiding blocking forever on a stream that never keys.
const PARAM_SET_SCAN_LIMIT: usize = 150;

/// The first IDR access unit found while collecting parameter sets, held onto
/// so it can be delivered as the first real sample instead of being dropped.
///
/// # Why buffer instead of discard
///
/// [`VdoIngestSession::new`] must return with a complete [`TrackSpec`] already
/// built (before the caller's `IngestDriver` ever calls `feed`), so it reads
/// ahead into the live buffer stream until it can resolve the parameter sets
/// — from the key frame's own header (the primary path) or, as a fallback,
/// from separately-delivered parameter-set buffers (see
/// [`scan_for_param_sets`]). That first resolving key frame is the first
/// decodable sync sample for this stream start — exactly the sample an LL-HLS
/// segmenter needs as its first pushed sample. Discarding it and pulling a
/// fresh buffer for the first `feed()` call would drop that IDR and could
/// hand the segmenter a non-sync first sample. It is delivered as its
/// header-stripped `data_copy()` (SPS/PPS/VPS are carried in the `avcC`/`hvcC`
/// init segment, not the coded samples); any parameter-set/SEI buffer and any
/// picture read before the parameter sets resolve are not decodable relative
/// to the init this source will publish, and are dropped.
struct PendingAu {
    data: Vec<u8>,
    timestamp_us: u64,
    frame_type: VdoFrameType,
}

/// A live VDO stream adapted into a [`media_plane::ingress::IngestSession`].
///
/// Built by [`VdoIngestSession::new`], which starts the VDO stream, scans
/// forward for in-band parameter sets, and pre-builds the single-track
/// [`TrackSpec`] — all before the first [`Stage::feed`] call, so
/// [`SessionEvent::Established`] and [`SessionEvent::NewProgram`] are already
/// queued and ready the moment the caller starts driving. Every subsequent
/// [`feed`](Stage::feed) call blocks (see the module doc) on
/// [`RunningStream::next_buffer`].
pub struct VdoIngestSession {
    running: RunningStream,
    track_id: u32,
    codec: Codec,
    clock_rate: u32,
    specs: Vec<TrackSpec>,
    prev_ts_us: Option<u64>,
    /// The parameter-set-bearing access unit found by `new()`, replayed as
    /// the very first sample (see [`PendingAu`]).
    pending_first: Option<PendingAu>,
    /// `true` once the initial `Established`+`NewProgram`(+first sample)
    /// batch has been queued — every `feed()` call after that instead
    /// performs one blocking VDO buffer read (see [`Self::feed`]).
    initial_batch_sent: bool,
    /// Events ready for [`Stage::poll`] to hand back, in order.
    pending: VecDeque<SessionEvent>,
}

impl VdoIngestSession {
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
            initial_batch_sent: false,
            pending: VecDeque::new(),
        })
    }

    /// Read exactly one more coded-picture buffer from VDO (blocking; skips
    /// any interleaved parameter-set/SEI buffers exactly like
    /// [`scan_for_param_sets`]'s own skip loop), and turn it into a
    /// [`Sample`].
    fn read_next_sample(&mut self) -> Result<Sample> {
        loop {
            let buf = self.running.next_buffer()?;
            let ft = buf.frame_type();
            if !is_picture(self.codec, ft) {
                // non-key pictures / SEI while live: dropped, exactly as the
                // scan loop drops them.
                continue;
            }
            let data = buf.data_copy()?;
            let timestamp_us = buf.timestamp();
            let is_sync = is_idr(self.codec, ft);
            let duration = convert::duration_ticks(
                self.prev_ts_us.unwrap_or(timestamp_us),
                timestamp_us,
                self.clock_rate,
            );
            self.prev_ts_us = Some(timestamp_us);
            let pts_dts = convert::absolute_ticks(timestamp_us, self.clock_rate);
            return Ok(convert::au_to_sample(
                self.codec, &data, pts_dts, duration, is_sync,
            ));
        }
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
/// a standalone SEI buffer is not a coded picture. [`VdoIngestSession::read_next_sample`]
/// skips everything that is not a picture.
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

impl Stage for VdoIngestSession {
    /// Nothing to feed — VDO capture has no bytes for a caller to read and
    /// hand in; `feed` itself performs the (blocking) hardware read. See the
    /// module doc's "`Stage::In` is `()`" section.
    type In<'a> = ();
    type Out = SessionEvent;
    type Error = AcapError;

    /// Always "one more" — VDO capture has no meaningful backlog signal to
    /// report; the caller drives this session in a plain loop regardless.
    fn demand(&self) -> Demand {
        Demand::new(1)
    }

    /// Advance the session by exactly one step.
    ///
    /// The **first** call queues [`SessionEvent::Established`],
    /// [`SessionEvent::NewProgram`] (this session's single video track), and
    /// — if [`VdoIngestSession::new`]'s parameter-set scan captured one — the
    /// buffered key-frame [`SessionEvent::Sample`], all without touching VDO
    /// again (everything needed was already resolved synchronously in
    /// `new()`). **Every call after that** blocks on
    /// [`RunningStream::next_buffer`] (via [`Self::read_next_sample`]) and
    /// queues exactly one more [`SessionEvent::Sample`]. A live camera
    /// channel has no natural end-of-stream, so a VDO read/convert failure
    /// here is reported as `Err` (driving [`media_plane::ingress::HealthState::Failed`])
    /// rather than a clean [`Stage::finish`].
    fn feed(&mut self, _input: (), _now: Timestamp) -> Result<()> {
        if !self.initial_batch_sent {
            self.initial_batch_sent = true;
            self.pending.push_back(SessionEvent::Established);
            self.pending.push_back(SessionEvent::NewProgram {
                program: PROGRAM,
                tracks: self.specs.clone(),
            });
            // The pending IDR from `new()`'s scan is always a coded picture
            // (parameter-set/SEI buffers read while scanning were already
            // dropped there) — deliver it as the first sample rather than
            // discarding it (see `PendingAu`'s doc).
            if let Some(pending) = self.pending_first.take() {
                let is_sync = is_idr(self.codec, pending.frame_type);
                // First sample: no previous timestamp to diff against, so
                // duration is 0 — matches every other ported source's first
                // sample (no extra frame of latency; VDO delivers one whole
                // access unit per buffer with its own timestamp).
                let duration = convert::duration_ticks(
                    pending.timestamp_us,
                    pending.timestamp_us,
                    self.clock_rate,
                );
                self.prev_ts_us = Some(pending.timestamp_us);
                let pts_dts = convert::absolute_ticks(pending.timestamp_us, self.clock_rate);
                let sample =
                    convert::au_to_sample(self.codec, &pending.data, pts_dts, duration, is_sync);
                self.pending.push_back(SessionEvent::Sample {
                    program: PROGRAM,
                    track_id: self.track_id,
                    retention: RetentionClass::Timed,
                    sample,
                });
            }
            return Ok(());
        }

        let sample = self.read_next_sample()?;
        self.pending.push_back(SessionEvent::Sample {
            program: PROGRAM,
            track_id: self.track_id,
            retention: RetentionClass::Timed,
            sample,
        });
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending.pop_front()
    }

    /// No time-driven work of its own — every event is produced by a `feed`
    /// call (see [`Self::feed`]).
    fn next_deadline(&self) -> Option<Timestamp> {
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    /// A live camera channel is never told to stop by VDO itself; this only
    /// runs if the caller decides to stop driving (e.g. process shutdown),
    /// and there is nothing buffered that needs flushing beyond what
    /// [`Stage::poll`] has not yet drained.
    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}

impl IngestSession for VdoIngestSession {
    /// Uninhabited: VDO capture never has anything of its own to send back —
    /// there is no connection to write handshake/keepalive requests onto (see
    /// the module doc's "why a bare `IngestSession`" section). A byte-stream
    /// scheme would set this to `bytes::Bytes` instead (see
    /// `multimux::source::rtsp::RtspIngestSession` for that shape).
    type Request = Infallible;
}
