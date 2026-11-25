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
//!   `PROBE_AUS` (8) samples are held while `N` is measured as the largest
//!   count of earlier-decoded frames with a later presentation time; the
//!   spec's own inference (`MaxDpbFrames`, up to 16) is deliberately not used,
//!   because it would hold 16 frames for a stream that has no reorder.
//!
//! # Field-coded video
//!
//! `max_num_reorder_frames` counts frames, complementary field pairs and
//! non-paired fields (H.264 §E.2.1,
//! `private/specs-verified/mpeg/transmux/codec/h264-vui-bitstream-restriction.md`),
//! while each coded field of a PAFF stream is its own access unit with its own
//! RTP timestamp (RFC 6184 §5.1 conveys one timestamp per coded picture). The
//! derivation counts access units, so once a slice header with
//! `field_pic_flag` = 1 is seen
//! (`private/specs-verified/mpeg/transmux/codec/h264-slice-header.md`) a
//! declared `N` frames is widened to `2N + 1` access units: up to `N` units of
//! two fields each precede and follow any picture, plus its own sibling field
//! when the pair is displayed in the opposite order to its decoding.
//!
//! # Failure handling
//!
//! A frame whose presentation time is earlier than a decode timestamp already
//! issued means the assumed `N` was too small (a declared bound that the
//! stream violates, or reordering deeper than the probe saw). The sample keeps
//! a strictly increasing `DTS` (previous plus one tick, which exceeds its
//! `PTS`), `N` grows by one up to `MAX_REORDER_DEPTH` (16), and the event is
//! returned for the caller to report. Packet loss needs no special case: a
//! missing access unit is absent from the sorted set, so every rank shifts
//! together and both invariants still hold.

use alloc::vec::Vec;

use crate::ir::{Sample, SampleTiming};

pub(crate) use crate::rtp_stream::MAX_REORDER_DEPTH;

pub(crate) const PROBE_AUS: usize = 8;

const AUS_PER_FIELD_PAIR: usize = 2;
/// Two consecutive samples, for the arrival-step estimate.
const CONSECUTIVE_SAMPLE_PAIR: usize = 2;

fn field_coded_depth(frames: usize) -> usize {
    (frames * AUS_PER_FIELD_PAIR + 1).min(MAX_REORDER_DEPTH)
}

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
    field_coded: bool,
    /// The previous access unit's presentation time (wire order), and whether
    /// any step has gone backward. A stream whose presentation order is its
    /// wire order needs no decode-timeline reconstruction: its decode time is
    /// its presentation time, exactly as the batch path returns it.
    last_wire_pts: Option<i64>,
    saw_backward: bool,
    /// Whether the reorder depth came from the stream's own declaration rather
    /// than the start-of-run probe. A declared depth is honoured even for a
    /// stream that shows no reorder yet; a probed zero depth means the wire
    /// order is the presentation order.
    declared: bool,
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
            field_coded: false,
            last_wire_pts: None,
            saw_backward: false,
            declared: declared.is_some(),
        }
    }

    pub(crate) fn declare_field_coded(&mut self) {
        if self.field_coded {
            return;
        }
        self.field_coded = true;
        if let Some(depth) = self.depth {
            self.depth = Some(field_coded_depth(depth));
        }
    }

    pub(crate) fn set_declared(&mut self, declared: Option<usize>) {
        if self.seen == 0 && self.held.is_empty() {
            self.declared = declared.is_some();
            self.depth = declared.map(|n| {
                let n = n.min(MAX_REORDER_DEPTH);
                if self.field_coded {
                    field_coded_depth(n)
                } else {
                    n
                }
            });
        }
    }

    pub(crate) fn feed(
        &mut self,
        sample: Sample,
        period: Option<i64>,
        out: &mut Vec<Sample>,
        exceeded: &mut Vec<DepthExceeded>,
    ) {
        let Some(SampleTiming::Timed { .. }) = sample.timing else {
            out.push(sample);
            return;
        };
        let pts = sample.timing.and_then(|t| t.pts()).unwrap_or_default();
        if self.last_wire_pts.is_some_and(|prev| pts < prev) {
            self.saw_backward = true;
        }
        self.last_wire_pts = Some(pts);
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
        self.last_wire_pts = None;
        self.saw_backward = false;
    }

    /// A stream with no backward step whose depth was probed as zero presents
    /// in decode order: its decode time is its presentation time, exactly as
    /// the batch path returns it.
    fn presentation_order_is_wire_order(&self) -> bool {
        !self.saw_backward && !self.declared && self.depth == Some(0)
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
            let pts = s.timing.and_then(|t| t.pts()).unwrap_or_default();
            let ahead = probe[..i]
                .iter()
                .filter(|e| e.timing.and_then(|t| t.pts()).unwrap_or_default() > pts)
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
        let pts = sample.timing.and_then(|t| t.pts()).unwrap_or_default();
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
        if self.presentation_order_is_wire_order() {
            let dts = self.window.remove(0);
            self.last_dts = Some(dts);
            if let Some(t) = sample.timing.as_mut() {
                *t = t.with_composition(dts, pts);
            }
            out.push(sample);
            return;
        }
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
        if let Some(t) = sample.timing.as_mut() {
            *t = t.with_composition(dts, pts);
        }
        out.push(sample);
    }

    fn anchor(&mut self, period: Option<i64>, pop_last: bool, out: &mut Vec<Sample>) {
        let depth = self.depth.unwrap_or(0);
        let origin = self.window.first().copied().unwrap_or_default();
        let step = period
            .filter(|&p| p > 0)
            .or_else(|| {
                self.window
                    .windows(CONSECUTIVE_SAMPLE_PAIR)
                    .map(|w| w[1] - w[0])
                    .filter(|&g| g > 0)
                    .min()
            })
            .or_else(|| {
                self.held
                    .iter()
                    .find_map(|s| s.timing.and_then(|t| t.duration()).map(i64::from))
                    .filter(|&d| d > 0)
            })
            .unwrap_or(1);
        let held = core::mem::take(&mut self.held);
        let last = held.len().saturating_sub(1);
        if self.presentation_order_is_wire_order() {
            for mut s in held {
                let dts = self.window.remove(0);
                let pts = s.timing.and_then(|t| t.pts()).unwrap_or(dts);
                self.last_dts = Some(dts);
                if let Some(t) = s.timing.as_mut() {
                    *t = t.with_composition(dts, pts);
                }
                out.push(s);
            }
            self.anchored = true;
            if !pop_last {
                self.window.clear();
            }
            return;
        }
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
            let pts = s.timing.and_then(|t| t.pts()).unwrap_or(dts);
            if let Some(t) = s.timing.as_mut() {
                *t = t.with_composition(dts, pts);
            }
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
            .expect("valid timing")
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

    /// The `dts` every sample the deriver emitted carries; a deriver only
    /// ever emits samples it was fed (which always have a `pts`), so any other
    /// shape is a test failure, not a value to paper over.
    fn dts_of(out: &[Sample]) -> Vec<i64> {
        assert!(
            out.iter().all(|s| s.timing.and_then(|t| t.dts()).is_some()),
            "every emitted sample carries a dts"
        );
        out.iter()
            .filter_map(|s| s.timing.and_then(|t| t.dts()))
            .collect()
    }

    fn assert_invariants(out: &[Sample]) {
        for w in out.windows(2) {
            assert!(
                w[0].timing.and_then(|t| t.dts()) < w[1].timing.and_then(|t| t.dts()),
                "dts not strictly increasing"
            );
        }
        for s in out {
            assert!(
                s.timing.and_then(|t| t.dts()).unwrap_or_default()
                    <= s.timing.and_then(|t| t.pts()).unwrap_or_default(),
                "dts {:?} > pts {:?}",
                s.timing.and_then(|t| t.dts()),
                s.timing.and_then(|t| t.pts())
            );
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
            assert_eq!(
                s.timing.and_then(|t| t.dts()),
                Some((k as i64 - 2) * D),
                "frame {k}"
            );
            assert_eq!(
                s.timing.and_then(|t| t.pts()),
                Some(wire[k]),
                "pts untouched, order untouched"
            );
        }
    }

    #[test]
    fn depth_zero_leaves_dts_equal_to_pts_without_holding_anything() {
        let wire: Vec<i64> = (0..10).map(|k| k * D).collect();
        let (out, _) = run(Some(0), &wire, false);
        assert_eq!(out.len(), wire.len());
        assert!(
            out.iter()
                .all(|s| s.timing.and_then(|t| t.dts()) == s.timing.and_then(|t| t.pts()))
        );
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
        assert!(
            out.iter()
                .all(|s| s.timing.and_then(|t| t.dts()) == s.timing.and_then(|t| t.pts()))
        );
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
            assert!(w[0].timing.and_then(|t| t.dts()) < w[1].timing.and_then(|t| t.dts()));
        }
        let tail = &out[out.len() - 10..];
        assert!(
            tail.iter()
                .all(|s| s.timing.and_then(|t| t.dts()).unwrap_or_default()
                    <= s.timing.and_then(|t| t.pts()).unwrap_or_default()),
            "recovers after growth"
        );
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
        assert_eq!(out[0].timing.and_then(|t| t.dts()), Some(1_000_000 - 2 * D));
    }

    #[test]
    fn field_coded_stream_keeps_dts_at_or_below_pts_without_exceeding_depth() {
        let frame_order = [0i64, 2, 1, 4, 3, 6, 5, 8, 7, 10, 9, 12, 11];
        let wire: Vec<i64> = frame_order
            .iter()
            .flat_map(|f| [f * D, f * D + D / 2])
            .collect();
        let mut d = DtsDeriver::new(Some(1));
        d.declare_field_coded();
        let mut out = Vec::new();
        let mut exceeded = Vec::new();
        for &p in &wire {
            d.feed(sample(p), Some(D / 2), &mut out, &mut exceeded);
        }
        d.finish(Some(D / 2), &mut out, &mut exceeded);
        assert_eq!(out.len(), wire.len());
        assert!(exceeded.is_empty());
        assert_invariants(&out);
    }

    fn sample_with(pts: i64, duration: u32) -> Sample {
        Sample::new(
            alloc::vec![0u8; 4],
            Some(pts),
            Some(pts),
            Some(duration),
            true,
        )
        .expect("valid timing")
    }

    #[test]
    fn field_coded_depth_is_two_per_frame_plus_one_capped() {
        assert_eq!(field_coded_depth(0), 1);
        assert_eq!(field_coded_depth(1), 3);
        assert_eq!(field_coded_depth(7), 15);
        assert_eq!(field_coded_depth(8), 16);

        let mut d = DtsDeriver::new(Some(1));
        d.declare_field_coded();
        assert_eq!(
            d.depth,
            Some(3),
            "one reorder frame is a field pair plus one"
        );
    }

    #[test]
    fn set_declared_applies_only_before_the_first_sample() {
        let mut d = DtsDeriver::new(None);
        d.set_declared(Some(2));
        assert_eq!(d.depth, Some(2), "a declaration before any sample wins");

        let mut d = DtsDeriver::new(Some(0));
        let mut out = Vec::new();
        let mut exceeded = Vec::new();
        d.feed(sample(0), Some(D), &mut out, &mut exceeded);
        d.set_declared(Some(5));
        assert_eq!(
            d.depth,
            Some(0),
            "a declaration after the first sample is ignored"
        );
    }

    #[test]
    fn probe_reports_the_observed_depth_not_duplicate_pts() {
        let wire = [0i64, 4000, 2000, 1000, 1000, 3000, 5000, 6000];
        let mut d = DtsDeriver::new(None);
        let mut out = Vec::new();
        let mut exceeded = Vec::new();
        for &p in &wire {
            d.feed(sample(p), Some(D), &mut out, &mut exceeded);
        }
        let dts = dts_of(&out);
        assert_eq!(dts[0], -2 * D, "the probe sees a reorder depth of two");
    }

    #[test]
    fn depth_growth_is_one_step_per_inversion() {
        let mut d = DtsDeriver::new(Some(0));
        let mut out = Vec::new();
        let mut exceeded = Vec::new();
        for &p in &[0i64, 0, 3000] {
            d.feed(sample(p), Some(D), &mut out, &mut exceeded);
        }
        assert_eq!(exceeded.len(), 2, "two inversions");
        assert_eq!(exceeded[0].depth, 1);
        assert_eq!(exceeded[1].depth, 2);
        let dts = dts_of(&out);
        assert!(
            dts.windows(2).all(|w| w[1] > w[0]),
            "dts stays strictly increasing: {dts:?}"
        );
    }

    #[test]
    fn depth_growth_stops_at_the_cap() {
        let mut d = DtsDeriver::new(Some(MAX_REORDER_DEPTH));
        d.anchored = true;
        d.depth = Some(MAX_REORDER_DEPTH);
        d.last_dts = Some(0);
        d.window = alloc::vec![50];
        let mut out = Vec::new();
        let mut exceeded = Vec::new();
        d.run(sample(0), Some(D), &mut out, &mut exceeded);
        assert_eq!(d.depth, Some(MAX_REORDER_DEPTH));
        assert_eq!(exceeded.len(), 1);
        assert_eq!(exceeded[0].depth, MAX_REORDER_DEPTH);
    }

    #[test]
    fn anchor_prefers_a_positive_period_then_the_smallest_window_gap() {
        let mut d = DtsDeriver::new(Some(2));
        d.depth = Some(2);
        d.window = alloc::vec![100, 200, 1000];
        d.held = alloc::vec![sample(100), sample(200), sample(1000)];
        let mut out = Vec::new();
        d.anchor(Some(0), false, &mut out);
        assert_eq!(dts_of(&out), alloc::vec![-100, 0, 100]);
    }

    #[test]
    fn anchor_ignores_a_zero_window_gap() {
        let mut d = DtsDeriver::new(Some(2));
        d.depth = Some(2);
        d.window = alloc::vec![100, 100, 200];
        d.held = alloc::vec![sample(100), sample(100), sample(200)];
        let mut out = Vec::new();
        d.anchor(Some(0), false, &mut out);
        assert_eq!(dts_of(&out), alloc::vec![-100, 0, 100]);
    }

    #[test]
    fn anchor_falls_back_to_the_first_positive_held_duration() {
        let mut d = DtsDeriver::new(Some(1));
        d.depth = Some(1);
        d.window = alloc::vec![100, 100];
        d.held = alloc::vec![sample_with(100, 0), sample_with(100, 40)];
        let mut out = Vec::new();
        d.anchor(None, false, &mut out);
        assert_eq!(dts_of(&out), alloc::vec![60, 100]);
    }

    #[test]
    fn anchor_applies_pop_last_only_to_the_last_held_sample() {
        let mut d = DtsDeriver::new(Some(1));
        d.depth = Some(1);
        d.window = alloc::vec![100, 200, 300];
        d.held = alloc::vec![sample(100), sample(200), sample(300)];
        let mut out = Vec::new();
        d.anchor(Some(100), true, &mut out);
        assert_eq!(dts_of(&out), alloc::vec![0, 100, 100]);
    }
}
