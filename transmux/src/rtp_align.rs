//! Places every RTP track's sample times on one common timeline.
//!
//! Each track's RTP timestamps start at an unrelated random base (RFC 3550 5.1), so the raw
//! sample times of two tracks cannot be compared. The common timeline is the video track's own
//! (its sample times pass through unchanged); every other track gets a per-track tick offset
//! added to its pts and dts.
//!
//! When the PLAY response carries an `RTP-Info` `rtptime` for every track (RFC 2326 12.33: each is
//! the RTP timestamp of the same start instant) that mapping is the offset source
//! ([`AvSync::RtpInfo`]) and Sender Reports no longer move it: a relay can derive its Sender
//! Reports from packet arrival, which for reordered video differs from presentation time, and an
//! `RTP-Info` is the server's statement about the streams' own timestamps. Without it the rules
//! below apply.
//!
//! Before Sender Reports for the tracks are known the offset lines the track's first sample up
//! with the video track's first sample ([`AvSync::Provisional`]). Once the depacketiser holds a
//! Sender Report (RFC 3550 6.4.1, NTP time to RTP time) for both tracks the offset comes from
//! `RtpStreamDepacketiser::sync_start_decode_times`, which owns the NTP arithmetic
//! ([`AvSync::SenderReport`]).
//!
//! If the Sender Reports' NTP clocks disagree by more than an hour the whole set is refused
//! (`RtpStreamDepacketiser::try_sync_start_decode_times` checks every anchored track together),
//! the tracks stay [`AvSync::Provisional`] for as long as the clocks keep disagreeing, and the
//! refusal is visible through [`TrackAligner::skew_rejections`] and
//! [`TrackAligner::sender_reports_refused`]. An `RTP-Info` mapping, when present, is not affected.
//!
//! Switching to the new offset never moves a track's times backwards: a forward step is applied at
//! once, a backward step is slewed by at most half a sample duration per sample. A step larger
//! than [`SR_SWITCH_TOLERANCE_US`] is an honest timeline change and is reported as one
//! [`AlignOut::Discontinuity`].

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use broadcast_common::timescale::{Rounding, Timescale, rescale_i64, to_micros};

use crate::TrackId;
use crate::pipeline::Sample;
use crate::rtp_stream::RtpStreamDepacketiser;

/// How the tracks of an RTSP session are placed on one common timeline.
///
/// Each RTSP track's RTP timestamps start at an unrelated random base (RFC 3550 5.1), so the
/// aligner maps every track onto the video track's timeline (video sample times are unchanged).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvSync {
    /// No RTCP Sender Report mapping is available for every track yet: each non-video track is
    /// placed so its first sample lines up with the video track's first sample, which is accurate
    /// only to the arrival skew between those two samples.
    Provisional,
    /// Every track is placed from the NTP to RTP mapping of its RTCP Sender Reports (RFC 3550
    /// 6.4.1), so the inter-track offset is the senders' own.
    SenderReport,
    /// Every track is placed from the `RTP-Info` header of the PLAY response (RFC 2326 12.33):
    /// each stream's `rtptime` is the RTP timestamp of the same start instant, so the inter-track
    /// offset is the server's own and no Sender Report can move it.
    RtpInfo,
}

impl AvSync {
    /// Stable label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Provisional => "provisional",
            Self::SenderReport => "sender-report",
            Self::RtpInfo => "rtp-info",
        }
    }
}
broadcast_common::impl_spec_display!(AvSync);

/// Largest offset change applied without a [`AlignOut::Discontinuity`]. The provisional offset is
/// wrong by the arrival skew between the first audio and first video sample, which on a healthy
/// stream is a few packet times; 100 ms is about five AAC frames at 44.1 kHz, inside the range where
/// audio/video offset is still read as in sync, so a correction up to that size is absorbed
/// quietly and anything larger is reported to the host.
pub const SR_SWITCH_TOLERANCE_US: i128 = 100_000;
const MAX_HELD_SAMPLES: usize = 256;
const SLEW_DIVISOR: i64 = 2;

/// One result of [`TrackAligner::push`].
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum AlignOut {
    /// A sample placed on the common timeline.
    Sample(TrackId, Sample),
    /// A track's offset moved by more than [`SR_SWITCH_TOLERANCE_US`]: the timeline stepped.
    Discontinuity,
    /// The placement rule in force changed (reported once per change, before the sample).
    State(AvSync),
}

struct Lane {
    timescale: Timescale,
    first_pts: Option<i64>,
    delta: i64,
    target: Option<i64>,
    placed: bool,
}

/// Sans-IO per-session track aligner; see the module docs for the placement rules.
#[derive(Default)]
pub struct TrackAligner {
    active: bool,
    video: Option<TrackId>,
    lanes: BTreeMap<TrackId, Lane>,
    anchor: Option<(TrackId, i64)>,
    rtp_info: BTreeMap<TrackId, u32>,
    state: Option<AvSync>,
    held: Vec<(TrackId, Sample)>,
    sr_dirty: bool,
    skew_rejections: u64,
    sender_reports_refused: bool,
}

fn rescale(ticks: i64, from: Timescale, to: Timescale) -> i64 {
    rescale_i64(ticks, from, to, Rounding::Nearest)
}

fn wrapped_gap(rtptime: u32, reference: i64) -> i64 {
    let low = u32::try_from(reference & i64::from(u32::MAX)).unwrap_or(0);
    i64::from(i32::from_ne_bytes(rtptime.wrapping_sub(low).to_ne_bytes()))
}

fn ticks_to_us(ticks: i64, timescale: Timescale) -> i128 {
    to_micros(i128::from(ticks), timescale, Rounding::TowardZero)
}

impl TrackAligner {
    /// `tracks` is `(track id, is the video track, RTP clock rate)`; fewer than two tracks
    /// pass every sample through untouched.
    #[must_use]
    pub fn new(tracks: &[(TrackId, bool, Timescale)]) -> Self {
        let lanes = tracks
            .iter()
            .map(|&(id, _, timescale)| {
                (
                    id,
                    Lane {
                        timescale,
                        first_pts: None,
                        delta: 0,
                        target: None,
                        placed: false,
                    },
                )
            })
            .collect();
        Self {
            active: tracks.len() > 1,
            video: tracks.iter().find(|t| t.1).map(|t| t.0),
            lanes,
            ..Self::default()
        }
    }

    /// Records the PLAY response's `RTP-Info` `rtptime` per track; used only when it names every
    /// track.
    pub fn note_rtp_info(&mut self, times: &[(TrackId, u32)]) {
        let complete = self
            .lanes
            .keys()
            .all(|id| times.iter().any(|(t, _)| t == id));
        if self.active && complete {
            self.rtp_info = times.iter().copied().collect();
        }
    }

    /// Call after feeding an RTCP packet to the depacketiser so new Sender Reports are re-read.
    pub fn note_rtcp(&mut self) {
        self.sr_dirty = true;
    }

    /// How many times a fresh set of Sender Reports was refused because the tracks' NTP clocks
    /// were more than an hour apart ([`RtpStreamDepacketiser::try_sync_start_decode_times`]); the
    /// tracks then stay on their provisional placement until the next Sender Report.
    #[must_use]
    pub fn skew_rejections(&self) -> u64 {
        self.skew_rejections
    }

    /// Whether the most recent evaluation of the Sender Reports was refused because the tracks'
    /// NTP clocks were more than an hour apart. While this is `true` the aligner cannot reach
    /// [`AvSync::SenderReport`]: the refusal covers every track at once (the skew check runs over
    /// all anchored tracks, so one mis-set clock blocks the good pair too), and no outlier is
    /// excluded and no fallback is taken. It clears on the first evaluation that is accepted, which
    /// happens only after a later Sender Report arrives ([`Self::note_rtcp`]) and the clocks agree.
    #[must_use]
    pub fn sender_reports_refused(&self) -> bool {
        self.sender_reports_refused
    }

    /// Places `sample`; may hold early non-anchor samples until the anchor track has one.
    pub fn push(
        &mut self,
        depa: &RtpStreamDepacketiser,
        track: TrackId,
        sample: Sample,
    ) -> Vec<AlignOut> {
        let (true, Some(pts)) = (self.active, sample.timing.and_then(|t| t.pts())) else {
            return alloc::vec![AlignOut::Sample(track, sample)];
        };
        let mut out = Vec::new();
        if let Some(lane) = self.lanes.get_mut(&track)
            && lane.first_pts.is_none()
        {
            lane.first_pts = Some(pts);
        }
        if self.anchor.is_none() {
            if self.video.is_none_or(|v| v == track) {
                self.anchor = Some((track, pts));
            } else {
                self.held.push((track, sample));
                if self.held.len() > MAX_HELD_SAMPLES {
                    self.anchor_on_earliest_held();
                    self.release_held(depa, &mut out);
                }
                return out;
            }
            self.refresh(depa);
            self.place(track, sample, &mut out);
            self.release_held(depa, &mut out);
            return out;
        }
        self.refresh(depa);
        self.place(track, sample, &mut out);
        out
    }

    /// Releases every held sample once the stream has ended without the anchor track producing
    /// one, anchoring on the earliest held sample.
    pub fn flush(&mut self, depa: &RtpStreamDepacketiser) -> Vec<AlignOut> {
        let mut out = Vec::new();
        if self.anchor.is_none() {
            self.anchor_on_earliest_held();
        }
        self.release_held(depa, &mut out);
        out
    }

    fn anchor_on_earliest_held(&mut self) {
        if let Some(&(track, _)) = self.held.first()
            && let Some(first) = self.lanes.get(&track).and_then(|l| l.first_pts)
        {
            self.anchor = Some((track, first));
        }
    }

    fn release_held(&mut self, depa: &RtpStreamDepacketiser, out: &mut Vec<AlignOut>) {
        if self.anchor.is_none() {
            return;
        }
        self.refresh(depa);
        for (track, sample) in core::mem::take(&mut self.held) {
            self.place(track, sample, out);
        }
    }

    fn refresh(&mut self, depa: &RtpStreamDepacketiser) {
        let Some((anchor, anchor_first)) = self.anchor else {
            return;
        };
        if !self.rtp_info.is_empty() {
            self.refresh_from_rtp_info(anchor, anchor_first);
            return;
        }
        if !self.sr_dirty {
            return;
        }
        let times: BTreeMap<TrackId, u64> = match depa.try_sync_start_decode_times() {
            Ok(times) => times.into_iter().collect(),
            Err(_) => {
                self.skew_rejections = self.skew_rejections.saturating_add(1);
                self.sender_reports_refused = true;
                self.sr_dirty = false;
                return;
            }
        };
        self.sender_reports_refused = false;
        let Some((&anchor_sdt, anchor_scale)) = times
            .get(&anchor)
            .zip(self.lanes.get(&anchor).map(|l| l.timescale))
        else {
            return;
        };
        let mut complete = true;
        for (id, lane) in &mut self.lanes {
            if *id == anchor {
                continue;
            }
            let (Some(&sdt), Some(first)) = (times.get(id), lane.first_pts) else {
                complete = false;
                continue;
            };
            let anchor_in_lane = rescale(anchor_first, anchor_scale, lane.timescale);
            let (Ok(anchor_sdt), Ok(sdt)) = (i64::try_from(anchor_sdt), i64::try_from(sdt)) else {
                complete = false;
                continue;
            };
            let anchor_sdt_in_lane = rescale(anchor_sdt, anchor_scale, lane.timescale);
            let target = anchor_in_lane
                .checked_add(sdt)
                .and_then(|t| t.checked_sub(anchor_sdt_in_lane))
                .and_then(|t| t.checked_sub(first));
            let Some(target) = target else {
                complete = false;
                continue;
            };
            lane.target = Some(target);
        }
        self.sr_dirty = !complete;
    }

    fn refresh_from_rtp_info(&mut self, anchor: TrackId, anchor_first: i64) {
        let Some((&anchor_rt, anchor_scale)) = self
            .rtp_info
            .get(&anchor)
            .zip(self.lanes.get(&anchor).map(|l| l.timescale))
        else {
            return;
        };
        let anchor_at_start = anchor_first.saturating_add(wrapped_gap(anchor_rt, anchor_first));
        for (id, lane) in &mut self.lanes {
            if *id == anchor || lane.target.is_some() {
                continue;
            }
            let (Some(&rt), Some(first)) = (self.rtp_info.get(id), lane.first_pts) else {
                continue;
            };
            let lane_at_start = first.saturating_add(wrapped_gap(rt, first));
            lane.target = Some(
                rescale(anchor_at_start, anchor_scale, lane.timescale)
                    .saturating_sub(lane_at_start),
            );
        }
    }

    fn place(&mut self, track: TrackId, mut sample: Sample, out: &mut Vec<AlignOut>) {
        let Some((anchor, anchor_first)) = self.anchor else {
            return;
        };
        let mut discontinuity = false;
        if track != anchor {
            let Some(anchor_scale) = self.lanes.get(&anchor).map(|l| l.timescale) else {
                out.push(AlignOut::Sample(track, sample));
                return;
            };
            let Some(lane) = self.lanes.get_mut(&track) else {
                out.push(AlignOut::Sample(track, sample));
                return;
            };
            if !lane.placed {
                lane.delta = lane.target.unwrap_or_else(|| {
                    rescale(anchor_first, anchor_scale, lane.timescale)
                        .saturating_sub(lane.first_pts.unwrap_or(0))
                });
                lane.placed = true;
            } else if let Some(target) = lane.target
                && target != lane.delta
            {
                let diff = target.saturating_sub(lane.delta);
                if ticks_to_us(diff, lane.timescale).abs() > SR_SWITCH_TOLERANCE_US {
                    lane.delta = target;
                    discontinuity = true;
                } else if diff > 0 {
                    lane.delta = target;
                } else {
                    let step = i64::from(sample.timing.and_then(|t| t.duration()).unwrap_or(0))
                        / SLEW_DIVISOR;
                    lane.delta = lane.delta.saturating_add(diff.max(-step));
                }
            }
            if let Some(t) = sample.timing.as_mut() {
                t.shift(lane.delta);
            }
        }
        let state = self.current_state(anchor);
        if discontinuity {
            out.push(AlignOut::Discontinuity);
        }
        if self.state != Some(state) {
            self.state = Some(state);
            out.push(AlignOut::State(state));
        }
        out.push(AlignOut::Sample(track, sample));
    }

    fn current_state(&self, anchor: TrackId) -> AvSync {
        if !self.rtp_info.is_empty()
            && self
                .lanes
                .iter()
                .filter(|(id, _)| **id != anchor)
                .all(|(_, l)| l.target.is_some())
        {
            return AvSync::RtpInfo;
        }
        let all = self
            .lanes
            .iter()
            .filter(|(id, _)| **id != anchor)
            .all(|(_, l)| l.target.is_some());
        if all || self.state == Some(AvSync::SenderReport) {
            AvSync::SenderReport
        } else {
            AvSync::Provisional
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
    use crate::pipeline::CodecConfig;
    use crate::rtp::RtpMediaKind;
    use crate::rtp_stream::RtpStreamTrack;
    use broadcast_common::Serialize;

    const VIDEO: TrackId = TrackId::new(1);
    const OTHER: TrackId = TrackId::new(2);
    const VIDEO_RATE: u32 = 90_000;
    const FPS: u32 = 30;
    const FRAMES: u32 = 24;
    const NTP_SECONDS: u32 = 3_900_000_000;
    const VIDEO_BASE: u32 = 5;
    const WRAPPING_BASE: u32 = u32::MAX - 1_000;
    const LAG_US: i128 = 100_000;
    const SR_ERROR_US: i128 = 50_000;
    const TOLERANCE_US: i128 = 1_000;
    const SR_SSRC: u32 = 7;
    const NTP_FRACTION: u64 = 1 << 32;
    const MICROS: u64 = 1_000_000;
    const NAL_IDR: u8 = 0x65;
    const NAL_SLICE: u8 = 0x41;
    const PAYLOAD_TYPE: u8 = 96;
    const RTP_VERSION_2: u8 = 0x80;
    const MARKER: u8 = 0x80;
    const LONG_STEP_SECONDS: u32 = 10;
    const LONG_RUN_FRAMES: u32 = 12_000;

    fn dummy_avc() -> CodecConfig {
        CodecConfig::Avc {
            config: AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
                configuration_version: 1,
                profile_indication: 0x42,
                profile_compatibility: 0,
                level_indication: 0x1E,
                length_size_minus_one: 3,
                sps: Vec::new(),
                pps: Vec::new(),
                chroma_format: None,
                bit_depth_luma_minus8: None,
                bit_depth_chroma_minus8: None,
                sps_ext: Vec::new(),
            }),
            width: 0,
            height: 0,
        }
    }

    fn rtp(seq: u16, ts: u32, ssrc: u32, nal: u8) -> Vec<u8> {
        let mut pkt = alloc::vec![RTP_VERSION_2, MARKER | PAYLOAD_TYPE];
        pkt.extend_from_slice(&seq.to_be_bytes());
        pkt.extend_from_slice(&ts.to_be_bytes());
        pkt.extend_from_slice(&ssrc.to_be_bytes());
        pkt.push(nal);
        pkt
    }

    fn sender_report(offset_us: i128, rtp_timestamp: u32) -> Vec<u8> {
        let micros = u64::try_from(offset_us.max(0)).unwrap_or(0);
        let whole = u32::try_from(micros / MICROS).unwrap_or(0);
        let lsw = u32::try_from((micros % MICROS) * NTP_FRACTION / MICROS).unwrap_or(0);
        sender_report_at(NTP_SECONDS + whole, lsw, rtp_timestamp)
    }

    fn sender_report_at(msw: u32, lsw: u32, rtp_timestamp: u32) -> Vec<u8> {
        let sr = crate::rtcp::SenderReport::new(SR_SSRC, msw, lsw, rtp_timestamp, 0, 0, Vec::new());
        let mut buf = alloc::vec![0u8; sr.serialized_len()];
        let n = sr.serialize_into(&mut buf).expect("serialize sr");
        buf.truncate(n);
        buf
    }

    struct Rig {
        depa: RtpStreamDepacketiser,
        aligner: TrackAligner,
        other_rate: u32,
        other_base: u32,
        video: Vec<i128>,
        other: Vec<i128>,
        discontinuities: u32,
    }

    impl Rig {
        fn new(other_rate: u32, other_base: u32) -> Self {
            let track = |id, rate| RtpStreamTrack::new(id, RtpMediaKind::H264, dummy_avc(), rate);
            Self {
                depa: RtpStreamDepacketiser::new(alloc::vec![
                    track(VIDEO, Timescale::from_nonzero(VIDEO_RATE)),
                    track(OTHER, Timescale::try_new(other_rate).expect("rate"))
                ]),
                aligner: TrackAligner::new(&[
                    (VIDEO, true, Timescale::from_nonzero(VIDEO_RATE)),
                    (OTHER, false, Timescale::try_new(other_rate).expect("rate")),
                ]),
                other_rate,
                other_base,
                video: Vec::new(),
                other: Vec::new(),
                discontinuities: 0,
            }
        }

        fn sender_reports(&mut self, other_error_us: i128) {
            self.depa.push_rtcp(VIDEO, &sender_report(0, VIDEO_BASE));
            self.depa
                .push_rtcp(OTHER, &sender_report(other_error_us, self.other_base));
            self.aligner.note_rtcp();
        }

        fn frame(&mut self, k: u32) {
            let lag_ticks =
                u32::try_from(i128::from(self.other_rate) * LAG_US / i128::from(MICROS))
                    .expect("lag ticks");
            let step = self.other_rate / FPS;
            let seq = u16::try_from(k).expect("seq");
            let nal = if k == 0 { NAL_IDR } else { NAL_SLICE };
            let video_ts = VIDEO_BASE.wrapping_add(k * (VIDEO_RATE / FPS));
            let other_ts = self.other_base.wrapping_add(lag_ticks + k * step);
            for (id, packet) in [
                (VIDEO, rtp(seq, video_ts, 1, nal)),
                (OTHER, rtp(seq, other_ts, 2, nal)),
            ] {
                for sample in self.depa.push(id, &packet).expect("push") {
                    self.place(id, sample);
                }
            }
        }

        fn place(&mut self, id: TrackId, sample: Sample) {
            for out in self.aligner.push(&self.depa, id, sample) {
                self.record(out);
            }
        }

        fn record(&mut self, out: AlignOut) {
            if matches!(out, AlignOut::Discontinuity) {
                self.discontinuities += 1;
            }
            if let AlignOut::Sample(id, sample) = out {
                let rate = if id == VIDEO {
                    VIDEO_RATE
                } else {
                    self.other_rate
                };
                let us = to_micros(
                    i128::from(sample.timing.and_then(|t| t.pts()).expect("pts")),
                    Timescale::try_new(rate).expect("rate"),
                    Rounding::TowardZero,
                );
                if id == VIDEO {
                    self.video.push(us);
                } else {
                    self.other.push(us);
                }
            }
        }

        fn long_frame(&mut self, k: u32, reports: bool) {
            let lag_ticks =
                u32::try_from(i128::from(self.other_rate) * LAG_US / i128::from(MICROS))
                    .expect("lag ticks");
            let video_ts = VIDEO_BASE.wrapping_add(k.wrapping_mul(LONG_STEP_SECONDS * VIDEO_RATE));
            let other_sr_ts = self
                .other_base
                .wrapping_add(k.wrapping_mul(LONG_STEP_SECONDS * self.other_rate));
            if reports {
                let offset_us = i128::from(k) * i128::from(LONG_STEP_SECONDS) * i128::from(MICROS);
                self.depa
                    .push_rtcp(VIDEO, &sender_report(offset_us, video_ts));
                self.depa
                    .push_rtcp(OTHER, &sender_report(offset_us, other_sr_ts));
                self.aligner.note_rtcp();
            }
            let seq = u16::try_from(k % u32::from(u16::MAX)).expect("seq");
            let nal = if k == 0 { NAL_IDR } else { NAL_SLICE };
            for (id, packet) in [
                (VIDEO, rtp(seq, video_ts, 1, nal)),
                (OTHER, rtp(seq, other_sr_ts.wrapping_add(lag_ticks), 2, nal)),
            ] {
                for sample in self.depa.push(id, &packet).expect("push") {
                    self.place(id, sample);
                }
            }
        }

        fn gap(&self, index: usize) -> i128 {
            self.other[index] - self.video[index]
        }
    }

    use broadcast_common::timescale::{Rounding, to_micros};

    #[test]
    fn sender_reports_align_tracks_whatever_the_clock_rate_or_rtp_wrap() {
        for other_rate in [90_000, 48_000, 44_100] {
            for other_base in [123_456, WRAPPING_BASE] {
                let mut rig = Rig::new(other_rate, other_base);
                rig.sender_reports(0);
                for k in 0..FRAMES {
                    rig.frame(k);
                }
                assert!(rig.other.len() > 4 && rig.video.len() > 4);
                for index in 0..rig.other.len().min(rig.video.len()) {
                    assert!(
                        (rig.gap(index) - LAG_US).abs() < TOLERANCE_US,
                        "rate {other_rate} base {other_base} sample {index}: gap {}us, want {LAG_US}us",
                        rig.gap(index)
                    );
                }
            }
        }
    }

    #[test]
    fn a_later_sender_report_refines_the_anchor() {
        let mut rig = Rig::new(48_000, WRAPPING_BASE);
        rig.sender_reports(SR_ERROR_US);
        for k in 0..FRAMES {
            if k == FRAMES / 2 {
                rig.sender_reports(0);
            }
            rig.frame(k);
        }
        assert!(
            (rig.gap(0) - (LAG_US + SR_ERROR_US)).abs() < TOLERANCE_US,
            "first gap {}us follows the first report",
            rig.gap(0)
        );
        let last = rig.other.len().min(rig.video.len()) - 1;
        assert!(
            (rig.gap(last) - LAG_US).abs() < TOLERANCE_US,
            "final gap {}us follows the refined report",
            rig.gap(last)
        );
    }

    #[test]
    fn before_a_sender_report_the_first_samples_line_up() {
        let mut rig = Rig::new(44_100, WRAPPING_BASE);
        for k in 0..FRAMES {
            rig.frame(k);
        }
        assert!(rig.gap(0).abs() < TOLERANCE_US);
    }

    #[test]
    fn wrapped_gap_is_the_signed_distance_modulo_two_to_the_32() {
        assert_eq!(wrapped_gap(5, i64::from(u32::MAX) - 4), 10);
        assert_eq!(wrapped_gap(u32::MAX, 3), -4);
        assert_eq!(wrapped_gap(7, (1_i64 << 32) + 5), 2);
    }

    #[test]
    fn sender_reports_keep_tracks_aligned_past_several_rtp_wraps() {
        for other_rate in [48_000, 90_000] {
            let mut rig = Rig::new(other_rate, WRAPPING_BASE);
            for k in 0..LONG_RUN_FRAMES {
                rig.long_frame(k, true);
            }
            assert!(rig.video.len() > 1_000 && rig.other.len() > 1_000);
            assert_eq!(rig.discontinuities, 0, "rate {other_rate}");
            for index in 0..rig.other.len().min(rig.video.len()) {
                assert!(
                    (rig.gap(index) - LAG_US).abs() < TOLERANCE_US,
                    "rate {other_rate} sample {index}: gap {}us, want {LAG_US}us",
                    rig.gap(index)
                );
                assert!(rig.other[index] >= 0, "rate {other_rate} sample {index}");
            }
        }
    }

    #[test]
    fn a_sender_report_with_ntp_zero_is_ignored() {
        let mut rig = Rig::new(48_000, WRAPPING_BASE);
        rig.depa.push_rtcp(VIDEO, &sender_report(0, VIDEO_BASE));
        rig.depa
            .push_rtcp(OTHER, &sender_report_at(0, 0, WRAPPING_BASE));
        rig.aligner.note_rtcp();
        assert_eq!(rig.depa.rejected_sender_reports(), 1);
        for k in 0..FRAMES {
            rig.frame(k);
        }
        assert_eq!(rig.discontinuities, 0);
        assert!(rig.gap(0).abs() < TOLERANCE_US, "provisional placement");
        assert!(rig.other.iter().all(|&us| us >= 0));
    }

    #[test]
    fn sender_reports_whose_clocks_are_far_apart_do_not_align() {
        let mut rig = Rig::new(48_000, WRAPPING_BASE);
        rig.depa.push_rtcp(VIDEO, &sender_report(0, VIDEO_BASE));
        rig.depa
            .push_rtcp(OTHER, &sender_report_at(1, 0, WRAPPING_BASE));
        rig.aligner.note_rtcp();
        for k in 0..FRAMES {
            rig.frame(k);
        }
        assert_eq!(rig.discontinuities, 0);
        assert!(rig.other.iter().all(|&us| us >= 0));
        assert_eq!(
            rig.aligner.skew_rejections(),
            1,
            "counted once, not once per sample"
        );
        let skew = rig.depa.try_sync_start_decode_times().unwrap_err();
        assert!(skew.skew_seconds > 3600.0, "{}", skew.skew_seconds);
    }

    #[test]
    fn a_skew_refusal_is_observable_until_consistent_sender_reports_arrive() {
        let mut rig = Rig::new(48_000, WRAPPING_BASE);
        assert!(!rig.aligner.sender_reports_refused());
        rig.depa.push_rtcp(VIDEO, &sender_report(0, VIDEO_BASE));
        rig.depa
            .push_rtcp(OTHER, &sender_report_at(1, 0, WRAPPING_BASE));
        rig.aligner.note_rtcp();
        for k in 0..FRAMES {
            rig.frame(k);
        }
        assert!(
            rig.aligner.sender_reports_refused(),
            "the refusal stands while the clocks disagree"
        );
        rig.sender_reports(0);
        for k in FRAMES..FRAMES * 2 {
            rig.frame(k);
        }
        assert!(!rig.aligner.sender_reports_refused());
    }
}
