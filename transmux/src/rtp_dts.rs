//! Decode-timestamp derivation for RTP video that carries only a
//! presentation time.
//!
//! RFC 6184 §5.1 sets the RTP timestamp of an H.264 access unit to its
//! sampling (presentation) time, and the access units arrive in decoding order
//! (non-interleaved packetization mode, `private/specs-verified/ietf/specs/rfc6184_rtp_h264.txt`
//! §5.5). A stream with B-frames therefore reaches the depacketiser with
//! non-monotonic presentation timestamps and no decode time at all.
//!
//! # Rule
//!
//! Let `N` be the codec's reorder depth: the largest number of frames that
//! precede any frame in decoding order and follow it in output order. ITU-T
//! H.264 §E.2.1 bounds it with `max_num_reorder_frames`
//! (`private/specs-verified/mpeg/transmux/codec/h264-vui-bitstream-restriction.md`).
//! A frame decoded `k`-th (0-based) then has at least `k - N` frames before it
//! in output order, so its presentation timestamp is no earlier than the
//! `(k - N)`-th smallest presentation timestamp `P(k - N)`. The derived decode
//! timestamp is
//!
//! ```text
//! DTS(k) = P(k - N)                      for k >= N
//! DTS(k) = P(0) - (N - k) * period       for k <  N
//! ```
//!
//! where `P(j)` is the `j`-th smallest presentation timestamp seen so far and
//! `period` is the declared frame period (H.264 §E.2.1 `timing_info`), else the
//! smallest gap between sorted presentation timestamps at the anchor. This
//! gives `DTS <= PTS` and a strictly increasing `DTS` on every sample, with a
//! `DTS` grid that is the `PTS` grid shifted by `N` frames. `N = 0` yields
//! `DTS == PTS`.
//!
//! # Latency
//!
//! `P(k - N)` is known the moment frame `k` is decoded (the frame with output
//! index `k - N` has decoding index at most `k`), so in steady state a sample
//! leaves with no added delay. Only the start of a run holds samples:
//!
//! - `N` known from the SPS (`max_num_reorder_frames`, or the §E.2.1 / §A.2.1
//!   inferences for intra-only and Baseline profiles): the first `N` samples
//!   wait for `P(0)`, which is the minimum of the first `N + 1`. That is `N`
//!   frame periods once per run.
//! - `N` not declared (no `bitstream_restriction`): the first
//!   [`PROBE_AUS`] samples are held while `N` is measured as the largest
//!   count of earlier-decoded frames with a later presentation time; the
//!   spec's own inference (`MaxDpbFrames`, up to 16) is deliberately not used,
//!   because it would hold 16 frames for a stream that has no reorder.
//!
//! # Failure handling
//!
//! A frame whose presentation time is earlier than a decode timestamp already
//! issued means the assumed `N` was too small (a declared bound that the
//! stream violates, or reordering deeper than the probe saw). The sample keeps
//! a strictly increasing `DTS` (previous plus one tick, which exceeds its
//! `PTS`), `N` grows by one up to [`MAX_REORDER_DEPTH`], and the event is
//! returned for the caller to report. Packet loss needs no special case: a
//! missing access unit is absent from the sorted set, so every rank shifts
//! together and both invariants still hold.

use alloc::vec::Vec;

use crate::ir::Sample;

pub(crate) use crate::rtp_stream::MAX_REORDER_DEPTH;

pub(crate) const PROBE_AUS: usize = 8;

pub(crate) struct DepthExceeded {
    pub(crate) depth: usize,
    pub(crate) pts: i64,
    pub(crate) dts: i64,
}

pub(crate) struct DtsDeriver {
    depth: Option<usize>,
    seen: usize,
    held: Vec<Sample>,
    window: Vec<i64>,
    last_dts: Option<i64>,
    anchored: bool,
}

impl DtsDeriver {
    pub(crate) fn new(declared: Option<usize>) -> Self {
        Self {
            depth: declared.map(|n| n.min(MAX_REORDER_DEPTH)),
            seen: 0,
            held: Vec::new(),
            window: Vec::new(),
            last_dts: None,
            anchored: false,
        }
    }

    pub(crate) fn set_declared(&mut self, declared: Option<usize>) {
        if self.seen == 0 && self.held.is_empty() {
            self.depth = declared.map(|n| n.min(MAX_REORDER_DEPTH));
        }
    }

    pub(crate) fn feed(
        &mut self,
        sample: Sample,
        period: Option<i64>,
        out: &mut Vec<Sample>,
        exceeded: &mut Vec<DepthExceeded>,
    ) {
        if sample.pts.is_none() {
            out.push(sample);
            return;
        }
        if self.depth.is_none() {
            self.held.push(sample);
            if self.held.len() >= PROBE_AUS {
                self.settle_probe(period, out, exceeded);
            }
            return;
        }
        self.run(sample, period, out, exceeded);
    }

    pub(crate) fn finish(
        &mut self,
        period: Option<i64>,
        out: &mut Vec<Sample>,
        exceeded: &mut Vec<DepthExceeded>,
    ) {
        if self.depth.is_none() {
            self.settle_probe(period, out, exceeded);
        }
        if !self.anchored && !self.held.is_empty() {
            self.anchor(period, false, out);
        }
        self.reset_run();
    }

    pub(crate) fn reset_run(&mut self) {
        self.seen = 0;
        self.held.clear();
        self.window.clear();
        self.last_dts = None;
        self.anchored = false;
    }

    fn settle_probe(
        &mut self,
        period: Option<i64>,
        out: &mut Vec<Sample>,
        exceeded: &mut Vec<DepthExceeded>,
    ) {
        let probe = core::mem::take(&mut self.held);
        let mut lag = 0usize;
        for (i, s) in probe.iter().enumerate() {
            let pts = s.pts.unwrap_or_default();
            let ahead = probe[..i]
                .iter()
                .filter(|e| e.pts.unwrap_or_default() > pts)
                .count();
            lag = lag.max(ahead);
        }
        self.depth = Some(lag.min(MAX_REORDER_DEPTH));
        for s in probe {
            self.run(s, period, out, exceeded);
        }
    }

    fn run(
        &mut self,
        sample: Sample,
        period: Option<i64>,
        out: &mut Vec<Sample>,
        exceeded: &mut Vec<DepthExceeded>,
    ) {
        let depth = self.depth.unwrap_or(0);
        let pts = sample.pts.unwrap_or_default();
        let at = self.window.partition_point(|&p| p < pts);
        self.window.insert(at, pts);
        if !self.anchored {
            self.held.push(sample);
            self.seen += 1;
            if self.seen > depth {
                self.anchor(period, true, out);
            }
            return;
        }
        self.seen += 1;
        let mut sample = sample;
        let smallest = self.window[0];
        let prev = self.last_dts.unwrap_or(i64::MIN);
        let dts = if smallest > prev {
            self.window.remove(0)
        } else if depth < MAX_REORDER_DEPTH {
            self.depth = Some(depth + 1);
            let dts = prev.saturating_add(1);
            exceeded.push(DepthExceeded {
                depth: depth + 1,
                pts,
                dts,
            });
            dts
        } else {
            self.window.remove(0);
            let dts = prev.saturating_add(1);
            exceeded.push(DepthExceeded { depth, pts, dts });
            dts
        };
        self.last_dts = Some(dts);
        sample.dts = Some(dts);
        out.push(sample);
    }

    fn anchor(&mut self, period: Option<i64>, pop_last: bool, out: &mut Vec<Sample>) {
        let depth = self.depth.unwrap_or(0);
        let origin = self.window.first().copied().unwrap_or_default();
        let step = period
            .filter(|&p| p > 0)
            .or_else(|| {
                self.window
                    .windows(2)
                    .map(|w| w[1] - w[0])
                    .filter(|&g| g > 0)
                    .min()
            })
            .or_else(|| {
                self.held
                    .iter()
                    .find_map(|s| s.duration.map(i64::from))
                    .filter(|&d| d > 0)
            })
            .unwrap_or(1);
        let held = core::mem::take(&mut self.held);
        let last = held.len().saturating_sub(1);
        for (j, mut s) in held.into_iter().enumerate() {
            let dts = if j < depth {
                let behind = i64::try_from(depth - j).unwrap_or(i64::MAX);
                origin.saturating_sub(behind.saturating_mul(step))
            } else if pop_last && j == last {
                self.window.remove(0)
            } else {
                origin
            };
            self.last_dts = Some(dts);
            s.dts = Some(dts);
            out.push(s);
        }
        self.anchored = true;
        if !pop_last {
            self.window.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: i64 = 3600;

    fn sample(pts: i64) -> Sample {
        Sample::new(alloc::vec![0u8; 4], Some(pts), Some(pts), Some(3600), true)
    }

    fn pyramid_wire(frames: i64) -> Vec<i64> {
        let mut out = alloc::vec![0];
        let mut base = 0;
        while (out.len() as i64) < frames {
            out.extend([base + 4, base + 2, base + 1, base + 3]);
            base += 4;
        }
        out.truncate(frames as usize);
        out.into_iter().map(|f| f * D).collect()
    }

    fn run(depth: Option<usize>, pts: &[i64], finish: bool) -> (Vec<Sample>, Vec<DepthExceeded>) {
        let mut d = DtsDeriver::new(depth);
        let mut out = Vec::new();
        let mut exceeded = Vec::new();
        for &p in pts {
            d.feed(sample(p), Some(D), &mut out, &mut exceeded);
        }
        if finish {
            d.finish(Some(D), &mut out, &mut exceeded);
        }
        (out, exceeded)
    }

    fn dts_of(out: &[Sample]) -> Vec<i64> {
        out.iter().map(|s| s.dts.unwrap()).collect()
    }

    fn assert_invariants(out: &[Sample]) {
        for w in out.windows(2) {
            assert!(w[0].dts < w[1].dts, "dts not strictly increasing");
        }
        for s in out {
            assert!(s.dts <= s.pts, "dts {:?} > pts {:?}", s.dts, s.pts);
        }
    }

    #[test]
    fn declared_depth_two_yields_the_pts_grid_shifted_by_two_frames() {
        let wire = pyramid_wire(40);
        let (out, exceeded) = run(Some(2), &wire, true);
        assert!(exceeded.is_empty());
        assert_eq!(out.len(), wire.len());
        assert_invariants(&out);
        for (k, s) in out.iter().enumerate() {
            assert_eq!(s.dts, Some((k as i64 - 2) * D), "frame {k}");
            assert_eq!(s.pts, Some(wire[k]), "pts untouched, order untouched");
        }
    }

    #[test]
    fn depth_zero_leaves_dts_equal_to_pts_without_holding_anything() {
        let wire: Vec<i64> = (0..10).map(|k| k * D).collect();
        let (out, _) = run(Some(0), &wire, false);
        assert_eq!(out.len(), wire.len());
        assert!(out.iter().all(|s| s.dts == s.pts));
    }

    #[test]
    fn steady_state_adds_no_latency_after_the_first_depth_frames() {
        let wire = pyramid_wire(40);
        let mut d = DtsDeriver::new(Some(2));
        let mut out = Vec::new();
        let mut exceeded = Vec::new();
        let mut emitted_per_feed = Vec::new();
        for &p in &wire {
            let before = out.len();
            d.feed(sample(p), Some(D), &mut out, &mut exceeded);
            emitted_per_feed.push(out.len() - before);
        }
        assert_eq!(&emitted_per_feed[..3], &[0, 0, 3]);
        assert!(emitted_per_feed[3..].iter().all(|&n| n == 1));
    }

    #[test]
    fn observed_depth_matches_the_declared_one_after_the_probe() {
        let wire = pyramid_wire(40);
        let (declared, _) = run(Some(2), &wire, true);
        let (probed, exceeded) = run(None, &wire, true);
        assert!(exceeded.is_empty());
        assert_eq!(dts_of(&probed), dts_of(&declared));
    }

    #[test]
    fn probe_holds_at_most_the_probe_window() {
        let wire: Vec<i64> = (0..30).map(|k| k * D).collect();
        let mut d = DtsDeriver::new(None);
        let mut out = Vec::new();
        let mut exceeded = Vec::new();
        for (i, &p) in wire.iter().enumerate() {
            d.feed(sample(p), Some(D), &mut out, &mut exceeded);
            let pending = i + 1 - out.len();
            assert!(pending < 8, "held {pending} after {} feeds", i + 1);
        }
        assert!(out.iter().all(|s| s.dts == s.pts));
    }

    #[test]
    fn a_lost_frame_keeps_both_invariants() {
        let wire = pyramid_wire(40);
        for lost in 0..wire.len() {
            let mut w = wire.clone();
            w.remove(lost);
            let (out, exceeded) = run(Some(2), &w, true);
            assert!(exceeded.is_empty(), "loss at {lost}");
            assert_eq!(out.len(), w.len());
            assert_invariants(&out);
        }
    }

    #[test]
    fn an_understated_depth_is_reported_and_dts_stays_monotonic() {
        let wire = pyramid_wire(40);
        let (out, exceeded) = run(Some(1), &wire, true);
        assert!(!exceeded.is_empty());
        for w in out.windows(2) {
            assert!(w[0].dts < w[1].dts);
        }
        let tail = &out[out.len() - 10..];
        assert!(tail.iter().all(|s| s.dts <= s.pts), "recovers after growth");
    }

    #[test]
    fn finish_with_fewer_frames_than_the_depth_still_orders_them() {
        let wire = pyramid_wire(2);
        let (out, _) = run(Some(2), &wire, true);
        assert_eq!(out.len(), 2);
        assert_invariants(&out);
    }

    #[test]
    fn a_new_run_after_reset_restarts_the_grid() {
        let wire = pyramid_wire(10);
        let mut d = DtsDeriver::new(Some(2));
        let mut out = Vec::new();
        let mut exceeded = Vec::new();
        for &p in &wire {
            d.feed(sample(p), Some(D), &mut out, &mut exceeded);
        }
        d.finish(Some(D), &mut out, &mut exceeded);
        out.clear();
        let shifted: Vec<i64> = wire.iter().map(|p| p + 1_000_000).collect();
        for &p in &shifted {
            d.feed(sample(p), Some(D), &mut out, &mut exceeded);
        }
        assert!(exceeded.is_empty());
        assert_invariants(&out);
        assert_eq!(out[0].dts, Some(1_000_000 - 2 * D));
    }
}
