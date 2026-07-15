//! Streaming RTP depayloader — RFC 6184 (H.264) / RFC 3640 (AAC).
//!
//! Stateful counterpart to [`crate::rtp::RtpDepacketizer`]: fed RTP packets
//! incrementally via [`RtpStreamDepacketizer::push`], it emits fully-timed
//! [`Sample`]s (real per-AU `duration` from RTP-timestamp deltas, `is_sync`
//! from IDR detection) carrying the real [`CodecConfig`] supplied at
//! construction (e.g. from [`crate::rtp_sdp`]).
//!
//! # Timing model
//!
//! - Per track, the IR timescale is the RTP `clock_rate` (video: 90 kHz;
//!   AAC: the sample rate).
//! - A sample's `duration` is the RTP-timestamp delta to the *next* access
//!   unit's timestamp — so a sample can only be emitted once the following
//!   AU's timestamp is known (one-AU emission latency). [`Self::flush`]
//!   emits the final pending AU using the last-computed duration (there is
//!   no "next" AU to measure against).
//! - The 32-bit wire RTP timestamp is unwrapped to a monotonic `u64` (see
//!   [`unwrap_ts`]).
//! - `is_sync` comes straight from the reassembled [`crate::rtp::ReassembledAu`]
//!   (IDR detection for video; always `true` for audio).
//! - `composition_offset` is always `0` — **v1 assumes low-delay H.264 with no
//!   B-frame reorder**: RTP carries only a presentation timestamp on the
//!   wire, so reconstructing a separate DTS (and therefore a non-zero
//!   composition offset) when B-frames are present is future work.
//! - Each track independently rebases its first unwrapped timestamp to
//!   `start_decode_time = 0` (via the caller building [`crate::media::Track`]
//!   from [`Self::track_specs`] + emitted samples); cross-track A/V alignment
//!   using RTCP Sender Report NTP/RTP correlation is out of v1 scope.

use crate::error::Result;
use crate::pipeline::{CodecConfig, Sample, TrackSpec};
use crate::rtp::{RtpMediaKind, parse_rtp_header, reassemble_audio, reassemble_video};
use alloc::vec::Vec;

/// One track's decode config for [`RtpStreamDepacketizer`].
pub struct RtpStreamTrack {
    /// Track ID (matches the IR `track_id` this depayloader emits).
    pub track_id: u32,
    /// The payload format carried on this track's RTP stream.
    pub kind: RtpMediaKind,
    /// Real codec config (e.g. from [`crate::rtp_sdp`]).
    pub config: CodecConfig,
    /// RTP clock rate (Hz) — also used as the IR track timescale.
    pub clock_rate: u32,
}

/// Per-track mutable depacketization state.
struct TrackState {
    kind: RtpMediaKind,
    config: CodecConfig,
    clock_rate: u32,
    /// RTP timestamp of the packets currently buffered in `cur_pkts` (the
    /// access unit still being assembled).
    cur_ts: Option<u32>,
    /// Packets accumulated for the current (not-yet-complete) RTP timestamp.
    cur_pkts: Vec<Vec<u8>>,
    /// Unwrapped 64-bit form of the most recently completed AU's timestamp.
    last_unwrapped: Option<u64>,
    /// AU awaiting its duration (filled in once the next AU's timestamp is
    /// known).
    pending: Option<PendingAu>,
    /// Last computed duration, reused for the final AU emitted by `flush`.
    last_duration: u32,
}

/// An access unit whose duration is not yet known (waiting on the next AU's
/// timestamp).
struct PendingAu {
    unwrapped_ts: u64,
    is_sync: bool,
    data: Vec<u8>,
}

/// Stateful, timing- and config-aware RTP depayloader (see module docs).
pub struct RtpStreamDepacketizer {
    tracks: Vec<(u32, TrackState)>,
}

/// Unwrap a 32-bit wire RTP timestamp against the last unwrapped value.
///
/// RTP timestamps are a 32-bit field (RFC 3550 §5.1) that wraps every
/// `2^32` ticks. Given the previous unwrapped 64-bit value, this takes its
/// low 32 bits and computes
/// the wire-to-wire delta via a wrapping subtraction reinterpreted as a
/// signed 32-bit integer — the standard idiom for unwrapping any monotonic
/// counter that wraps modulo `2^N` (the same trick used for TCP sequence
/// numbers). That signed delta (forward *or* backward) is then added to the
/// full-width previous value.
///
/// This is exact for any single step whose true magnitude is within half the
/// wrap range (~13.6 hours at a 90 kHz clock) — i.e. any real packet
/// arrival, in order or mildly reordered, between two AUs. It cannot tell a
/// giant forward jump from an equally giant backward jump (an inherent
/// ambiguity of wraparound counters), but that distinction only matters at
/// magnitudes no real RTP stream produces between consecutive AUs.
fn unwrap_ts(prev: Option<u64>, ts: u32) -> u64 {
    let Some(prev) = prev else {
        return u64::from(ts);
    };
    let prev_low = prev as u32;
    let delta = ts.wrapping_sub(prev_low) as i32;
    if delta >= 0 {
        prev + u64::from(delta as u32)
    } else {
        prev.saturating_sub(u64::from(delta.unsigned_abs()))
    }
}

impl RtpStreamDepacketizer {
    /// Build a depayloader for the given tracks (kind, config, clock rate).
    pub fn new(tracks: Vec<RtpStreamTrack>) -> Self {
        let tracks = tracks
            .into_iter()
            .map(|t| {
                (
                    t.track_id,
                    TrackState {
                        kind: t.kind,
                        config: t.config,
                        clock_rate: t.clock_rate,
                        cur_ts: None,
                        cur_pkts: Vec::new(),
                        last_unwrapped: None,
                        pending: None,
                        last_duration: 0,
                    },
                )
            })
            .collect();
        Self { tracks }
    }

    /// Build the [`TrackSpec`]s (timescale = `clock_rate`) for init-segment
    /// construction.
    pub fn track_specs(&self) -> Vec<TrackSpec> {
        self.tracks
            .iter()
            .map(|(id, st)| TrackSpec::new(*id, st.clock_rate, st.config.clone()))
            .collect()
    }

    fn state(&mut self, track_id: u32) -> Option<&mut TrackState> {
        self.tracks
            .iter_mut()
            .find(|(id, _)| *id == track_id)
            .map(|(_, st)| st)
    }

    /// Feed one RTP packet for `track_id`. Returns any [`Sample`]s that
    /// became fully timed as a result (zero, one — the AU this packet
    /// completed — the previous AU emitted with its now-known duration).
    /// Unknown `track_id`s are silently ignored (return an empty `Vec`).
    pub fn push(&mut self, track_id: u32, rtp_packet: &[u8]) -> Result<Vec<Sample>> {
        let Some(st) = self.state(track_id) else {
            return Ok(Vec::new());
        };
        let hdr = parse_rtp_header(rtp_packet)?;
        let ts = hdr.timestamp;
        let mut out = Vec::new();

        // A timestamp change while packets are buffered means the previous
        // timestamp's packets already form a complete AU (defensive: covers
        // a dropped/missing marker bit).
        if let Some(cur) = st.cur_ts {
            if cur != ts && !st.cur_pkts.is_empty() {
                Self::drain_complete(st, &mut out)?;
            }
        }
        st.cur_ts = Some(ts);
        st.cur_pkts.push(rtp_packet.to_vec());

        // The video marker bit ends an AU immediately (RFC 6184 §5.1).
        if matches!(st.kind, RtpMediaKind::H264) && hdr.marker {
            Self::drain_complete(st, &mut out)?;
            st.cur_ts = None;
        }
        Ok(out)
    }

    /// Flush a track at end-of-stream: reassemble any buffered packets, and
    /// emit the final pending AU using the last-known duration (there is no
    /// following AU to measure a real delta against).
    pub fn flush(&mut self, track_id: u32) -> Result<Vec<Sample>> {
        let Some(st) = self.state(track_id) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        if !st.cur_pkts.is_empty() {
            Self::drain_complete(st, &mut out)?;
            st.cur_ts = None;
        }
        if let Some(p) = st.pending.take() {
            out.push(Sample::new(p.data, st.last_duration, p.is_sync, 0));
        }
        Ok(out)
    }

    /// Reassemble the buffered packets into AUs, then for each: unwrap its
    /// timestamp and emit the previously-pending AU with `duration` = the
    /// delta to this AU's timestamp.
    fn drain_complete(st: &mut TrackState, out: &mut Vec<Sample>) -> Result<()> {
        let pkts = core::mem::take(&mut st.cur_pkts);
        let aus = match st.kind {
            RtpMediaKind::H264 => reassemble_video(&pkts)?,
            RtpMediaKind::Aac => reassemble_audio(&pkts)?,
        };
        for au in aus {
            let unwrapped = unwrap_ts(st.last_unwrapped, au.timestamp);
            st.last_unwrapped = Some(unwrapped);
            if let Some(prev) = st.pending.take() {
                let delta = unwrapped.saturating_sub(prev.unwrapped_ts);
                let duration = u32::try_from(delta).unwrap_or(u32::MAX);
                st.last_duration = duration;
                out.push(Sample::new(prev.data, duration, prev.is_sync, 0));
            }
            st.pending = Some(PendingAu {
                unwrapped_ts: unwrapped,
                is_sync: au.is_sync,
                data: au.data,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};

    fn dummy_avc() -> CodecConfig {
        CodecConfig::Avc {
            config: AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
                configuration_version: 1,
                profile_indication: 0x42,
                profile_compatibility: 0,
                level_indication: 0x1E,
                length_size_minus_one: 3,
                sps: alloc::vec![],
                pps: alloc::vec![],
                chroma_format: None,
                bit_depth_luma_minus8: None,
                bit_depth_chroma_minus8: None,
                sps_ext: alloc::vec![],
            }),
            width: 0,
            height: 0,
        }
    }

    fn vpkt(seq: u16, ts: u32, marker: bool, nal: &[u8]) -> Vec<u8> {
        let mut p = alloc::vec![0x80u8, if marker { 0x80 | 96 } else { 96 }];
        p.extend_from_slice(&seq.to_be_bytes());
        p.extend_from_slice(&ts.to_be_bytes());
        p.extend_from_slice(&[0, 0, 0, 0]);
        p.extend_from_slice(nal);
        p
    }

    #[test]
    fn video_stream_recovers_durations_and_sync() {
        let mut d = RtpStreamDepacketizer::new(alloc::vec![RtpStreamTrack {
            track_id: 1,
            kind: RtpMediaKind::H264,
            config: dummy_avc(),
            clock_rate: 90_000,
        }]);

        // AU0 @1000 (IDR), AU1 @4000 (non-IDR), AU2 @7000 (non-IDR). 3000-tick spacing.
        let idr = [0x65u8, 0xAA];
        let non = [0x41u8, 0xBB];
        // AU0: emits nothing yet (duration needs AU1).
        assert!(d.push(1, &vpkt(1, 1000, true, &idr)).unwrap().is_empty());
        // AU1 arrives → AU0 emitted with duration 3000, is_sync=true.
        let s0 = d.push(1, &vpkt(2, 4000, true, &non)).unwrap();
        assert_eq!(s0.len(), 1);
        assert_eq!(s0[0].duration, 3000);
        assert!(s0[0].is_sync);
        assert_eq!(s0[0].composition_offset, 0);
        // AU2 arrives → AU1 emitted, duration 3000, is_sync=false.
        let s1 = d.push(1, &vpkt(3, 7000, true, &non)).unwrap();
        assert_eq!(s1.len(), 1);
        assert_eq!(s1[0].duration, 3000);
        assert!(!s1[0].is_sync);
        // flush → AU2 emitted with the last-known duration (3000).
        let s2 = d.flush(1).unwrap();
        assert_eq!(s2.len(), 1);
        assert_eq!(s2[0].duration, 3000);
    }

    #[test]
    fn track_specs_use_clock_rate_as_timescale() {
        let d = RtpStreamDepacketizer::new(alloc::vec![RtpStreamTrack {
            track_id: 7,
            kind: RtpMediaKind::H264,
            config: dummy_avc(),
            clock_rate: 90_000,
        }]);
        let specs = d.track_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].track_id, 7);
        assert_eq!(specs[0].timescale, 90_000);
    }
}
