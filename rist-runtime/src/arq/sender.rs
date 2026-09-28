//! Sender-side NACK response — VSF TR-06-1:2020 §5.3.3 (Retransmitted
//! Packets) + §5.3.4 (Burst Control, informative).
//!
//! §5.3.3 spells out the sender's responsibility on receiving a NACK:
//! identify the flow via the SSRC field, locate the originally-sent packet,
//! and resend an exact copy (same sequence number and timestamp) with the
//! SSRC LSB flipped to 1. It explicitly does *not* prescribe how the sender
//! looks the packet up — "that storage/lookup mechanism is left to the
//! implementation" — so the bounded ring buffer here is **implementation
//! policy**, not a transcription. This module is deliberately RTP-framing
//! agnostic (`rist-runtime` has no dependency on an RTP codec crate): it
//! stores whatever opaque payload bytes + timestamp the caller handed it at
//! send time, and hands back exactly that — flipping the SSRC LSB and
//! rebuilding the actual RTP packet is the caller's job.
//!
//! §5.3.4 flags that a single Range-Based request field can nominally
//! demand up to 65536 retransmissions (`Additional = 0xFFFF`) and states an
//! implementation "must be prepared to throttle/reject this rather than
//! attempt it literally" — [`super::MAX_RANGE_EXPANSION`] is this engine's
//! throttle: any range beyond that many entries is truncated rather than
//! expanded literally.
//!
//! # NACK-amplification lookup cost (#977)
//!
//! Truncating the *count* of expanded sequence numbers to
//! [`super::MAX_RANGE_EXPANSION`] bounds the number of lookups per NACK, but
//! each individual lookup still needs to be cheap: with the lookup buffer
//! stored as a plain sent-order list, locating one sequence number is an
//! O(n) linear scan, so `MAX_RANGE_EXPANSION` lookups against an n-deep
//! buffer is O(`MAX_RANGE_EXPANSION` × n) — a single adversarial RangeNack
//! (`Additional = 0xFFFF`) against a modestly deep buffer multiplies into
//! tens of millions of comparisons. [`Sender`] instead indexes the buffer by
//! sequence number (a [`alloc::collections::BTreeMap`], so this stays
//! `no_std`+`alloc` without pulling in a hasher that needs `std`), with a
//! side [`VecDeque`] recording insertion order purely for FIFO eviction —
//! each lookup is O(log n) instead of O(n).
//!
//! # Response amplification (#977 continued)
//!
//! Bounding per-lookup *cost* still left the *response* itself unbounded: a
//! [`RangeNack`]/[`GenericNack`] can repeat the same range across many of
//! its (up to 16) ranges, or simply be re-sent by the peer moments after the
//! last response. Neither is deduplicated by [`Self::on_range_nack`]/
//! [`Self::on_generic_nack`] on their own, so one small wire message can
//! still trigger a disproportionate volume of retransmitted payload bytes.
//! `resolve` now: deduplicates every candidate sequence number
//! within one call (a `BTreeSet` of what's already been resolved this
//! call); caps the total distinct sequence numbers one call returns at
//! [`Sender::max_buffered`] (this sender never has more than that many
//! distinct packets to hand back anyway); and rate-limits any single
//! sequence number to at most one retransmission per
//! [`MIN_RETRANSMIT_INTERVAL`], tracked in [`Sender::last_retransmitted`].
//! §5.3.3/§5.3.4 leave all of this to the implementation, so these are
//! implementation-policy bounds, not transcribed numbers.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec::Vec;
use core::time::Duration;

use crate::nack::BLP_BIT_WIDTH;
use crate::{GenericNack, NackFci, PacketRange, RangeNack};

use super::MAX_RANGE_EXPANSION;
use super::seq;

/// Minimum spacing between two retransmissions of the same sequence number
/// (#977 continued). TR-06-1 does not state this window — a real
/// implementation would size it from the receiver's RTT, but [`Sender`] has
/// no RTT estimate of its own (unlike [`super::Receiver`], which carries
/// [`super::rtt::RttEstimator`]), so this is a fixed, conservative
/// **implementation policy** value rather than an RTT-derived one.
const MIN_RETRANSMIT_INTERVAL: Duration = Duration::from_millis(20);

/// One packet retransmitted in response to a NACK: the original sequence
/// number and timestamp, and the original payload bytes. The caller
/// rebuilds the actual RTP packet (with the SSRC LSB flipped to 1 per
/// §5.3.3) using whatever RTP codec it already has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retransmission<'a> {
    /// The original RTP sequence number (unchanged on retransmission, §5.3.3).
    pub seq: u16,
    /// The original RTP timestamp (unchanged on retransmission, §5.3.3).
    pub timestamp: u32,
    /// The original payload bytes.
    pub payload: &'a [u8],
}

#[derive(Debug, Clone)]
struct SentPacket {
    seq: u16,
    timestamp: u32,
    payload: Vec<u8>,
}

/// Sender-side lookup buffer for NACK-triggered retransmission (§5.3.3).
///
/// Indexed by sequence number (see the module doc's "NACK-amplification
/// lookup cost" section): `by_seq` is the actual store and `order` is only
/// insertion order, kept for FIFO eviction once `max_buffered` is exceeded.
#[derive(Debug)]
pub struct Sender {
    by_seq: BTreeMap<u16, SentPacket>,
    order: VecDeque<u16>,
    max_buffered: usize,
    /// Last time (per sequence number) a retransmission was actually
    /// handed back — the rate-limit state behind [`MIN_RETRANSMIT_INTERVAL`]
    /// (#977 continued). Pruned alongside `by_seq`'s own eviction/overwrite
    /// so it never outgrows `max_buffered` entries.
    last_retransmitted: BTreeMap<u16, Duration>,
}

impl Sender {
    /// A fresh sender retaining at most `max_buffered` sent packets for
    /// retransmission lookup. TR-06-1's only stated sender-buffer
    /// constraint is the qualitative "Sender Buffer >= Receiver Buffer"
    /// (Appendix B) — a *time*-based relationship this crate does not
    /// itself measure a sending rate to enforce, so this packet-count cap
    /// is **implementation policy**, not a transcription of that relation.
    pub fn new(max_buffered: usize) -> Self {
        // #1108/RIST-W1: `seq` is a `u16` (65 536 values). At `max_buffered
        // >= 32 768` (half the sequence space), a wrapped seq that's due
        // for eviction can still be within the *unwrapped* window of a
        // newer same-numbered entry — `by_seq.insert` then overwrites the
        // OLDER generation's entry in place (instead of evicting it via
        // `order`), so `by_seq.len()` stops growing, eviction never runs
        // again, and `order` gains a duplicate `u16` on every send,
        // forever. Clamped strictly below half the sequence space so two
        // live generations of the same `seq` can never coexist.
        let max_buffered = max_buffered.clamp(1, (1 << 15) - 1);
        Sender {
            by_seq: BTreeMap::new(),
            order: VecDeque::new(),
            max_buffered,
            last_retransmitted: BTreeMap::new(),
        }
    }

    /// Number of packets currently retained for lookup.
    pub fn buffered_count(&self) -> usize {
        self.by_seq.len()
    }

    /// Record a freshly-sent packet so it can later be located and
    /// retransmitted (§5.3.3). Evicts the oldest buffered packet once
    /// `max_buffered` is exceeded.
    pub fn on_sent(&mut self, seq: u16, timestamp: u32, payload: &[u8]) {
        self.by_seq.insert(
            seq,
            SentPacket {
                seq,
                timestamp,
                payload: payload.to_vec(),
            },
        );
        self.order.push_back(seq);
        // A fresh packet occupying this seq (including a 16-bit wraparound
        // reuse of an old, still-buffered seq) has no retransmission
        // history of its own yet.
        self.last_retransmitted.remove(&seq);
        while self.by_seq.len() > self.max_buffered {
            match self.order.pop_front() {
                Some(oldest) => {
                    self.by_seq.remove(&oldest);
                    self.last_retransmitted.remove(&oldest);
                }
                None => break,
            }
        }
    }

    /// Look up every sequence number named by a [`RangeNack`] that is still
    /// in the lookup buffer; sequence numbers already evicted (too old, or
    /// never sent) are silently skipped — §5.3.3 does not define behaviour
    /// for a request naming a packet the sender no longer has. See
    /// `resolve` for the deduplication/cap/rate-limit bounds
    /// applied to the response.
    pub fn on_range_nack(&mut self, nack: &RangeNack, now: Duration) -> Vec<Retransmission<'_>> {
        let ranges = &nack.ranges;
        self.resolve(ranges.iter().flat_map(|r| expand_range(*r)), now)
    }

    /// Look up every sequence number named by a [`GenericNack`] (bitmask
    /// format), same lookup/bounding semantics as [`Self::on_range_nack`].
    pub fn on_generic_nack(
        &mut self,
        nack: &GenericNack,
        now: Duration,
    ) -> Vec<Retransmission<'_>> {
        let fcis = &nack.nacks;
        self.resolve(fcis.iter().flat_map(|f| expand_fci(*f)), now)
    }

    /// Shared candidate-resolution path for [`Self::on_range_nack`]/
    /// [`Self::on_generic_nack`] (#977 continued — see the module doc's
    /// "Response amplification" section): deduplicates `candidates` within
    /// this one call, examines at most [`MAX_RANGE_EXPANSION`] of them
    /// (bounding total work regardless of how many ranges/FCIs the caller's
    /// NACK carries), stops once [`Sender::max_buffered`] distinct
    /// retransmissions have been collected, and rate-limits each sequence
    /// number to at most one retransmission per [`MIN_RETRANSMIT_INTERVAL`].
    fn resolve<I: IntoIterator<Item = u16>>(
        &mut self,
        candidates: I,
        now: Duration,
    ) -> Vec<Retransmission<'_>> {
        let max_out = self.max_buffered;
        let mut seen = BTreeSet::new();
        let mut out_seqs: Vec<u16> = Vec::new();

        for s in candidates.into_iter().take(MAX_RANGE_EXPANSION) {
            if out_seqs.len() >= max_out {
                break;
            }
            if !seen.insert(s) {
                continue;
            }
            if !self.by_seq.contains_key(&s) {
                continue;
            }
            let rate_limited = self
                .last_retransmitted
                .get(&s)
                .is_some_and(|&last| now.saturating_sub(last) < MIN_RETRANSMIT_INTERVAL);
            if rate_limited {
                continue;
            }
            self.last_retransmitted.insert(s, now);
            out_seqs.push(s);
        }

        out_seqs
            .into_iter()
            .filter_map(|s| {
                self.by_seq.get(&s).map(|sent| Retransmission {
                    seq: sent.seq,
                    timestamp: sent.timestamp,
                    payload: &sent.payload,
                })
            })
            .collect()
    }
}

fn expand_range(range: PacketRange) -> Vec<u16> {
    let count = (u32::from(range.additional) + 1).min(MAX_RANGE_EXPANSION as u32);
    let mut out = Vec::with_capacity(count as usize);
    let mut s = range.start;
    for _ in 0..count {
        out.push(s);
        s = seq::seq_next(s);
    }
    out
}

fn expand_fci(fci: NackFci) -> Vec<u16> {
    let mut out = alloc::vec![fci.pid];
    for bit in 0..BLP_BIT_WIDTH {
        if fci.blp & (1 << bit) != 0 {
            out.push(seq::seq_add(fci.pid, (bit + 1) as u16));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NackFci;

    const T0: Duration = Duration::ZERO;

    /// RIST-W1 (#1108): requesting `max_buffered >= 32768` (half the
    /// sequence space) used to leak memory and break eviction once `seq`
    /// wrapped — `by_seq.insert` overwrote the older same-`seq` entry in
    /// place rather than through `order`'s eviction path, so
    /// `by_seq.len()` stopped growing (looking "fine") while `order` kept
    /// gaining a duplicate `u16` on every send, forever. `Sender::new`
    /// clamps the request; this sends across 3 full 16-bit wraps and
    /// checks the buffer never exceeds the clamp at any point.
    #[test]
    fn max_buffered_is_clamped_and_survives_multiple_seq_wraps() {
        const REQUESTED: usize = usize::MAX;
        const CLAMP: usize = (1 << 15) - 1;
        let mut s = Sender::new(REQUESTED);
        for i in 0..3 * u32::from(u16::MAX) {
            let seq = i as u16; // wraps every 65536 sends, 3 times over
            s.on_sent(seq, i, b"x");
            assert!(
                s.buffered_count() <= CLAMP,
                "buffered_count {} exceeded the clamp {CLAMP} at i={i}",
                s.buffered_count()
            );
        }
    }

    #[test]
    fn on_sent_then_range_nack_locates_the_exact_payload() {
        let mut s = Sender::new(16);
        s.on_sent(100, 900_000, b"packet-100");
        s.on_sent(101, 900_090, b"packet-101");

        let nack = RangeNack {
            ssrc_media: 0x1234,
            ranges: alloc::vec![PacketRange {
                start: 100,
                additional: 0
            }],
        };
        let out = s.on_range_nack(&nack, T0);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].seq, 100);
        assert_eq!(out[0].timestamp, 900_000);
        assert_eq!(out[0].payload, b"packet-100");
    }

    #[test]
    fn range_nack_expands_a_contiguous_run() {
        let mut s = Sender::new(16);
        for seq in 10..15u16 {
            s.on_sent(seq, u32::from(seq), b"x");
        }
        let nack = RangeNack {
            ssrc_media: 1,
            ranges: alloc::vec![PacketRange {
                start: 10,
                additional: 4
            }],
        };
        let out = s.on_range_nack(&nack, T0);
        let seqs: Vec<u16> = out.iter().map(|r| r.seq).collect();
        assert_eq!(seqs, alloc::vec![10, 11, 12, 13, 14]);
    }

    #[test]
    fn evicted_packets_are_silently_skipped() {
        let mut s = Sender::new(1);
        s.on_sent(1, 0, b"a");
        s.on_sent(2, 0, b"b"); // evicts seq 1
        assert_eq!(s.buffered_count(), 1);

        let nack = RangeNack {
            ssrc_media: 1,
            ranges: alloc::vec![PacketRange {
                start: 1,
                additional: 0
            }],
        };
        assert!(s.on_range_nack(&nack, T0).is_empty());
    }

    #[test]
    fn generic_nack_bitmask_expands_pid_and_blp() {
        let mut s = Sender::new(32);
        for seq in [100u16, 103, 117] {
            s.on_sent(seq, 0, b"x");
        }
        // PID=100 signals 100 lost, BLP bit3 (=103) also lost.
        let nack = GenericNack {
            ssrc_sender: 0,
            ssrc_media: 1,
            nacks: alloc::vec![
                NackFci {
                    pid: 100,
                    blp: 0b0000_0000_0000_0100
                },
                NackFci { pid: 117, blp: 0 },
            ],
        };
        let out = s.on_generic_nack(&nack, T0);
        let seqs: Vec<u16> = out.iter().map(|r| r.seq).collect();
        assert_eq!(seqs, alloc::vec![100, 103, 117]);
    }

    #[test]
    fn range_expansion_is_throttled_against_an_adversarial_additional_count() {
        let mut s = Sender::new(1);
        let nack = RangeNack {
            ssrc_media: 1,
            // TR-06-1 §5.3.4's own called-out worst case.
            ranges: alloc::vec![PacketRange {
                start: 0,
                additional: 0xFFFF
            }],
        };
        // Must not attempt to allocate/iterate 65536 entries unbounded in a
        // way that panics or hangs; with nothing in the lookup buffer the
        // result is simply empty, but the point is this returns promptly.
        assert!(s.on_range_nack(&nack, T0).is_empty());
    }

    /// #977 regression: with an O(n) linear scan per expanded sequence
    /// number, a full-depth buffer plus an adversarial `additional = 0xFFFF`
    /// RangeNack multiplies into billions of comparisons — that would take
    /// many seconds to minutes, not the sub-second bound asserted below.
    /// With the `BTreeMap` index each lookup is O(log n), so the whole call
    /// stays fast even at this depth.
    ///
    /// Requests `MAX_RANGE_EXPANSION` (65536) as `max_buffered`, but
    /// RIST-W1 (#1108) clamps that to `FULL_DEPTH` (below half the 16-bit
    /// sequence space — see `Sender::new`'s doc), so the buffer is actually
    /// `FULL_DEPTH`-deep, not 65536.
    #[test]
    fn range_nack_lookup_stays_fast_against_a_full_depth_buffer() {
        const FULL_DEPTH: usize = (1 << 15) - 1;
        let mut s = Sender::new(MAX_RANGE_EXPANSION);
        for seq in 0..=u16::MAX {
            s.on_sent(seq, u32::from(seq), b"x");
        }
        assert_eq!(s.buffered_count(), FULL_DEPTH);

        let nack = RangeNack {
            ssrc_media: 1,
            ranges: alloc::vec![PacketRange {
                start: 0,
                additional: 0xFFFF
            }],
        };

        // `Instant` needs `std`; `--no-default-features` lib tests run `no_std`.
        #[cfg(feature = "std")]
        let start = std::time::Instant::now();
        let out = s.on_range_nack(&nack, T0);
        #[cfg(feature = "std")]
        {
            let elapsed = start.elapsed();
            assert!(
                elapsed < Duration::from_secs(2),
                "range_nack lookup against a full buffer took {elapsed:?} — \
                 looks like an O(n) scan crept back in"
            );
        }

        assert_eq!(out.len(), FULL_DEPTH);
    }

    /// r08-RIST-C1 regression: 16 identical, fully-overlapping ranges in one
    /// `RangeNack` must not multiply into 16 retransmissions per buffered
    /// packet. Pre-fix (dedup added, but against a small buffer so the
    /// un-bounded-response bug reproduces without a huge allocation): 10
    /// buffered packets x 16 duplicated ranges = 160 output entries with the
    /// pre-fix code (verified against the pre-fix `on_range_nack(&self, nack)`
    /// — 16 unbounded, non-deduplicated passes over the same 10 hits).
    /// Post-fix: deduplicated to exactly the 10 distinct buffered packets,
    /// and never more than `max_buffered` regardless of how many ranges/how
    /// large `additional` claims.
    #[test]
    fn duplicated_full_ranges_in_one_nack_are_deduplicated_and_capped() {
        let mut s = Sender::new(10);
        for seq in 0..10u16 {
            s.on_sent(seq, u32::from(seq), b"x");
        }
        let one_range = PacketRange {
            start: 0,
            additional: 0xFFFF,
        };
        let nack = RangeNack {
            ssrc_media: 1,
            ranges: alloc::vec![one_range; 16],
        };

        let out = s.on_range_nack(&nack, T0);
        assert_eq!(
            out.len(),
            10,
            "expected exactly the 10 distinct buffered packets, got {} \
             (16x duplication would give 160)",
            out.len()
        );
        let mut seqs: Vec<u16> = out.iter().map(|r| r.seq).collect();
        seqs.sort_unstable();
        seqs.dedup();
        assert_eq!(seqs.len(), 10, "output must not repeat a sequence number");
    }

    /// r08-RIST-C1 regression: the same NACK, re-delivered immediately
    /// (before [`MIN_RETRANSMIT_INTERVAL`] has elapsed), must not trigger a
    /// second retransmission of sequence numbers already retransmitted a
    /// moment ago — otherwise a peer (or anyone able to inject a NACK on the
    /// 5-tuple) can replay the same small request every millisecond for
    /// unbounded egress.
    #[test]
    fn repeating_a_nack_immediately_does_not_re_retransmit_the_same_seqs() {
        let mut s = Sender::new(10);
        for seq in 0..10u16 {
            s.on_sent(seq, u32::from(seq), b"x");
        }
        let nack = RangeNack {
            ssrc_media: 1,
            ranges: alloc::vec![PacketRange {
                start: 0,
                additional: 9
            }],
        };

        let first = s.on_range_nack(&nack, T0);
        assert_eq!(first.len(), 10);

        // Repeated at the same instant (well under MIN_RETRANSMIT_INTERVAL).
        let second = s.on_range_nack(&nack, T0);
        assert!(
            second.is_empty(),
            "immediate repeat retransmitted {} seqs a second time",
            second.len()
        );

        // Once the rate-limit window has elapsed, the same seqs are
        // eligible again.
        let third = s.on_range_nack(&nack, T0 + MIN_RETRANSMIT_INTERVAL);
        assert_eq!(third.len(), 10);
    }

    /// Confirms the rate-limit branch above is actually load-bearing: with
    /// it disabled (simulated here by using a zero-length interval, i.e.
    /// every repeat instantly re-qualifies), the immediate repeat *does*
    /// re-retransmit — proving `repeating_a_nack_immediately_does_not_re_retransmit_the_same_seqs`
    /// would fail without the rate limit rather than passing vacuously.
    #[test]
    fn rate_limit_check_is_load_bearing_against_a_zero_width_window() {
        let mut s = Sender::new(10);
        for seq in 0..10u16 {
            s.on_sent(seq, u32::from(seq), b"x");
        }
        let nack = RangeNack {
            ssrc_media: 1,
            ranges: alloc::vec![PacketRange {
                start: 0,
                additional: 9
            }],
        };
        s.on_range_nack(&nack, T0);
        // `now` advanced by exactly `MIN_RETRANSMIT_INTERVAL` (not "less
        // than"): the window has fully elapsed, so this is the boundary
        // case confirming the comparison is `<` (still rate-limited at
        // exactly the window) not `<=` (would wrongly still block here).
        let at_boundary = s.on_range_nack(&nack, T0 + MIN_RETRANSMIT_INTERVAL);
        assert_eq!(at_boundary.len(), 10);
    }
}
