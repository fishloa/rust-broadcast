//! ARQ receiver-side state — VSF TR-06-1:2020 §5.3.1 (Reorder Section +
//! Retransmission Reassembly Section, loss detection), §5.3.2
//! (retransmission-request formats), §5.3.4 (burst control). Retry-timing
//! specifics are librist-sourced, not spec-stated — see the `arq` module
//! doc's Attribution section before reading [`ArqConfig`] fields as if they
//! were transcribed numbers.
//!
//! Sans-IO: [`Receiver`] never reads a wall clock — [`Receiver::feed`] and
//! [`Receiver::tick`] take a caller-supplied `now: core::time::Duration`.
//!
//! # The two-stage buffer (§5.3.1)
//!
//! TR-06-1 describes packets crossing a Reorder Section before entering a
//! Retransmission Reassembly Section, with loss detected "at the boundary
//! between these two sections." This engine implements that as: an arriving
//! packet that is exactly the next expected sequence number is delivered
//! immediately (§5.3.1's own "minimum-delay" alternative — TR-06-1 names
//! this as a valid, if noisier, implementation choice); an arriving packet
//! *ahead* of the next expected number opens a gap, and every sequence
//! number in that gap starts aging in the Reorder Section from that moment.
//! [`Receiver::tick`] promotes a gap into the Retransmission Reassembly
//! Section — i.e. treats it as a confirmed, NACK-eligible loss — once it has
//! aged [`ArqConfig::reorder_section`] without the missing packet arriving.
//! This mapping (time-based promotion per gap, rather than a literal
//! per-packet dwell buffer every packet passes through) is this crate's own
//! reading of §5.3.1's informative description, not itself spec-cited.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::time::Duration;

use crate::nack::{BLP_BIT_WIDTH, MAX_RANGE_ENTRIES};
use crate::{NackFci, PacketRange};

use super::ArqConfig;
use super::rtt::RttEstimator;
use super::seq;

/// Outcome of one [`Receiver::feed`] call.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct DeliveryOutcome {
    /// Sequence numbers that became cumulatively in-order-deliverable as a
    /// result of this packet's arrival — includes the fed sequence number
    /// itself when it was the next-expected packet, plus any
    /// previously-buffered out-of-order packets it consequently unblocked.
    pub delivered: Vec<u16>,
}

/// Outcome of one [`Receiver::tick`] call.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct TickOutcome {
    /// Sequence numbers newly promoted this tick from the Reorder Section
    /// into the Retransmission Reassembly Section (§5.3.1) — informational;
    /// no caller action is required beyond what `due` already asks for.
    pub promoted: Vec<u16>,
    /// Contiguous runs of sequence numbers due for a (re)transmission
    /// request this tick, coalesced and capped at TR-06-1 §5.3.2.2's
    /// 16-range-per-packet wire limit. Wrap directly into a
    /// [`crate::RangeNack`], or pass to [`super::ranges_to_fci`] for the
    /// bitmask format. A due range beyond the 16th is simply not requested
    /// this tick — it was never marked as sent, so it stays due and is
    /// reoffered on a later tick (**implementation policy**: TR-06-1 states
    /// the 16-range wire cap but not what an implementation should do with
    /// an oversupply of simultaneously-due ranges).
    pub due: Vec<PacketRange>,
    /// Sequence numbers that exhausted [`ArqConfig::max_retransmission_requests`]
    /// or aged out of [`ArqConfig::reassembly_budget`] (librist's `* 1.1`
    /// margin applied — see the `arq` module doc) and have been given up
    /// on: permanently lost from this engine's point of view.
    pub given_up: Vec<u16>,
    /// Sequence numbers delivered as a side effect of `given_up` skipping
    /// past a permanently-lost packet that was blocking delivery.
    pub unblocked: Vec<u16>,
}

/// Per-sequence-number tracking state while a packet is missing.
#[derive(Debug, Clone, Copy)]
struct MissingState {
    /// Time this seq was first identified as missing — start of its
    /// Reorder Section dwell (§5.3.1).
    first_missing_at: Duration,
    /// Time this seq crossed into the Retransmission Reassembly Section
    /// (librist's `insertion_time` analogue); `None` while still aging in
    /// the Reorder Section.
    promoted_at: Option<Duration>,
    /// Number of retransmission requests already sent for this seq.
    requests_sent: u32,
    /// Absolute time the next (re)request is due; `None` until promoted.
    next_request_due: Option<Duration>,
}

/// ARQ receiver-side reliability engine (TR-06-1 §5.3). See the module doc
/// for the two-stage buffer model and the `arq` module doc for the
/// retry-timing Attribution.
#[derive(Debug)]
pub struct Receiver {
    config: ArqConfig,
    /// Cumulative delivery point: every seq strictly before this has either
    /// been delivered in order or given up on and skipped past. `None`
    /// until the first packet ever arrives.
    next_expected: Option<u16>,
    /// Received but not yet deliverable (a gap remains below it).
    received_ahead: BTreeSet<u16>,
    /// Sequence numbers currently believed missing, keyed by tracking state.
    missing: BTreeMap<u16, MissingState>,
    /// Highest sequence number ever received — bookkeeping to detect newly
    /// opened gaps, not itself a spec-named field.
    highest_received: Option<u16>,
    rtt: RttEstimator,
}

impl Receiver {
    /// A fresh receiver with no packets observed yet.
    pub fn new(config: ArqConfig) -> Self {
        Receiver {
            config,
            next_expected: None,
            received_ahead: BTreeSet::new(),
            missing: BTreeMap::new(),
            highest_received: None,
            rtt: RttEstimator::new(),
        }
    }

    /// The cumulative delivery point (every seq strictly before this has
    /// been delivered or given up on). `None` before the first packet.
    pub fn next_expected(&self) -> Option<u16> {
        self.next_expected
    }

    /// Number of sequence numbers currently tracked as missing (Reorder
    /// Section + Retransmission Reassembly Section combined).
    pub fn missing_count(&self) -> usize {
        self.missing.len()
    }

    /// The current smoothed RTT estimate, if any real sample has been
    /// folded in via [`Self::on_rtt_sample`] yet.
    pub fn rtt_estimate(&self) -> Option<Duration> {
        self.rtt.smoothed()
    }

    /// Fold a fresh RTT sample (e.g. from [`super::rtt::rtt_sample`]) into
    /// this receiver's smoothed RTT estimate, used to schedule
    /// retransmission-request retries (see the `arq` module doc's
    /// Attribution section).
    pub fn on_rtt_sample(&mut self, sample: Duration) {
        self.rtt.update(sample);
    }

    /// Process one arriving RTP data packet's sequence number.
    pub fn feed(&mut self, seq_number: u16, now: Duration) -> DeliveryOutcome {
        match self.highest_received {
            None => {
                self.highest_received = Some(seq_number);
                self.next_expected = Some(seq_number);
            }
            Some(highest) if seq::seq_gt(seq_number, highest) => {
                let mut s = seq::seq_next(highest);
                while s != seq_number {
                    self.missing.entry(s).or_insert(MissingState {
                        first_missing_at: now,
                        promoted_at: None,
                        requests_sent: 0,
                        next_request_due: None,
                    });
                    s = seq::seq_next(s);
                }
                self.highest_received = Some(seq_number);
            }
            _ => {}
        }

        // Arrived, whichever bucket it was tracked in (if any).
        self.missing.remove(&seq_number);

        let mut delivered = Vec::new();
        let next_expected = self.next_expected.unwrap_or(seq_number);
        if seq_number == next_expected {
            delivered.push(seq_number);
            let mut n = seq::seq_next(seq_number);
            while self.received_ahead.remove(&n) {
                delivered.push(n);
                n = seq::seq_next(n);
            }
            self.next_expected = Some(n);
        } else if seq::seq_gt(seq_number, next_expected) {
            self.received_ahead.insert(seq_number);
        }
        // seq_number before next_expected: a duplicate (e.g. a redundant
        // retransmission, or one arriving after its gap was already given
        // up on) — nothing further to do.

        DeliveryOutcome { delivered }
    }

    /// Advance to absolute time `now`: age Reorder-Section gaps into
    /// tracked losses, give up on anything that exhausted its retry budget
    /// or aged out of the reassembly budget, and report which sequence
    /// numbers are due a (re)transmission request now.
    pub fn tick(&mut self, now: Duration) -> TickOutcome {
        let mut promoted = Vec::new();
        for (&s, state) in self.missing.iter_mut() {
            if state.promoted_at.is_none()
                && elapsed(now, state.first_missing_at) >= self.config.reorder_section
            {
                state.promoted_at = Some(now);
                let delay = request_delay(self.rtt.smoothed(), &self.config, true);
                state.next_request_due = Some(now + delay);
                promoted.push(s);
            }
        }

        // Give up: age-out against the reassembly budget (librist's `* 1.1`
        // margin) checked first, retry-count exhaustion second — order
        // stated explicitly in the `arq` module doc's Attribution section;
        // functionally the two conditions are an unordered OR.
        let age_limit = scale_1_1(self.config.reassembly_budget());
        let given_up: Vec<u16> = self
            .missing
            .iter()
            .filter(|(_, s)| {
                s.promoted_at.is_some_and(|p| elapsed(now, p) >= age_limit)
                    || s.requests_sent >= self.config.max_retransmission_requests
            })
            .map(|(&s, _)| s)
            .collect();
        for s in &given_up {
            self.missing.remove(s);
        }

        // Skip delivery past whichever given-up sequence numbers are at the
        // front of the queue (a gap not at the front stays blocking until
        // its own turn — the buffer behaves as a FIFO, §5.3.1).
        let mut unblocked = Vec::new();
        let given_up_set: BTreeSet<u16> = given_up.iter().copied().collect();
        if let Some(mut ne) = self.next_expected {
            while given_up_set.contains(&ne) {
                ne = seq::seq_next(ne);
                while self.received_ahead.remove(&ne) {
                    unblocked.push(ne);
                    ne = seq::seq_next(ne);
                }
            }
            self.next_expected = Some(ne);
        }

        // Due-for-request: promoted, under the retry cap, and either never
        // requested or past its scheduled next-request time.
        let mut due_seqs: Vec<u16> = self
            .missing
            .iter()
            .filter(|(_, s)| {
                s.promoted_at.is_some()
                    && s.requests_sent < self.config.max_retransmission_requests
                    && s.next_request_due.is_some_and(|d| now >= d)
            })
            .map(|(&s, _)| s)
            .collect();
        due_seqs.sort_unstable();

        let due = cap_ranges(coalesce_ranges(&due_seqs), MAX_RANGE_ENTRIES);
        let requested_seqs = expand_ranges(&due);
        for s in &requested_seqs {
            if let Some(state) = self.missing.get_mut(s) {
                state.requests_sent += 1;
                // Always the "subsequent retry" (1.1x / no is_first
                // exception) schedule here: the 1.0x schedule is only ever
                // used once, at promotion time, to set up the *first*
                // request's due time (see the promotion loop above). This
                // call is scheduling the time *after* a request has just
                // been sent, which librist's `rist_process_nack` always
                // does at `rtt * 1.1` regardless of retry count.
                let delay = request_delay(self.rtt.smoothed(), &self.config, false);
                state.next_request_due = Some(now + delay);
            }
        }

        TickOutcome {
            promoted,
            due,
            given_up,
            unblocked,
        }
    }
}

/// `now - since`, clamped to zero rather than panicking on a non-monotonic
/// `now` (a caller bug, not a protocol condition this module needs to
/// reject) — mirrors `srt_runtime::arq::receiver`'s identical helper.
fn elapsed(now: Duration, since: Duration) -> Duration {
    now.checked_sub(since).unwrap_or(Duration::ZERO)
}

/// Scale a duration by librist's `* 1.1` margin, in whole microseconds to
/// avoid float drift.
fn scale_1_1(d: Duration) -> Duration {
    let us = d.as_micros().min(u128::from(u64::MAX)) as u64;
    Duration::from_micros(us.saturating_mul(11) / 10)
}

/// The delay before the *first* NACK for a newly-promoted loss
/// (`is_first = true`), or before the *next* retry — librist schedules the
/// first at `1.0 * rtt` and every subsequent retry at `1.1 * rtt`, clamped
/// to `[recovery_rtt_min, recovery_rtt_max]`; this falls back to TR-06-1
/// Appendix B's derived interval only until a real RTT sample exists. See
/// the `arq` module doc's Attribution section.
fn request_delay(rtt: Option<Duration>, cfg: &ArqConfig, is_first: bool) -> Duration {
    match rtt {
        Some(rtt) => {
            let clamped = rtt.clamp(cfg.recovery_rtt_min, cfg.recovery_rtt_max);
            if is_first {
                clamped
            } else {
                scale_1_1(clamped)
            }
        }
        None => cfg.fallback_retransmission_interval(),
    }
}

/// Coalesce a sorted, deduplicated run of sequence numbers (already
/// circularly increasing) into [`PacketRange`] entries (TR-06-1 §5.3.2.2) —
/// a maximal-contiguous-run encoding; the coalescing itself is not a spec
/// rule (mirrors `srt_runtime::arq::receiver::coalesce`'s NAK-side
/// analogue).
fn coalesce_ranges(seqs: &[u16]) -> Vec<PacketRange> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < seqs.len() {
        let start = seqs[i];
        let mut end = start;
        let mut j = i + 1;
        while j < seqs.len() && seqs[j] == seq::seq_next(end) {
            end = seqs[j];
            j += 1;
        }
        out.push(PacketRange {
            start,
            additional: end.wrapping_sub(start),
        });
        i = j;
    }
    out
}

/// Truncate a coalesced range list to at most `max` entries (TR-06-1
/// §5.3.2.2's per-packet cap). Entries beyond `max` are simply dropped from
/// *this* call's output — see [`TickOutcome::due`]'s doc for why that is
/// safe (they remain tracked as missing and are reoffered later).
fn cap_ranges(mut ranges: Vec<PacketRange>, max: usize) -> Vec<PacketRange> {
    ranges.truncate(max);
    ranges
}

/// Expand a coalesced range list back into individual sequence numbers —
/// used internally to update per-seq request bookkeeping after building
/// [`TickOutcome::due`]. Bounded by [`super::MAX_RANGE_EXPANSION`] as a
/// defensive cap (this crate's own output is already small, but the
/// function is not restricted to trusted input).
fn expand_ranges(ranges: &[PacketRange]) -> Vec<u16> {
    let mut out = Vec::new();
    for r in ranges {
        let count = (u32::from(r.additional) + 1).min(super::MAX_RANGE_EXPANSION as u32);
        let mut s = r.start;
        for _ in 0..count {
            out.push(s);
            s = seq::seq_next(s);
        }
    }
    out
}

/// Convert a coalesced list of [`PacketRange`]s (e.g. [`TickOutcome::due`])
/// into [`NackFci`] entries for the bitmask-based Generic NACK format
/// (TR-06-1 §5.3.2.1). Each FCI can name at most 17 consecutive lost
/// packets (the `PID` itself plus 16 `BLP` bits), so it packs as many
/// still-missing sequence numbers as fit within each 17-wide window before
/// starting a new FCI — reproducing TR-06-1 Appendix A's own bitmask worked
/// example exactly (see `tests/spec_vectors.rs`).
pub fn ranges_to_fci(ranges: &[PacketRange]) -> Vec<NackFci> {
    let seqs = expand_ranges(ranges);
    seqs_to_fci(&seqs)
}

/// Convert a sorted, deduplicated, circularly-increasing list of missing
/// sequence numbers directly into [`NackFci`] entries (TR-06-1 §5.3.2.1),
/// packing up to 17 consecutive positions per FCI. See [`ranges_to_fci`]'s
/// doc for the packing algorithm.
pub fn seqs_to_fci(missing_sorted: &[u16]) -> Vec<NackFci> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < missing_sorted.len() {
        let pid = missing_sorted[i];
        let mut blp: u16 = 0;
        let mut j = i + 1;
        while j < missing_sorted.len() {
            let diff = seq::seq_diff(missing_sorted[j], pid);
            if !(1..=(BLP_BIT_WIDTH as i32)).contains(&diff) {
                break;
            }
            blp |= 1u16 << (diff - 1);
            j += 1;
        }
        out.push(NackFci { pid, blp });
        i = j;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ArqConfig {
        ArqConfig::default()
    }

    #[test]
    fn in_order_arrivals_deliver_immediately_with_no_missing() {
        let mut r = Receiver::new(cfg());
        for s in 0..5u16 {
            let out = r.feed(s, Duration::ZERO);
            assert_eq!(out.delivered, alloc::vec![s]);
        }
        assert_eq!(r.next_expected(), Some(5));
        assert_eq!(r.missing_count(), 0);
    }

    #[test]
    fn a_gap_opens_a_missing_entry_but_does_not_deliver() {
        let mut r = Receiver::new(cfg());
        r.feed(0, Duration::ZERO);
        r.feed(1, Duration::ZERO);
        let out = r.feed(3, Duration::ZERO); // seq 2 missing
        assert!(out.delivered.is_empty());
        assert_eq!(r.missing_count(), 1);
        assert_eq!(r.next_expected(), Some(2)); // stalled at the gap

        // Filling the gap unblocks the buffered seq 3 too.
        let fill = r.feed(2, Duration::ZERO);
        assert_eq!(fill.delivered, alloc::vec![2, 3]);
        assert_eq!(r.next_expected(), Some(4));
        assert_eq!(r.missing_count(), 0);
    }

    #[test]
    fn a_gap_is_not_due_until_it_ages_past_reorder_section() {
        let mut r = Receiver::new(cfg());
        r.feed(0, Duration::ZERO);
        r.feed(2, Duration::ZERO); // seq 1 missing, opened at t=0

        let before = r.tick(Duration::from_millis(69));
        assert!(before.promoted.is_empty());
        assert!(before.due.is_empty());

        let after = r.tick(Duration::from_millis(70));
        assert_eq!(after.promoted, alloc::vec![1]);
        // No RTT sample yet -> Appendix B fallback interval, due
        // immediately at the moment of promotion (delay is added to
        // `now`, so the earliest it can fire is on a *later* tick).
        assert!(after.due.is_empty());

        let due_tick = r.tick(Duration::from_millis(70) + cfg().fallback_retransmission_interval());
        assert_eq!(
            due_tick.due,
            alloc::vec![PacketRange {
                start: 1,
                additional: 0
            }]
        );
    }

    #[test]
    fn rtt_driven_scheduling_uses_1x_then_1_1x_once_a_sample_exists() {
        let mut r = Receiver::new(cfg());
        r.on_rtt_sample(Duration::from_millis(20));
        r.feed(0, Duration::ZERO);
        r.feed(2, Duration::ZERO); // seq 1 missing at t=0

        let promote_at = Duration::from_millis(70);
        let promoted = r.tick(promote_at);
        assert_eq!(promoted.promoted, alloc::vec![1]);
        assert!(promoted.due.is_empty());

        // First request at promote_at + 1.0*rtt = 90ms.
        let too_early = r.tick(promote_at + Duration::from_millis(19));
        assert!(too_early.due.is_empty());
        let first = r.tick(promote_at + Duration::from_millis(20));
        assert_eq!(
            first.due,
            alloc::vec![PacketRange {
                start: 1,
                additional: 0
            }]
        );

        // Second request at 90ms + 1.1*20ms = 112ms.
        let too_early2 = r.tick(promote_at + Duration::from_millis(41));
        assert!(too_early2.due.is_empty());
        let second = r.tick(promote_at + Duration::from_millis(42));
        assert_eq!(
            second.due,
            alloc::vec![PacketRange {
                start: 1,
                additional: 0
            }]
        );
    }

    #[test]
    fn retry_cap_gives_up_after_max_retransmission_requests() {
        let mut small_cfg = cfg();
        small_cfg.max_retransmission_requests = 2;
        small_cfg.reorder_section = Duration::ZERO;
        let mut r = Receiver::new(small_cfg);
        r.on_rtt_sample(Duration::from_millis(10));
        r.feed(0, Duration::ZERO);
        r.feed(2, Duration::ZERO); // seq 1 missing

        let mut now = Duration::ZERO;
        r.tick(now); // promotes seq 1 immediately (reorder_section = 0)

        now += Duration::from_millis(10); // 1.0x
        let t1 = r.tick(now);
        assert_eq!(
            t1.due,
            alloc::vec![PacketRange {
                start: 1,
                additional: 0
            }]
        );

        now += scale_1_1(Duration::from_millis(10));
        let t2 = r.tick(now);
        assert_eq!(
            t2.due,
            alloc::vec![PacketRange {
                start: 1,
                additional: 0
            }]
        );

        // Retry cap (2) now reached — no third request, and the packet is
        // given up on, unblocking delivery.
        now += scale_1_1(Duration::from_millis(10));
        let t3 = r.tick(now);
        assert!(t3.due.is_empty());
        assert_eq!(t3.given_up, alloc::vec![1]);
        assert_eq!(r.next_expected(), Some(3));
    }

    #[test]
    fn age_out_gives_up_even_under_the_retry_cap() {
        let mut c = cfg();
        c.reorder_section = Duration::ZERO;
        c.receiver_buffer = Duration::from_millis(100);
        c.max_retransmission_requests = 100; // won't be hit
        let mut r = Receiver::new(c);
        r.feed(0, Duration::ZERO);
        r.feed(2, Duration::ZERO); // seq 1 missing

        r.tick(Duration::ZERO); // promotes immediately
        // reassembly_budget = 100ms, *1.1 = 110ms.
        let still_alive = r.tick(Duration::from_millis(109));
        assert!(still_alive.given_up.is_empty());
        let aged_out = r.tick(Duration::from_millis(111));
        assert_eq!(aged_out.given_up, alloc::vec![1]);
    }

    #[test]
    fn due_ranges_are_capped_at_sixteen_per_tick() {
        let mut c = cfg();
        c.reorder_section = Duration::ZERO;
        let mut r = Receiver::new(c);
        r.feed(0, Duration::ZERO);
        // Open 20 isolated single-packet gaps (each its own PacketRange).
        let mut seq_number = 1u16;
        for _ in 0..20 {
            seq_number += 2; // skip one to keep each loss isolated
            r.feed(seq_number, Duration::ZERO);
        }
        r.tick(Duration::ZERO); // promotes all 20 gaps immediately
        let due = r.tick(c.fallback_retransmission_interval()).due;
        assert_eq!(due.len(), MAX_RANGE_ENTRIES);
    }

    #[test]
    fn seqs_to_fci_matches_appendix_a_worked_example() {
        // TR-06-1 Appendix A: seq 100 lost, 101/102 received, 103-122 lost
        // (20 consecutive).
        let mut missing: Vec<u16> = alloc::vec![100];
        missing.extend(103..=122u16);
        let fci = seqs_to_fci(&missing);
        assert_eq!(
            fci,
            alloc::vec![
                NackFci {
                    pid: 100,
                    blp: 0b1111_1111_1111_1100
                },
                NackFci {
                    pid: 117,
                    blp: 0b0000_0000_0001_1111
                },
            ]
        );
    }

    #[test]
    fn ranges_to_fci_round_trips_through_range_form() {
        let ranges = alloc::vec![
            PacketRange {
                start: 100,
                additional: 0
            },
            PacketRange {
                start: 103,
                additional: 19
            },
        ];
        let fci = ranges_to_fci(&ranges);
        assert_eq!(
            fci,
            alloc::vec![
                NackFci {
                    pid: 100,
                    blp: 0b1111_1111_1111_1100
                },
                NackFci {
                    pid: 117,
                    blp: 0b0000_0000_0001_1111
                },
            ]
        );
    }
}
