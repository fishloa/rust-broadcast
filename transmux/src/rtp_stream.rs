//! Streaming RTP depayloader — RFC 6184 (H.264) / RFC 3640 (AAC).
//!
//! Stateful counterpart to [`crate::rtp::RtpDepacketiser`]: fed RTP packets
//! incrementally via [`RtpStreamDepacketiser::push`], it emits fully-timed
//! [`Sample`]s (real per-AU `duration` from RTP-timestamp deltas, `is_sync`
//! from IDR detection) carrying the real [`CodecConfig`] supplied at
//! construction (e.g. from [`crate::rtp_sdp`]).
//!
//! See [`transmux/docs/rtp/rtp-payload-formats.md`](../rtp/rtp-payload-formats.md)
//! for the RFC background and timing-model specification.
//!
//! # Timing model
//!
//! - Per track, the IR timescale is the RTP `clock_rate` (video: 90 kHz;
//!   AAC: the sample rate).
//! - A sample's `duration` is the RTP-timestamp delta to the *next* access
//!   unit's timestamp — so a sample can only be emitted once the following
//!   AU's timestamp is known (one-AU emission latency). `flush`
//!   emits the final pending AU using the last-computed duration (there is
//!   no "next" AU to measure against).
//! - The 32-bit wire RTP timestamp is unwrapped to a monotonic `u64`.
//! - `is_sync` comes straight from the reassembled access unit
//!   (IDR detection for video; always `true` for audio).
//! - `composition_offset` is always `0` — **v1 assumes low-delay H.264 with no
//!   B-frame reorder**: RTP carries only a presentation timestamp on the
//!   wire, so reconstructing a separate DTS (and therefore a non-zero
//!   composition offset) when B-frames are present is future work.
//! - Each track independently rebases its first unwrapped timestamp to
//!   `start_decode_time = 0` (via the caller building [`crate::media::Track`]
//!   from `track_specs` + emitted samples) **unless** an RTCP Sender Report
//!   has been fed for at least two tracks (issue #722): RTP timestamps alone
//!   carry no cross-stream relationship (RFC 3550 §5.1 — each SSRC's clock
//!   has an arbitrary random offset), so recovering true A/V sync needs the
//!   NTP-wallclock ↔ RTP-timestamp correlation each Sender Report carries
//!   (RFC 3550 §6.4.1). Feed reports via [`RtpStreamDepacketiser::push_sender_report`]
//!   / [`RtpStreamDepacketiser::push_rtcp`] as they arrive on a track's RTCP
//!   channel, then read [`RtpStreamDepacketiser::sync_start_decode_times`]
//!   once at least two tracks have an anchor: it maps every anchored
//!   track's first sample onto one common wallclock and returns each
//!   track's `start_decode_time` (in that track's own `clock_rate` ticks)
//!   relative to the earliest of them — preserving the real inter-track
//!   offset instead of discarding it. This is strictly additive: with no
//!   Sender Reports fed (or fewer than two anchored tracks), the method
//!   returns an empty `Vec` and callers keep the v1
//!   independent-rebase-to-0 behaviour unchanged.
//! - When an RFC 3640 `AAC-hbr` packet aggregates more than one access unit,
//!   all AUs in that packet share the RTP timestamp, so non-final AUs get
//!   `duration = 0`; v1 assumes one AU per packet, which is what transmux's
//!   own packetiser emits.

use crate::error::{Error, Result};
use crate::pipeline::{CodecConfig, Sample, TrackSpec};
use crate::rtcp::SenderReport;
use crate::rtp::{RtpMediaKind, parse_rtp_header, reassemble_audio, reassemble_video};
use alloc::vec::Vec;
use broadcast_common::Parse;

/// Number of fractional-second units in the 32.32 fixed-point NTP timestamp
/// format an RTCP Sender Report's `ntp_msw`/`ntp_lsw` carry (RFC 3550 §6.4.1,
/// citing RFC 5905 §6 for the NTP timestamp format itself): `2^32`.
const NTP_FRACTION_SCALE: f64 = 4_294_967_296.0;

/// Hard cap, in bytes, on one track's in-progress access-unit buffer
/// (`TrackState::cur_pkts`, the raw RTP packets accumulated since the last
/// completed AU) — the same buffer that ultimately feeds
/// [`crate::rtp::reassemble_video`]'s FU-A `fu_buf`, so bounding it here
/// transitively bounds that too. A real access unit (even a 4K IDR frame)
/// is at most a few hundred KB; this is comfortably above that, but far
/// below what a malformed/hostile stream — a dropped final FU-A fragment
/// (`E=1` never seen) or a marker bit that's never set — would otherwise
/// accumulate for the life of the (indefinitely long, per the P0 reconnect
/// loop) session: an unbounded-memory DoS (audit-ingest #4). On overflow the
/// in-progress AU is dropped and [`RtpStreamDepacketiser::push`] returns a
/// recoverable [`Error::BufferCapExceeded`]; internal state is already reset
/// so the next packet starts a fresh AU (resync on the next timestamp change
/// or marker bit).
const MAX_AU_BUFFER_BYTES: usize = 4 * 1024 * 1024;

/// One track's decode config for [`RtpStreamDepacketiser`].
#[non_exhaustive]
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

impl RtpStreamTrack {
    /// Build a track config from its fields.
    pub fn new(track_id: u32, kind: RtpMediaKind, config: CodecConfig, clock_rate: u32) -> Self {
        Self {
            track_id,
            kind,
            config,
            clock_rate,
        }
    }
}

/// Per-track mutable depacketisation state.
struct TrackState {
    kind: RtpMediaKind,
    config: CodecConfig,
    clock_rate: u32,
    /// RTP timestamp of the packets currently buffered in `cur_pkts` (the
    /// access unit still being assembled).
    cur_ts: Option<u32>,
    /// Packets accumulated for the current (not-yet-complete) RTP timestamp.
    cur_pkts: Vec<Vec<u8>>,
    /// Total bytes across `cur_pkts` — tracked incrementally so
    /// [`MAX_AU_BUFFER_BYTES`] can be enforced without re-summing the vec on
    /// every packet.
    cur_bytes: usize,
    /// Unwrapped 64-bit form of the most recently completed AU's timestamp.
    last_unwrapped: Option<u64>,
    /// Unwrapped 64-bit form of this track's very first completed AU's
    /// timestamp — the anchor [`RtpStreamDepacketiser::sync_start_decode_times`]
    /// measures this track's RTCP SR offset against.
    first_unwrapped: Option<u64>,
    /// AU awaiting its duration (filled in once the next AU's timestamp is
    /// known).
    pending: Option<PendingAu>,
    /// Last computed duration, reused for the final AU emitted by `flush`.
    last_duration: u32,
    /// This track's most recent RTCP Sender Report anchor, if any has been
    /// fed via [`RtpStreamDepacketiser::push_sender_report`].
    sr_anchor: Option<SrAnchor>,
}

/// One track's RTCP Sender Report wallclock anchor (RFC 3550 §6.4.1): the NTP
/// wallclock instant at which the sender's RTP clock read `raw_rtp_ts`.
///
/// `raw_rtp_ts` is kept in wire (32-bit) form rather than eagerly unwrapped:
/// an SR can be fed before, interleaved with, or after the access units it
/// anchors, so it is unwrapped lazily in [`wall_seconds`] against whichever
/// AU's own unwrapped timestamp needs a wallclock value — always valid
/// because a real SR's RTP timestamp is at most a few RTCP-interval seconds
/// from the AUs it anchors, far under half the 32-bit wrap range.
struct SrAnchor {
    /// NTP wallclock time, in seconds (32.32 fixed-point `ntp_msw`/`ntp_lsw`
    /// converted to `f64`).
    ntp_seconds: f64,
    /// The wire-form (32-bit) RTP timestamp corresponding to `ntp_seconds`.
    raw_rtp_ts: u32,
}

/// Resolve one access unit's NTP wallclock instant from a track's SR anchor:
/// `wall(au) = anchor.ntp_seconds + (unwrapped_au_ts − unwrapped_anchor_ts) /
/// clock_rate` (RFC 3550 §6.4.1's NTP/RTP correlation, generalised from the
/// SR's own instant to any AU on the same RTP clock).
fn wall_seconds(anchor: &SrAnchor, clock_rate: u32, au_unwrapped_ts: u64) -> f64 {
    let anchor_unwrapped = unwrap_ts(Some(au_unwrapped_ts), anchor.raw_rtp_ts);
    let delta_ticks = au_unwrapped_ts as i128 - i128::from(anchor_unwrapped);
    anchor.ntp_seconds + (delta_ticks as f64) / f64::from(clock_rate)
}

/// An access unit whose duration is not yet known (waiting on the next AU's
/// timestamp).
struct PendingAu {
    unwrapped_ts: u64,
    is_sync: bool,
    data: Vec<u8>,
}

/// Stateful, timing- and config-aware RTP depayloader (see module docs).
pub struct RtpStreamDepacketiser {
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

impl RtpStreamDepacketiser {
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
                        cur_bytes: 0,
                        last_unwrapped: None,
                        first_unwrapped: None,
                        pending: None,
                        last_duration: 0,
                        sr_anchor: None,
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

    /// Feed one RTCP Sender Report (RFC 3550 §6.4.1) for `track_id`, anchoring
    /// this track's wallclock for [`Self::sync_start_decode_times`]. Replaces
    /// any previous anchor for the track (the most recent SR wins); unknown
    /// `track_id`s are silently ignored, matching [`Self::push`].
    pub fn push_sender_report(&mut self, track_id: u32, sr: SenderReport) {
        if let Some(st) = self.state(track_id) {
            let ntp_seconds = f64::from(sr.ntp_msw) + f64::from(sr.ntp_lsw) / NTP_FRACTION_SCALE;
            st.sr_anchor = Some(SrAnchor {
                ntp_seconds,
                raw_rtp_ts: sr.rtp_timestamp,
            });
        }
    }

    /// Parse `bytes` as a single RTCP Sender Report (RFC 3550 §6.4.1) and feed
    /// it to [`Self::push_sender_report`]. Anything that isn't a parseable SR
    /// (a Receiver Report, a different compound-packet member, malformed
    /// bytes) is silently ignored — this is a convenience wrapper for
    /// callers holding raw RTCP bytes off the wire, not a general RTCP
    /// dispatcher.
    pub fn push_rtcp(&mut self, track_id: u32, bytes: &[u8]) {
        if let Ok(sr) = SenderReport::parse(bytes) {
            self.push_sender_report(track_id, sr);
        }
    }

    /// Compute the `start_decode_time` (in each track's own `clock_rate`
    /// ticks) that rebases every SR-anchored track onto one common wallclock,
    /// preserving their real inter-track offset (issue #722; RFC 3550
    /// §6.4.1). The earliest anchored track's first sample becomes the
    /// origin (`start_decode_time = 0`); every other anchored track's
    /// `start_decode_time` is its first sample's wallclock distance from
    /// that origin, converted to its own clock rate.
    ///
    /// Returns an empty `Vec` — the v1 opt-out — unless at least two tracks
    /// have both received a Sender Report ([`Self::push_sender_report`]) and
    /// emitted a first sample. A track absent from the returned `Vec` (no
    /// anchor, or fewer than two anchored tracks overall) keeps the existing
    /// independent-rebase-to-0 behaviour: the caller should only apply the
    /// returned `start_decode_time` to tracks it names.
    pub fn sync_start_decode_times(&self) -> Vec<(u32, u64)> {
        let anchored: Vec<(u32, f64, u32)> = self
            .tracks
            .iter()
            .filter_map(|(id, st)| {
                let anchor = st.sr_anchor.as_ref()?;
                let first_ts = st.first_unwrapped?;
                Some((
                    *id,
                    wall_seconds(anchor, st.clock_rate, first_ts),
                    st.clock_rate,
                ))
            })
            .collect();
        if anchored.len() < 2 {
            return Vec::new();
        }
        let origin = anchored
            .iter()
            .map(|(_, wall, _)| *wall)
            .fold(f64::INFINITY, f64::min);
        anchored
            .into_iter()
            .map(|(id, wall, clock_rate)| {
                let raw_ticks = ((wall - origin) * f64::from(clock_rate)).max(0.0);
                // `f64::round` is a `std`-only inherent method (it needs
                // libm), unavailable in this crate's `no_std` core build —
                // round-half-up via a truncating cast instead, which is
                // exact for `round()`'s behaviour given `raw_ticks >= 0.0`.
                let ticks = (raw_ticks + 0.5) as u64;
                (id, ticks)
            })
            .collect()
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
        st.cur_bytes += rtp_packet.len();
        st.cur_pkts.push(rtp_packet.to_vec());

        // Runaway AU — a dropped final FU-A fragment or a marker bit that
        // never arrives would otherwise grow `cur_pkts` forever (see
        // `MAX_AU_BUFFER_BYTES`). Drop the partial unit and resync: the next
        // packet starts a fresh AU exactly as if this were the first packet
        // ever seen for this track.
        if st.cur_bytes > MAX_AU_BUFFER_BYTES {
            st.cur_pkts.clear();
            st.cur_bytes = 0;
            st.cur_ts = None;
            return Err(Error::BufferCapExceeded {
                what: "RTP access-unit reassembly",
                cap: MAX_AU_BUFFER_BYTES,
            });
        }

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
        st.cur_bytes = 0;
        let aus = match st.kind {
            RtpMediaKind::H264 => reassemble_video(&pkts)?,
            RtpMediaKind::Aac => reassemble_audio(&pkts)?,
        };
        for au in aus {
            let unwrapped = unwrap_ts(st.last_unwrapped, au.timestamp);
            st.last_unwrapped = Some(unwrapped);
            if st.first_unwrapped.is_none() {
                st.first_unwrapped = Some(unwrapped);
            }
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
        let mut d = RtpStreamDepacketiser::new(alloc::vec![RtpStreamTrack::new(
            1,
            RtpMediaKind::H264,
            dummy_avc(),
            90_000,
        )]);

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

    /// Builds one FU-A (RFC 6184 §5.8) fragment payload: `fu_indicator` +
    /// `fu_header` (start bit only on the first fragment, end bit **never**
    /// set) + `extra_len` bytes of filler — the "dropped final fragment"
    /// scenario audit-ingest #4 flags.
    fn fu_a_fragment(start: bool, extra_len: usize) -> Vec<u8> {
        const NAL_TYPE_FU_A: u8 = 28;
        const FU_START: u8 = 0x80;
        const ORIG_TYPE_IDR: u8 = 5;
        let fu_header = if start {
            FU_START | ORIG_TYPE_IDR
        } else {
            ORIG_TYPE_IDR
        };
        let mut payload = alloc::vec![NAL_TYPE_FU_A, fu_header];
        payload.extend(core::iter::repeat_n(0xABu8, extra_len));
        payload
    }

    /// A never-terminating FU-A run (end bit never set, marker bit never
    /// set, same RTP timestamp throughout — exactly a dropped/corrupted
    /// final fragment or a hostile encoder) must not grow `cur_pkts`
    /// without bound: [`MAX_AU_BUFFER_BYTES`] must trip, dropping the
    /// partial AU, and the depacketiser must keep working normally
    /// afterward (resync proof) rather than being wedged or OOMing.
    #[test]
    fn runaway_fu_a_without_end_bit_is_bounded_not_unbounded() {
        let mut d = RtpStreamDepacketiser::new(alloc::vec![RtpStreamTrack::new(
            1,
            RtpMediaKind::H264,
            dummy_avc(),
            90_000,
        )]);

        // Each fragment carries ~2 KiB of filler; MAX_AU_BUFFER_BYTES (4 MiB)
        // must trip well before we'd reach an unreasonable iteration count —
        // bounding proves the cap, not exhausting real memory.
        const FRAGMENT_FILLER: usize = 2048;
        let mut hit_cap = false;
        for i in 0..4096u16 {
            match d.push(
                1,
                &vpkt(i, 1000, false, &fu_a_fragment(i == 0, FRAGMENT_FILLER)),
            ) {
                Ok(samples) => assert!(
                    samples.is_empty(),
                    "a never-completing AU must not emit a sample"
                ),
                Err(e) => {
                    assert!(
                        matches!(e, crate::error::Error::BufferCapExceeded { .. }),
                        "unexpected error variant: {e:?}"
                    );
                    hit_cap = true;
                    break;
                }
            }
        }
        assert!(
            hit_cap,
            "expected MAX_AU_BUFFER_BYTES to trip well within {} fragments \
             (never grow unbounded)",
            4096
        );

        // Resync proof: normal AUs at fresh timestamps process exactly as if
        // nothing had gone wrong — the overflow reset internal state cleanly.
        let idr = [0x65u8, 0xAA];
        let non = [0x41u8, 0xBB];
        assert!(d.push(1, &vpkt(1, 4000, true, &idr)).unwrap().is_empty());
        let s = d.push(1, &vpkt(2, 7000, true, &non)).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].duration, 3000);
        assert!(s[0].is_sync);
    }

    #[test]
    fn track_specs_use_clock_rate_as_timescale() {
        let d = RtpStreamDepacketiser::new(alloc::vec![RtpStreamTrack::new(
            7,
            RtpMediaKind::H264,
            dummy_avc(),
            90_000,
        )]);
        let specs = d.track_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].track_id, 7);
        assert_eq!(specs[0].timescale, 90_000);
    }
}
