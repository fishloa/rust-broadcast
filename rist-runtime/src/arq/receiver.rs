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

/// Baseline forward sequence-number gap [`Receiver`] will treat as ordinary
/// packet loss and backfill with [`MissingState`] entries (#978), under
/// TR-06-1 Appendix B's own suggested default receiver buffer (1000 ms).
/// [`Receiver::max_gap`] scales this baseline by the receiver's actually
/// configured [`ArqConfig::receiver_buffer`] (see [`derive_max_gap`]) rather
/// than applying it literally — a fixed 512 was found to be too small for a
/// legitimate high-bitrate burst (a burst longer than ~270 ms at 20 Mbit/s
/// already exceeds it, per r08-RIST-C2). **Implementation policy**, not a
/// TR-06-1 number — chosen as a generous multiple of Appendix B's suggested
/// default buffer depth in packets at a plausible bitrate, not a literal
/// transcription.
const BASE_MAX_GAP: u16 = 512;

/// Number of consecutive sequence numbers, each exactly continuing the one
/// before it, [`Receiver::feed`] requires before committing a jump beyond
/// [`Receiver::max_gap`] as a genuine stream resynchronisation (r08-RIST-C2).
/// An *unauthenticated* RTP sequence number lets one spoofed/forged packet
/// claim any position in the 16-bit space; requiring it to be immediately
/// followed by this many more packets that exactly continue it makes a
/// single forged packet inert (it is simply dropped as an unconfirmed
/// candidate) while a genuine resynchronised sender satisfies it within a
/// handful of packets. **Implementation policy** — TR-06-1 does not specify
/// a confirmation count.
const RESYNC_CONFIRM_COUNT: u32 = 3;

/// Derive a [`Receiver`]'s [`Receiver::max_gap`] ceiling from its configured
/// [`ArqConfig::receiver_buffer`]: [`BASE_MAX_GAP`] scaled proportionally to
/// how the configured buffer compares to Appendix B's own suggested default
/// (1000 ms), so a receiver configured to ride out a longer burst gets a
/// proportionally larger gap ceiling before an out-of-range jump is treated
/// as a resync candidate rather than ordinary tracked loss. Clamped to never
/// fall below [`BASE_MAX_GAP`] (a very small configured buffer still gets at
/// least the original ceiling) and never to exceed `u16::MAX` (the largest
/// gap [`seq::seq_diff`] can ever report).
fn derive_max_gap(receiver_buffer: Duration) -> u16 {
    let default_ms = ArqConfig::appendix_b_defaults()
        .receiver_buffer
        .as_millis()
        .max(1);
    let scaled = u128::from(BASE_MAX_GAP).saturating_mul(receiver_buffer.as_millis()) / default_ms;
    scaled.clamp(u128::from(BASE_MAX_GAP), u128::from(u16::MAX)) as u16
}

/// State of an in-progress, not-yet-confirmed resynchronisation candidate
/// (r08-RIST-C2): a run of consecutive sequence numbers seen beyond
/// [`Receiver::max_gap`], not yet acted on until it reaches
/// [`RESYNC_CONFIRM_COUNT`].
#[derive(Debug, Clone, Copy)]
struct PendingResync {
    /// The first sequence number of the candidate run — where the receiver
    /// will resynchronise to if/when this candidate is confirmed.
    base: u16,
    /// The next sequence number that must arrive to extend this candidate;
    /// anything else drops it (in favour of a fresh candidate, if the
    /// non-matching arrival is itself still beyond `max_gap`).
    next_needed: u16,
    /// Consecutive matching arrivals so far, including the one that opened
    /// the candidate.
    count: u32,
}

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
    /// This receiver's forward-gap ceiling (r08-RIST-C2): derived once at
    /// construction time from `config.receiver_buffer` (see
    /// [`derive_max_gap`]) since `ArqConfig` itself has no packet-count
    /// field to carry this — a `pub(crate)` field here instead, so the
    /// public `ArqConfig` surface is unchanged.
    pub(crate) max_gap: u16,
    /// An in-progress, not-yet-confirmed resynchronisation candidate, if
    /// the most recent out-of-range arrival hasn't yet reached
    /// [`RESYNC_CONFIRM_COUNT`].
    pending_resync: Option<PendingResync>,
    rtt: RttEstimator,
}

impl Receiver {
    /// A fresh receiver with no packets observed yet.
    pub fn new(config: ArqConfig) -> Self {
        let max_gap = derive_max_gap(config.receiver_buffer);
        Receiver {
            config,
            next_expected: None,
            received_ahead: BTreeSet::new(),
            missing: BTreeMap::new(),
            highest_received: None,
            max_gap,
            pending_resync: None,
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
                // seq_gt above guarantees seq_diff is in (0, SEQ_HALF], so
                // this always fits in a u16 — the cast is not lossy.
                let gap = seq::seq_diff(seq_number, highest) as u16;
                if gap > self.max_gap {
                    // r08-RIST-C2: a jump this large is handled by the
                    // resync-candidate path below, which requires
                    // RESYNC_CONFIRM_COUNT consecutive confirmations before
                    // acting — never resync (or blackhole the stream) on
                    // one arrival alone.
                    return self.feed_resync_candidate(seq_number);
                }
                // A genuine forward-progressing arrival within the gap
                // ceiling — any stale, unconfirmed resync candidate from an
                // earlier one-off out-of-range arrival is no longer
                // relevant.
                self.pending_resync = None;
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

    /// Handle one arrival whose forward gap from `highest_received` exceeds
    /// [`Self::max_gap`] (r08-RIST-C2): extend or (re)start the
    /// [`PendingResync`] candidate, and only actually resynchronise once it
    /// reaches [`RESYNC_CONFIRM_COUNT`] consecutive confirmations. Until
    /// then the arrival is an unconfirmed candidate — dropped, exactly like
    /// a genuinely bogus outlier, so a single spoofed packet cannot
    /// blackhole the stream.
    fn feed_resync_candidate(&mut self, seq_number: u16) -> DeliveryOutcome {
        match &mut self.pending_resync {
            Some(p) if p.next_needed == seq_number => {
                p.count += 1;
                p.next_needed = seq::seq_next(seq_number);
            }
            _ => {
                self.pending_resync = Some(PendingResync {
                    base: seq_number,
                    next_needed: seq::seq_next(seq_number),
                    count: 1,
                });
            }
        }

        let candidate = *self.pending_resync.as_ref().expect("just set above");
        if candidate.count < RESYNC_CONFIRM_COUNT {
            return DeliveryOutcome::default();
        }

        // Confirmed: the whole run from `base` to `seq_number` arrived
        // consecutively by construction of the matching above, so it is
        // now cumulatively deliverable in one go.
        self.pending_resync = None;
        self.missing.clear();
        self.received_ahead.clear();
        self.highest_received = Some(seq_number);
        self.next_expected = Some(seq::seq_next(seq_number));

        let mut delivered = Vec::new();
        let mut s = candidate.base;
        loop {
            delivered.push(s);
            if s == seq_number {
                break;
            }
            s = seq::seq_next(s);
        }
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

    /// #978 regression: a gap right at the receiver's `max_gap` boundary
    /// (`gap == max_gap`, not yet over it) is still tracked as ordinary
    /// loss — one `MissingState` per skipped seq, no resync candidate.
    #[test]
    fn a_gap_at_the_max_gap_boundary_is_still_tracked_as_ordinary_loss() {
        let mut r = Receiver::new(cfg());
        r.feed(0, Duration::ZERO);
        let max_gap = r.max_gap;
        let jump = seq::seq_add(0, max_gap); // gap == max_gap exactly
        let out = r.feed(jump, Duration::ZERO);
        assert!(out.delivered.is_empty());
        assert_eq!(r.missing_count(), usize::from(max_gap - 1));
        assert_eq!(r.next_expected(), Some(1));
    }

    /// r08-RIST-C2 regression: one past the boundary (`gap == max_gap + 1`)
    /// must NOT resync on a single packet — it only opens an unconfirmed
    /// resync candidate, which is dropped (not delivered, no missing
    /// entries, `next_expected`/`highest_received` untouched) until
    /// `RESYNC_CONFIRM_COUNT` consecutive continuations arrive.
    #[test]
    fn one_past_the_max_gap_boundary_starts_an_unconfirmed_candidate_not_a_reset() {
        let mut r = Receiver::new(cfg());
        r.feed(0, Duration::ZERO);
        let max_gap = r.max_gap;
        let jump = seq::seq_add(0, max_gap + 1); // gap == max_gap + 1
        let out = r.feed(jump, Duration::ZERO);
        assert!(
            out.delivered.is_empty(),
            "unconfirmed candidate must not deliver"
        );
        assert_eq!(r.missing_count(), 0);
        // Stream position is untouched by the mere candidate.
        assert_eq!(r.next_expected(), Some(1));

        // The opening candidate feed above already counts as 1; up to (but
        // not including) the RESYNC_CONFIRM_COUNT-th continuation, still
        // unconfirmed.
        for i in 1..(RESYNC_CONFIRM_COUNT - 1) {
            let out = r.feed(seq::seq_add(jump, i as u16), Duration::ZERO);
            assert!(out.delivered.is_empty());
        }
        assert_eq!(r.next_expected(), Some(1));

        // The RESYNC_CONFIRM_COUNT-th consecutive continuation confirms:
        // the whole run is delivered at once and the stream resyncs.
        let last = seq::seq_add(jump, (RESYNC_CONFIRM_COUNT - 1) as u16);
        let confirmed = r.feed(last, Duration::ZERO);
        assert_eq!(confirmed.delivered.len(), RESYNC_CONFIRM_COUNT as usize);
        assert_eq!(confirmed.delivered[0], jump);
        assert_eq!(*confirmed.delivered.last().unwrap(), last);
        assert_eq!(r.next_expected(), Some(seq::seq_next(last)));
    }

    /// r08-RIST-C2 regression: a 3000-packet loss burst is still ordinary
    /// tracked loss (not a resync) when the receiver is configured with a
    /// buffer wide enough to cover it — a fixed 512-packet ceiling would
    /// have wrongly resynced (and so never NACKed) a burst this size.
    #[test]
    fn a_large_burst_within_a_wide_enough_window_is_still_nacked_as_ordinary_loss() {
        let mut wide_cfg = cfg();
        // 6x Appendix B's default (1000ms) -> max_gap scales to 6x too.
        wide_cfg.receiver_buffer = Duration::from_millis(6000);
        let mut r = Receiver::new(wide_cfg);
        assert!(
            r.max_gap >= 3000,
            "max_gap = {} did not scale enough to cover the burst",
            r.max_gap
        );

        r.feed(0, Duration::ZERO);
        let jump = seq::seq_add(0, 3000); // a 3000-packet loss burst
        let out = r.feed(jump, Duration::ZERO);
        assert!(out.delivered.is_empty());
        assert_eq!(r.missing_count(), 2999);
        assert_eq!(r.next_expected(), Some(1));

        // It is genuinely NACK-eligible: ticking past reorder_section
        // promotes the whole burst.
        let promote_at = wide_cfg.reorder_section + Duration::from_millis(1);
        let ticked = r.tick(promote_at);
        assert_eq!(ticked.promoted.len(), 2999);
    }

    /// r08-RIST-C2 regression: one spoofed far-ahead packet must not
    /// blackhole the stream — the legitimate next-expected packet must
    /// still be deliverable normally right after it, proving the spoof was
    /// dropped as an unconfirmed candidate rather than resetting
    /// `next_expected`.
    #[test]
    fn one_spoofed_far_ahead_packet_does_not_blackhole_the_stream() {
        let mut r = Receiver::new(cfg());
        r.feed(0, Duration::ZERO);
        r.feed(1, Duration::ZERO);
        r.feed(2, Duration::ZERO); // seq 0,1,2 delivered, nothing missing

        // Forged/spoofed packet: seq jumps by far more than max_gap, and is
        // never followed up (a real spoof is a one-off, not a sustained
        // stream).
        let forged = seq::seq_add(2, 5000);
        let spoof_out = r.feed(forged, Duration::ZERO);
        assert!(
            spoof_out.delivered.is_empty(),
            "unconfirmed spoof must not deliver"
        );
        assert_eq!(
            r.missing_count(),
            0,
            "spoof must not backfill MissingState entries"
        );
        // Crucially: the stream position is untouched by the single spoof.
        assert_eq!(r.next_expected(), Some(3));

        // The genuine next packet still arrives and delivers normally —
        // the stream was never blackholed/reset by the spoof.
        let genuine = r.feed(3, Duration::ZERO);
        assert_eq!(genuine.delivered, alloc::vec![3]);
        assert_eq!(r.next_expected(), Some(4));
    }

    /// A confirmed resync (unlike a lone unconfirmed candidate) does clear
    /// any previously-buffered out-of-order packets — they're irrelevant
    /// once the stream has genuinely resynchronised past them.
    #[test]
    fn a_confirmed_resync_clears_previously_buffered_received_ahead() {
        let mut r = Receiver::new(cfg());
        r.feed(0, Duration::ZERO);
        r.feed(2, Duration::ZERO); // seq 1 missing, seq 2 buffered ahead
        assert_eq!(r.missing_count(), 1);

        let base = seq::seq_add(2, 5000);
        for i in 0..RESYNC_CONFIRM_COUNT {
            r.feed(seq::seq_add(base, i as u16), Duration::ZERO);
        }
        let last = seq::seq_add(base, (RESYNC_CONFIRM_COUNT - 1) as u16);
        assert_eq!(r.missing_count(), 0);
        assert_eq!(r.next_expected(), Some(seq::seq_next(last)));

        // Confirm `received_ahead` was really cleared, not just `missing`:
        // feeding the old buffered seq 2 again must not spuriously deliver
        // anything since it's now far behind `next_expected`.
        let out = r.feed(2, Duration::ZERO);
        assert!(out.delivered.is_empty());
    }

    /// r08-RIST-C2: a resync candidate run that wraps the 16-bit sequence
    /// space (`0xFFFF` -> `0`) is confirmed and delivered correctly.
    #[test]
    fn resync_confirmation_handles_16bit_wraparound() {
        let mut r = Receiver::new(cfg());
        assert_eq!(RESYNC_CONFIRM_COUNT, 3, "test assumes the current constant");
        // `seq::seq_diff` picks the *shorter* circular direction between two
        // sequence numbers, so a candidate must be a genuinely forward gap
        // from `highest` (not just numerically close to the 0xFFFF/0
        // boundary) to take the resync-candidate path at all. 0xFDE6 is
        // exactly 600 "before" 0xFFFE in the forward direction — comfortably
        // past the default max_gap (512) — so the jump to 0xFFFE opens a
        // candidate, and its RESYNC_CONFIRM_COUNT-confirmed run then
        // genuinely crosses the 0xFFFF -> 0 wrap.
        let highest: u16 = 0xFFFEu16.wrapping_sub(600); // 0xFDE6
        r.feed(highest, Duration::ZERO);

        let base = 0xFFFEu16;
        for i in 0..RESYNC_CONFIRM_COUNT {
            r.feed(seq::seq_add(base, i as u16), Duration::ZERO);
        }
        // base=0xFFFE, +1=0xFFFF, +2 wraps to 0 (RESYNC_CONFIRM_COUNT == 3).
        assert_eq!(r.next_expected(), Some(1));
        assert_eq!(r.missing_count(), 0);
    }
}
