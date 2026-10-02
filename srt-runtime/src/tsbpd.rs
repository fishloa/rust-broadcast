//! Timestamp-Based Packet Delivery (TSBPD) + Too-Late Packet Drop — SRT
//! receiver-side delivery scheduling.
//!
//! Spec grounding: [`draft-sharabayko-srt-01`](https://datatracker.ietf.org/doc/html/draft-sharabayko-srt-01)
//! §4.5 "Timestamp-Based Packet Delivery", §4.5.1 "Packet Delivery Time",
//! §4.5.1.1 "TSBPD Time Base Calculation", §4.6 "Too-Late Packet Drop", and
//! §4.7 "Drift Management" (for the `Drift` term in the `PktTsbpdTime`
//! formula). Curated behavioral rules: [`specs/rules/srt-tsbpd.md`](
//! https://github.com/fishloa/rust-broadcast/blob/main/specs/rules/srt-tsbpd.md).
//!
//! # Sans-IO contract
//!
//! [`TsbpdScheduler`] never reads a wall clock. All timing is driven by:
//! - [`TsbpdScheduler::feed_data`] — submit a received data packet's sequence
//!   number + 32-bit timestamp (from the SRT header). The scheduler computes
//!   the packet's `PktTsbpdTime` (rule 9) and stores it for timed delivery.
//! - [`TsbpdScheduler::tick`] — advance the virtual clock to `now` and release
//!   any packets whose play time has arrived, in sequence order. Also drops
//!   packets whose play time has already passed the too-late threshold
//!   (rule 17-19, if enabled).
//!
//! # Delivery model
//!
//! `tick` returns a [`TickOutcome`] containing:
//! - `delivered` — sequence numbers released to the application in order.
//! - `dropped` — sequence numbers dropped because they arrived after their
//!   play time (too-late drop, rules 17-18).
//!
//! Packets are always released in monotonically increasing sequence order.
//! A packet whose `PktTsbpdTime` has not yet arrived is withheld.
//!
//! # Time base, wrap and drift
//!
//! - `TsbpdTimeBase` (rule 12) is a *signed* microsecond offset: it is seeded
//!   as `T_NOW - HSREQ_TIMESTAMP`, which is negative whenever the peer's
//!   handshake timestamp exceeds the local time since the caller's epoch.
//! - The 32-bit wire timestamp wraps every ~71.58 minutes (rule 15). The
//!   scheduler unwraps it into an always-increasing value with a maintained
//!   reference point and a signed circular delta
//!   (`TsbpdScheduler::unwrap_timestamp`, mirroring `arq::seq::seq_diff` one
//!   bit wider), which is equivalent to adding `MAX_TIMESTAMP + 1` to the
//!   base at each wrap (rule 16) without a separate wrap-window state machine.
//!   (An earlier version used plain modular arithmetic, dropping `PktTsbpdTime`
//!   back near zero on every wrap — issue #1063.)
//! - Drift (§4.7, rules 26-27) is estimated internally from fresh in-sequence
//!   packets, in a packet-count-based window ([`DRIFT_SAMPLE_COUNT`]); the
//!   window average is applied as `Drift`, with any part beyond
//!   [`DRIFT_MAX_US`] moved into the time base, mirroring libsrt's tracer.
//!
//! # Fake ACK on skip
//! When this scheduler skips or drops packets under TLPKTDROP it reports them
//! in [`TickOutcome::dropped`]; moving the ARQ receiver's ack point past them
//! (the "fake ACK" of rule 22 / `srt-arq.md` rule 13) is the integration
//! layer's job, and [`crate::io`] does it (only when TLPKTDROP was
//! negotiated — with it off the scheduler never drops, and the ack point waits
//! for the retransmission).
//!
//! # Non-goals (explicit follow-ups)
//! - Sender-side TLPKTDROP (rule 18-20): out of scope for the receiver.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::time::Duration;

use crate::arq::seq;

/// How many fresh in-sequence packets make up one drift-sampling window
/// (`specs/rules/srt-tsbpd.md` rules 26-27: "based on packet count, not time
/// duration"). The draft gives no number (the srt-tsbpd.md "Ambiguous" list
/// says so); this matches the 1000-sample window of libsrt's drift tracer
/// (`srtcore/tsbpd_time.h`), so both ends of a link correct at the same pace.
pub const DRIFT_SAMPLE_COUNT: u32 = 1000;

/// The most a drift correction may move `Drift` itself, in microseconds; any
/// excess of the window average is folded into `TsbpdTimeBase` instead
/// (libsrt's tracer bound, 5 ms — implementation-defined, see
/// [`DRIFT_SAMPLE_COUNT`]).
pub const DRIFT_MAX_US: i64 = 5_000;

/// Minimum negotiated `TsbpdDelay` — 120 milliseconds
/// (`specs/rules/srt-tsbpd.md` rule 10, verbatim L2601-2603:
/// "The value of minimum TsbpdDelay is negotiated during the SRT handshake
/// exchange and is equal to 120 milliseconds.").
const TSBPD_DELAY_MIN_MS: u64 = 120;

/// A bound on how many sequence numbers one too-late skip may walk past a
/// gap. Not a spec rule — a safety cap against an adversarial or corrupt
/// sequence-number layout causing unbounded work in `release_ready`
/// (the receiver bounds its own gap tracking by the flow window instead).
const MAX_TLPKT_SKIP_SPAN: u32 = 1 << 16;

/// A bound on how many not-yet-reached DROPREQ ranges are remembered.
/// Not a spec rule — memory back-pressure against a peer flooding DROPREQs;
/// once reached, further gaps are cleared by the too-late skip instead.
#[cfg(feature = "tokio")]
const MAX_SKIPS: usize = 1024;

/// `now` as whole microseconds, saturating (a `Duration` past ~584 000 years).
fn duration_us(now: Duration) -> u64 {
    u64::try_from(now.as_micros()).unwrap_or(u64::MAX)
}

/// Outcome of one [`TsbpdScheduler::tick`] call.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct TickOutcome {
    /// Sequence numbers released to the application in monotonically
    /// increasing order — each at or after its `PktTsbpdTime`.
    pub delivered: Vec<u32>,
    /// Sequence numbers dropped because their play time was already past the
    /// too-late threshold upon arrival (or before `tick` could release them),
    /// or skipped as unrecoverable when a gap's earliest buffered successor
    /// went too late (§4.6, rule 21).
    pub dropped: Vec<u32>,
}

/// SRT receiver-side TSBPD delivery scheduler + Too-Late Packet Drop
/// (`draft-sharabayko-srt-01` §4.5/§4.6).
///
/// Sans-IO: never reads a wall clock. All timing is driven by caller-supplied
/// `now: core::time::Duration` in [`feed_data`](Self::feed_data) and
/// [`tick`](Self::tick).
///
/// # State variables (per `specs/rules/srt-tsbpd.md`)
///
/// - `TsbpdTimeBase` (µs, rule 12) — seeded at construction (signed), reflects
///   the clock difference between receiver-local time and the sender's
///   timestamp clock; moved by the drift tracer past [`DRIFT_MAX_US`].
/// - `TsbpdDelay` (ms, rule 10) — receiver latency buffer, floor 120 ms.
/// - `Drift` (µs, rules 24-27) — current drift correction; seeded at
///   construction and re-estimated every [`DRIFT_SAMPLE_COUNT`] fresh packets.
/// - `TLPKTDROP_THRESHOLD` (rule 19) — threshold beyond which a packet whose
///   play time has passed is dropped; enabled by default.
/// - `next_release` — the next sequence number to release (cumulative delivery
///   point).
#[derive(Debug)]
pub struct TsbpdScheduler {
    /// `TsbpdTimeBase` — time base reflecting the clock difference between
    /// receiver-local time and the sender's packet-timestamping clock
    /// (µs, `specs/rules/srt-tsbpd.md` rule 12, §4.5.1.1 L2612-2618).
    tsbpd_time_base: i64,
    /// `TsbpdDelay` — receiver's buffer delay, in milliseconds
    /// (rule 9-10, §4.5.1 L2588-2592).
    tsbpd_delay_ms: u64,
    /// `Drift` — time drift correction between sender/receiver clocks, in
    /// microseconds, signed (rule 9, §4.7 L2757-2765).
    drift_us: i64,
    /// Running sum of the current drift window's samples (µs).
    drift_sample_sum: i64,
    /// Samples in the current drift window.
    drift_sample_count: u32,
    /// `TLPKTDROP_THRESHOLD` — threshold for too-late packet drop, in
    /// microseconds (rule 19, §4.6 L2664-2670). Computed as
    /// `1.25 * TsbpdDelay_ms * 1000` when constructed; exposed as a field
    /// so the caller can customize.
    tlpktdrop_threshold_us: u64,
    /// Whether too-late packet drop is enabled (rule 23, §4.6 L2729-2733).
    tlpktdrop_enabled: bool,
    /// The next sequence number to release — cumulative delivery point.
    next_release: u32,
    /// Out-of-order packets buffered for timed delivery, keyed by sequence
    /// number. Each entry holds the computed `PktTsbpdTime` in microseconds.
    /// Unordered delivery (notably `BTreeMap`) — out-of-order packets are
    /// inserted here until their predecessors arrive.
    buffer: BTreeMap<u32, u64>,
    /// The highest sequence number ever fed — used to detect monotonically
    /// increasing deliveries when no gaps remain.
    highest_fed: Option<u32>,
    /// DROPREQ ranges (§3.2.9) the delivery cursor has not reached yet, as
    /// inclusive `(first, last)` pairs. `release_ready` jumps the cursor past
    /// them on arrival; pruned once passed.
    pending_skips: Vec<(u32, u32)>,
    /// The current reference point for unwrapping the 32-bit wire timestamp
    /// into an always-increasing microsecond value (issue #1063): the most
    /// recently *advanced-to* raw timestamp and the absolute value it was
    /// unwrapped to. `None` until the first packet.
    ts_unwrap_reference: Option<(u32, u64)>,
}

impl TsbpdScheduler {
    /// Create a new TSBPD scheduler.
    ///
    /// # Parameters
    ///
    /// * `initial_seq` — the first expected sequence number (the peer's ISN).
    /// * `tsbpd_time_base` — `TsbpdTimeBase` in microseconds (signed), seeded
    ///   per rule 12 (`T_NOW - HSREQ_TIMESTAMP`).
    /// * `tsbpd_delay_ms` — `TsbpdDelay` in milliseconds (rule 10). A value
    ///   below the minimum 120 ms is silently raised to 120 ms.
    /// * `drift_us` — initial `Drift` correction in microseconds, signed
    ///   (rule 9); supply `0` when no drift estimate is available.
    /// * `tlpktdrop_enabled` — whether too-late packet drop is enabled
    ///   (rule 23).
    /// * `tlpktdrop_threshold_us` — custom too-late threshold in microseconds.
    ///   If `None`, the recommended default `1.25 × TsbpdDelay` is used
    ///   (rule 19).
    pub fn new(
        initial_seq: u32,
        tsbpd_time_base: i64,
        tsbpd_delay_ms: u64,
        drift_us: i64,
        tlpktdrop_enabled: bool,
        tlpktdrop_threshold_us: Option<u64>,
    ) -> Self {
        let tsbpd_delay_ms = tsbpd_delay_ms.max(TSBPD_DELAY_MIN_MS);
        let tlpktdrop_threshold_us = tlpktdrop_threshold_us.unwrap_or_else(|| {
            // Recommended threshold: `1.25 × SRT_latency` (rule 19).
            // TsbpdDelay is in ms, convert to µs. Use integer arithmetic:
            // tsbpd_delay_ms * 1250 / 1000 = tsbpd_delay_ms * 5 / 4 * 1000
            // Rounded up via (a * 5 + 3) / 4 to match ceil(1.25 * delay).
            tsbpd_delay_ms
                .saturating_mul(5)
                .div_ceil(4)
                .saturating_mul(1000)
        });
        TsbpdScheduler {
            tsbpd_time_base,
            tsbpd_delay_ms,
            drift_us,
            drift_sample_sum: 0,
            drift_sample_count: 0,
            tlpktdrop_threshold_us,
            tlpktdrop_enabled,
            next_release: initial_seq,
            buffer: BTreeMap::new(),
            highest_fed: None,
            pending_skips: Vec::new(),
            ts_unwrap_reference: None,
        }
    }

    /// Computed PktTsbpdTime for a packet.
    ///
    /// Per `specs/rules/srt-tsbpd.md` rule 9 (verbatim from
    /// `draft-sharabayko-srt-01` L2581):
    ///
    /// > PktTsbpdTime = TsbpdTimeBase + PKT_TIMESTAMP + TsbpdDelay + Drift
    ///
    /// where:
    /// - `TsbpdTimeBase` is in µs (rule 12).
    /// - `PKT_TIMESTAMP` is in µs (§3.1).
    /// - `TsbpdDelay` is in ms (rule 10), converted to µs by ×1000.
    /// - `Drift` is in µs (rule 9).
    ///
    /// Computed in signed saturating arithmetic (the time base and drift are
    /// signed); a play time before the epoch is clamped to `0` ("already due").
    fn pkt_tsbpd_time(&self, unwrapped_pkt_timestamp_us: u64) -> u64 {
        let sum = self
            .tsbpd_time_base
            .saturating_add(i64::try_from(unwrapped_pkt_timestamp_us).unwrap_or(i64::MAX))
            .saturating_add(
                i64::try_from(self.tsbpd_delay_ms.saturating_mul(1000)).unwrap_or(i64::MAX),
            )
            .saturating_add(self.drift_us);
        u64::try_from(sum).unwrap_or(0)
    }

    /// The current `TsbpdTimeBase` in microseconds (rule 12; moved by the
    /// drift tracer past [`DRIFT_MAX_US`]).
    pub fn time_base_us(&self) -> i64 {
        self.tsbpd_time_base
    }

    /// The current `Drift` correction in microseconds (rule 9, §4.7).
    pub fn drift_us(&self) -> i64 {
        self.drift_us
    }

    /// Add one drift sample for a fresh packet that arrived at `now_us` with
    /// (unwrapped) timestamp `unwrapped_ts`: how far its arrival is past the
    /// `TsbpdTimeBase + PKT_TIMESTAMP` the receiver expected (§4.7). After
    /// [`DRIFT_SAMPLE_COUNT`] samples the window average becomes the new
    /// `Drift`; the part beyond [`DRIFT_MAX_US`] is folded into the time base
    /// so `Drift` stays small (the same split libsrt's tracer makes). The
    /// window is packet-count based, not time based (rule 27).
    fn add_drift_sample(&mut self, now_us: u64, unwrapped_ts: u64) {
        let arrival = i64::try_from(now_us).unwrap_or(i64::MAX);
        let expected = self
            .tsbpd_time_base
            .saturating_add(i64::try_from(unwrapped_ts).unwrap_or(i64::MAX));
        self.drift_sample_sum = self
            .drift_sample_sum
            .saturating_add(arrival.saturating_sub(expected));
        self.drift_sample_count += 1;
        if self.drift_sample_count < DRIFT_SAMPLE_COUNT {
            return;
        }
        let average = self.drift_sample_sum / i64::from(DRIFT_SAMPLE_COUNT);
        self.drift_sample_sum = 0;
        self.drift_sample_count = 0;
        let overdrift = if average > DRIFT_MAX_US {
            average - DRIFT_MAX_US
        } else if average < -DRIFT_MAX_US {
            average + DRIFT_MAX_US
        } else {
            0
        };
        self.tsbpd_time_base = self.tsbpd_time_base.saturating_add(overdrift);
        self.drift_us = average - overdrift;
    }

    /// Extend a packet's raw 32-bit wire timestamp into an always-increasing
    /// microsecond value, correctly spanning a real wraparound past `2^32`
    /// microseconds (~71.58 minutes, `specs/rules/srt-tsbpd.md` rule 15) —
    /// without this, `pkt_tsbpd_time` fed the raw value straight through, so
    /// every packet after a wrap computed a `PktTsbpdTime` far in the past
    /// (the field dropped back near 0) and got Too-Late-Packet-Dropped
    /// forever (issue #1063).
    ///
    /// Mirrors `arq::seq::seq_diff` (signed circular distance) one bit wider:
    /// the shortest signed delta from the current reference's raw value to
    /// `raw`, added to the reference's absolute value. Correct as long as
    /// consecutive *reference advances* stay within +/-2^31 us (~35.79 min)
    /// of each other — true for any stream whose packets keep arriving more
    /// often than that. The reference only ever advances forward (to the
    /// highest absolute value produced so far); an out-of-order/retransmitted
    /// packet earlier than the current reference is still unwrapped
    /// correctly against it, it just doesn't move the reference.
    fn unwrap_timestamp(&mut self, raw: u32) -> u64 {
        let Some((ref_raw, ref_abs)) = self.ts_unwrap_reference else {
            self.ts_unwrap_reference = Some((raw, u64::from(raw)));
            return u64::from(raw);
        };
        let mut delta = i64::from(raw) - i64::from(ref_raw);
        if delta < -(1i64 << 31) {
            delta += 1i64 << 32;
        } else if delta >= (1i64 << 31) {
            delta -= 1i64 << 32;
        }
        let abs = u64::try_from(
            i64::try_from(ref_abs)
                .unwrap_or(i64::MAX)
                .saturating_add(delta),
        )
        .unwrap_or(0);
        if abs > ref_abs {
            self.ts_unwrap_reference = Some((raw, abs));
        }
        abs
    }

    /// Feed a received data packet's sequence number and timestamp.
    ///
    /// Returns [`TickOutcome`] with any packets that can be immediately
    /// delivered (when the packet fills the next-released sequence gap and
    /// its play time is at or before `now`), plus any packets that are
    /// already too late on arrival.
    ///
    /// # Parameters
    ///
    /// * `seq_number` — the data packet's 31-bit sequence number.
    /// * `pkt_timestamp` — the data packet's 32-bit timestamp (§3.1).
    /// * `now` — current receiver time since the fixed epoch (the `T_NOW`
    ///   used to decide whether `PktTsbpdTime ≤ now`).
    pub fn feed_data(&mut self, seq_number: u32, pkt_timestamp: u32, now: Duration) -> TickOutcome {
        let now_us = duration_us(now);
        let unwrapped_ts = self.unwrap_timestamp(pkt_timestamp);

        // Track highest fed (monotonic, for detecting when delivery advances
        // with no gap). A *fresh* packet — the very next sequence number, not
        // a retransmission or a gap-jumper — is a clean drift sample: its
        // arrival is not delayed by a NAK round trip.
        match self.highest_fed {
            None => {
                self.highest_fed = Some(seq_number);
                self.add_drift_sample(now_us, unwrapped_ts);
            }
            Some(h) if seq::seq_gt(seq_number, h) => {
                if seq_number == seq::seq_next(h) {
                    self.add_drift_sample(now_us, unwrapped_ts);
                }
                self.highest_fed = Some(seq_number);
            }
            _ => {}
        }
        // After the drift sample: it may have just moved the time base.
        let pkt_tsbpd_time = self.pkt_tsbpd_time(unwrapped_ts);

        // Check if this packet is already too late on arrival.
        // Rule 17-18/21: drop a packet whose PktTsbpdTime is before
        // (now - TLPKTDROP_THRESHOLD).
        if self.tlpktdrop_enabled {
            let drop_before = now_us.saturating_sub(self.tlpktdrop_threshold_us);
            if pkt_tsbpd_time < drop_before {
                // Packet is too late — drop it immediately.
                let mut dropped = alloc::vec![seq_number];
                // Advance past the dropped sequence if it's the next
                // expected, so the queue doesn't stall.
                if seq_number == self.next_release {
                    self.next_release = seq::seq_next(seq_number);
                }
                // Remove from buffer if it was somehow already there.
                self.buffer.remove(&seq_number);
                // Try to release any now-unblocked packets that are also
                // too-late (the receiver-buffer read pseudocode, rule 21:
                // "Drop packets which buffer position number is less than
                // i").
                let (delivered, mut later_dropped) = self.release_ready(now_us);
                dropped.append(&mut later_dropped);
                return TickOutcome { delivered, dropped };
            }
        }

        // Ignore duplicate of an already-delivered sequence.
        if !seq::seq_lt(seq_number, self.next_release) {
            // Insert into the buffer (or replace — a retransmission arriving
            // after the original is fine; the PktTsbpdTime is the same per
            // rule 4 L2496-2502).
            self.buffer.insert(seq_number, pkt_tsbpd_time);
        }

        let (delivered, dropped) = self.release_ready(now_us);
        TickOutcome { delivered, dropped }
    }

    /// Advance the virtual clock and release/drop packets whose time has
    /// come.
    ///
    /// Call this periodically (e.g. every millisecond) to drain the buffer.
    /// Packets whose `PktTsbpdTime ≤ now` are released in sequence order.
    /// If too-late drop is enabled, packets whose play time has already
    /// passed the threshold are dropped instead.
    pub fn tick(&mut self, now: Duration) -> TickOutcome {
        let now_us = duration_us(now);
        let (delivered, dropped) = self.release_ready(now_us);
        TickOutcome { delivered, dropped }
    }

    /// Release all packets that are ready for delivery in sequence order.
    ///
    /// Walks forward from `self.next_release` while the next packet is
    /// present in the buffer AND its `PktTsbpdTime ≤ now_us`. Returns the
    /// released sequence numbers plus those dropped as too late. If too-late
    /// drop is enabled, packets whose scheduled play time is already past
    /// `(now_us - TLPKTDROP_THRESHOLD)` are dropped instead of delivered
    /// (matching the receiver-buffer read pseudocode, rule 21: "if
    /// T_NOW < PktTsbpdTime: continue;" / "Drop packets which buffer
    /// position number is less than i;" / "Deliver packet ...").
    ///
    /// With too-late drop enabled, a *missing* head-of-line packet is also
    /// skipped as soon as the earliest buffered successor's play time has
    /// arrived: that successor is then delivered (or dropped only if it is in
    /// turn past its own too-late threshold), while the missing positions
    /// before it are reported dropped (§4.6; rule 21's "Drop packets which
    /// buffer position number is less than i" / "Deliver packet with the
    /// buffer position i"). With too-late drop disabled the scheduler keeps
    /// waiting for the gap indefinitely, preserving reliable in-order
    /// delivery.
    ///
    /// The logic here follows the pseudocode from
    /// `specs/rules/srt-tsbpd.md` rule 21 (L2693-2715):
    ///
    /// ```text
    /// while(True) {
    ///     i = next_avail();
    ///     PktTsbpdTime = delivery_time(i);
    ///     if T_NOW < PktTsbpdTime:
    ///         continue;
    ///     Drop packets which buffer position number is less than i;
    ///     Deliver packet with the buffer position i;
    ///     pos = i + 1;
    /// }
    /// ```
    fn release_ready(&mut self, now_us: u64) -> (Vec<u32>, Vec<u32>) {
        let mut delivered = Vec::new();
        let mut dropped = Vec::new();

        loop {
            if let Some(&tsbpd_time) = self.buffer.get(&self.next_release) {
                if now_us < tsbpd_time {
                    break; // not yet time
                }

                // Too-late drop check: if PktTsbpdTime is so far in the past
                // that it's past the drop threshold, drop instead of deliver.
                if self.tlpktdrop_enabled {
                    let drop_before = now_us.saturating_sub(self.tlpktdrop_threshold_us);
                    if tsbpd_time < drop_before {
                        // Drop this packet and any others before the skip
                        // point. The receiver-buffer pseudocode says "Drop
                        // packets which buffer position number is less than
                        // i" — i.e. we drop the packet at position i and
                        // advance past it. (rule 21, L2693-2715)
                        self.buffer.remove(&self.next_release);
                        dropped.push(self.next_release);
                        self.next_release = seq::seq_next(self.next_release);
                        continue;
                    }
                }

                // Deliver.
                self.buffer.remove(&self.next_release);
                delivered.push(self.next_release);
                self.next_release = seq::seq_next(self.next_release);
                continue;
            }

            // Gap at `next_release`. A DROPREQ (§3.2.9) the sender already
            // announced for this range: those packets are gone by its own
            // admission — jump past them immediately, in any delivery mode.
            if let Some(pos) = self
                .pending_skips
                .iter()
                .position(|&(f, l)| seq::seq_in_closed_range(self.next_release, f, l))
            {
                let (_, last) = self.pending_skips.remove(pos);
                self.next_release = seq::seq_next(last);
                continue;
            }

            // Too-late drop disabled → wait for the missing packet
            // (reliable delivery).
            if !self.tlpktdrop_enabled {
                break;
            }

            // §4.6 / rule 21 pseudocode: `i = next_avail()` — the first
            // buffered packet at or after the cursor. If its play time has
            // not arrived, keep waiting (a retransmission of the missing one
            // could still make it). Once it HAS arrived, drop the missing
            // positions before `i` and let the loop deliver `i` itself — it
            // is dropped only if it is in turn past its own too-late
            // threshold, via the branch above.
            let Some((&skip_to, &skip_time)) = self
                .buffer
                .range(self.next_release..)
                .chain(self.buffer.range(..self.next_release))
                .next()
            else {
                break;
            };
            if now_us < skip_time {
                break; // T_NOW < PktTsbpdTime(i) — keep waiting
            }
            // Defensive bound against an adversarial sequence-number layout
            // (the ARQ receiver bounds its tracking by the flow window): a skip span
            // this large means the buffered timestamps cannot be trusted to
            // describe one contiguous live stream — wait rather than walk.
            if !u32::try_from(seq::seq_diff(skip_to, self.next_release))
                .is_ok_and(|span| span <= MAX_TLPKT_SKIP_SPAN)
            {
                break;
            }
            // Drop everything before the skip point (rule 21: "Drop packets
            // which buffer position number is less than i") — including any
            // buffered entries whose play time has already been overtaken.
            let mut s = self.next_release;
            while s != skip_to {
                self.buffer.remove(&s);
                dropped.push(s);
                s = seq::seq_next(s);
            }
            self.next_release = skip_to;
        }

        (delivered, dropped)
    }

    /// Whether Too-Late Packet Drop is enabled (§4.6): `false` makes the
    /// scheduler wait for a missing packet indefinitely.
    pub fn tlpktdrop_enabled(&self) -> bool {
        self.tlpktdrop_enabled
    }

    /// The next sequence number expected for release.
    pub fn next_release(&self) -> u32 {
        self.next_release
    }

    /// Mark the inclusive circular range `first..=last` as permanently
    /// unavailable — the peer sent a DROPREQ (§3.2.9) announcing it will
    /// never deliver those packets.
    ///
    /// Any buffered entries inside the range are discarded (the sender has
    /// given up on them; delivering them late would break message integrity),
    /// and when the delivery cursor reaches the range it jumps past it: if
    /// `next_release` is already inside, immediately; otherwise the range is
    /// recorded so [`Self::tick`]/[`Self::feed_data`] skip it the moment the
    /// cursor arrives — unblocking everything behind the gap without waiting
    /// for the too-late threshold. A `last` preceding `first` is a malformed
    /// (empty) range and ignored.
    //
    // The DROPREQ consumer lives in the tokio adapter (`crate::io`); the
    // engine core itself never receives packets, so it is gated accordingly.
    #[cfg(feature = "tokio")]
    pub(crate) fn skip_range(&mut self, first: u32, last: u32) {
        if seq::seq_diff(last, first) < 0 {
            // `last` precedes `first` — a malformed DROPREQ range; ignoring
            // it is safer than treating every sequence as skipped.
            return;
        }
        self.buffer
            .retain(|&s, _| !seq::seq_in_closed_range(s, first, last));
        // Ranges the cursor has already passed are dead weight — prune them.
        self.pending_skips
            .retain(|&(_, l)| !seq::seq_lt(l, self.next_release));
        if seq::seq_in_closed_range(self.next_release, first, last) {
            self.next_release = seq::seq_next(last);
        } else if seq::seq_lt(self.next_release, first) && self.pending_skips.len() < MAX_SKIPS {
            // Ahead of the cursor: remember it so `release_ready` jumps the
            // gap when it gets there. Bounded by `MAX_SKIPS`; a peer that
            // floods DROPREQs beyond that is served by the too-late skip.
            self.pending_skips.push((first, last));
        }
    }

    /// Number of packets currently buffered and awaiting their play time.
    pub fn buffered_count(&self) -> usize {
        self.buffer.len()
    }

    /// Whether the buffer has a gap (the next expected sequence number is
    /// not present).
    pub fn has_gap(&self) -> bool {
        !self.buffer.contains_key(&self.next_release)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use core::time::Duration;

    const TIME_BASE: u64 = 1_000_000; // arbitrary TsbpdTimeBase
    const TIME_BASE_I64: i64 = 1_000_000; // the same, as the scheduler's signed seed
    const DELAY_MS: u64 = 120; // minimum
    const ISN: u32 = 0;

    /// Helper: default scheduler for tests.
    fn sched() -> TsbpdScheduler {
        TsbpdScheduler::new(ISN, TIME_BASE_I64, DELAY_MS, 0, true, None)
    }

    #[test]
    fn pkt_tsbpd_time_formula() {
        let s = sched();
        // PktTsbpdTime = TsbpdTimeBase + PKT_TIMESTAMP + TsbpdDelay_us + Drift
        let ts = 5000u32;
        let expected = TIME_BASE + u64::from(ts) + DELAY_MS * 1000;
        assert_eq!(s.pkt_tsbpd_time(u64::from(ts)), expected);
    }

    #[test]
    fn in_order_after_delay() {
        let mut s = sched();
        let ts_a = 0u32;
        let ts_b = 10_000u32; // 10 ms later

        // Feed at now=0 — neither is deliverable yet (PktTsbpdTime is
        // in the future).
        let outcome = s.feed_data(0, ts_a, Duration::ZERO);
        assert!(outcome.delivered.is_empty());
        assert!(outcome.dropped.is_empty());
        assert_eq!(s.buffered_count(), 1);

        let outcome = s.feed_data(1, ts_b, Duration::ZERO);
        assert!(outcome.delivered.is_empty());
        assert!(outcome.dropped.is_empty());
        assert_eq!(s.buffered_count(), 2);

        // Advance clock past packet 1's PktTsbpdTime but not packet 2's.
        let pkt1_tsbpd = TIME_BASE + u64::from(ts_a) + DELAY_MS * 1000;
        let outcome = s.tick(Duration::from_micros(pkt1_tsbpd));
        assert_eq!(outcome.delivered, vec![0]);

        let pkt2_tsbpd = TIME_BASE + u64::from(ts_b) + DELAY_MS * 1000;
        let outcome = s.tick(Duration::from_micros(pkt2_tsbpd));
        assert_eq!(outcome.delivered, vec![1]);
        assert!(s.buffer.is_empty());
    }

    #[test]
    fn out_of_order_arrival() {
        let mut s = sched();
        // Packet 1 arrives out of order before packet 0 — fed well before its
        // play time, so it buffers waiting for the gap (at the play time
        // itself rule 21 would skip the gap; see
        // `gap_skip_delivers_successor_at_its_play_time`).
        let ts = 10_000u32;
        let pkt1_tsbpd = TIME_BASE + 10_000 + DELAY_MS * 1000;
        let outcome = s.feed_data(1, ts, Duration::ZERO);
        assert!(outcome.delivered.is_empty());
        assert_eq!(s.buffered_count(), 1);

        // Now packet 0 arrives — tick at the same time so that both are
        // past their play time and can be delivered together.
        let outcome = s.feed_data(0, 0, Duration::from_micros(pkt1_tsbpd));
        // Both should be delivered in order.
        assert_eq!(outcome.delivered, vec![0, 1]);
        assert!(s.buffer.is_empty());
    }

    #[test]
    fn too_late_drop_on_arrival() {
        let mut s = sched();
        // The drop threshold is 1.25 × 120ms = 150ms.
        // Feed a packet whose PktTsbpdTime is deep in the past — well past
        // the drop threshold.
        let pkt_tsbpd = TIME_BASE + DELAY_MS * 1000;
        // Arrive now at PktTsbpdTime + threshold + 1 µs — just past the
        // drop window.
        let threshold_us = (DELAY_MS * 5).div_ceil(4) * 1000;
        let very_late_now = Duration::from_micros(pkt_tsbpd + threshold_us + 1);
        let outcome = s.feed_data(0, 0, very_late_now);
        assert!(
            outcome.delivered.is_empty(),
            "should not deliver late packet"
        );
        assert_eq!(outcome.dropped, vec![0]);
    }

    #[test]
    fn too_late_drop_buffered() {
        // Packets 1 and 2 arrive while packet 0 is missing. At packet 1's
        // play time the gap is skipped and packet 1 delivered (§4.6 / rule
        // 21); a buffered packet that itself goes past its too-late threshold
        // — here packet 2, starved of ticks between the two clock jumps — is
        // dropped instead.
        let mut s = TsbpdScheduler::new(0, TIME_BASE_I64, DELAY_MS, 0, true, None);

        // Feed packets 1 and 2 well before their play times — they buffer.
        s.feed_data(1, 10_000, Duration::ZERO);
        s.feed_data(2, 20_000, Duration::ZERO);
        assert_eq!(s.buffered_count(), 2);

        // At packet 1's play time the missing packet 0 is reported dropped
        // and packet 1 delivered — not thrown away with the gap.
        let pkt1_tsbpd = TIME_BASE + 10_000 + DELAY_MS * 1000;
        let outcome = s.tick(Duration::from_micros(pkt1_tsbpd));
        assert_eq!(outcome.delivered, vec![1]);
        assert_eq!(outcome.dropped, vec![0]);

        // Packet 2 is now left sitting past its too-late threshold.
        let pkt2_tsbpd = TIME_BASE + 20_000 + DELAY_MS * 1000;
        let threshold = (DELAY_MS * 5).div_ceil(4) * 1000;
        let outcome = s.tick(Duration::from_micros(pkt2_tsbpd + threshold + 1));
        assert!(outcome.delivered.is_empty());
        assert_eq!(outcome.dropped, vec![2]);

        // A late duplicate of the skipped packet 0 is a no-op — the cursor
        // has already advanced past it.
        let outcome = s.feed_data(0, 0, Duration::from_micros(pkt2_tsbpd + threshold * 10));
        assert!(outcome.delivered.is_empty());
        assert_eq!(s.next_release(), 3);
        assert_eq!(s.buffered_count(), 0);
    }

    #[test]
    fn sequence_order_preserved() {
        let mut s = sched();
        // Deliver several packets in order.
        let ts_step = 10_000u32;
        let num_packets = 5;
        for i in 0..num_packets {
            s.feed_data(i, ts_step * i, Duration::ZERO);
        }
        // All are buffered.
        assert_eq!(s.buffered_count(), num_packets as usize);

        // Tick past the last packet's play time.
        let last_tsbpd =
            TIME_BASE + u64::from(ts_step) * (num_packets - 1) as u64 + DELAY_MS * 1000;
        let outcome = s.tick(Duration::from_micros(last_tsbpd));
        assert_eq!(outcome.delivered, (0..num_packets).collect::<Vec<_>>());
    }

    #[test]
    fn timestamp_wrap_smoke() {
        // Issue #1063: a real wraparound of the 32-bit wire timestamp
        // (`past_wrap` arriving after `near_wrap`, having actually wrapped
        // past `2^32` microseconds) must unwrap to an ever-*increasing*
        // absolute time — not the raw numeric value, which drops back near
        // 0 and would make `past_wrap`'s `PktTsbpdTime` look far in the
        // *past* relative to `near_wrap`'s (the pre-fix bug: every packet
        // after a real wrap computed a play time the receiver's clock had
        // already passed, so Too-Late-Packet-Drop discarded it forever).
        let mut s = TsbpdScheduler::new(0, TIME_BASE_I64, DELAY_MS, 0, false, None);
        // A timestamp near the 32-bit max value.
        let near_wrap: u32 = 0xFFFF_FF00u32;
        // The same clock, 756 us later, having wrapped past 0
        // (`0x1_0000_0000 - 0xFFFF_FF00 + 500 = 256 + 500 = 756`).
        let past_wrap: u32 = 500;

        let outcome = s.feed_data(0, near_wrap, Duration::ZERO);
        assert!(outcome.delivered.is_empty());

        let outcome = s.feed_data(1, past_wrap, Duration::ZERO);
        assert!(outcome.delivered.is_empty());
        assert_eq!(s.buffered_count(), 2);

        // Correctly unwrapped, pkt1 (the later, post-wrap packet) plays
        // 756 µs AFTER pkt0. Naively widening the raw u32 would give pkt1 a
        // play time ~71 minutes BEFORE pkt0's, so it would be released the
        // instant pkt0 is — that is what the checkpoints below pin down.
        let pkt0_tsbpd = TIME_BASE + u64::from(near_wrap) + DELAY_MS * 1000;
        let outcome = s.tick(Duration::from_micros(pkt0_tsbpd));
        assert_eq!(
            outcome.delivered,
            vec![0],
            "at pkt0's play time only pkt0 is due"
        );
        let outcome = s.tick(Duration::from_micros(pkt0_tsbpd + 755));
        assert!(outcome.delivered.is_empty(), "pkt1 is 756 µs later");
        let outcome = s.tick(Duration::from_micros(pkt0_tsbpd + 756));
        assert_eq!(outcome.delivered, vec![1]);
    }

    #[test]
    fn unwrap_timestamp_stays_monotonic_across_many_wraps() {
        // A longer-lived connection wraps the 32-bit timestamp every
        // ~71.58 min. Simulate one by walking the raw wire value forward in
        // realistic, sub-half-range steps (1 real second each, `wrapping_add`
        // so it genuinely rolls over at `u32::MAX` the way the wire field
        // does) through more than two full wraps, and confirm the unwrapped
        // absolute value strictly increases by exactly one step every time
        // — never resetting or going backward at either wrap boundary.
        let mut s = TsbpdScheduler::new(0, TIME_BASE_I64, DELAY_MS, 0, false, None);
        const STEP: u32 = 1_000_000; // 1 second in microseconds
        let iterations = (u64::from(u32::MAX) / u64::from(STEP)) * 2 + 10; // > 2 wraps

        let mut raw: u32 = 0;
        let mut expected_abs = u64::from(raw);
        let mut prev_abs = s.unwrap_timestamp(raw);
        assert_eq!(prev_abs, expected_abs);

        for _ in 0..iterations {
            raw = raw.wrapping_add(STEP);
            expected_abs += u64::from(STEP);
            let abs = s.unwrap_timestamp(raw);
            assert_eq!(abs, expected_abs, "raw={raw:#x}");
            assert!(abs > prev_abs, "prev={prev_abs} abs={abs}");
            prev_abs = abs;
        }
    }

    #[test]
    fn unwrap_timestamp_handles_reordered_packet_without_moving_reference() {
        // An out-of-order/retransmitted packet earlier than the current
        // reference must still unwrap correctly against it, and must not
        // itself move the reference backward.
        let mut s = TsbpdScheduler::new(0, TIME_BASE_I64, DELAY_MS, 0, false, None);
        let a = s.unwrap_timestamp(10_000);
        let b = s.unwrap_timestamp(20_000); // reference advances to here
        let reordered = s.unwrap_timestamp(15_000); // arrives late, between a and b
        assert_eq!(a, 10_000);
        assert_eq!(b, 20_000);
        assert_eq!(reordered, 15_000);
        // Reference must still be at `b` (20_000), not `reordered`: the next
        // packet close to `b` unwraps relative to `b`, not `reordered`.
        let next = s.unwrap_timestamp(20_500);
        assert_eq!(next, 20_500);
    }

    #[test]
    fn minimum_delay_floor_applied() {
        // A delay below 120 ms should be silently raised.
        let s = TsbpdScheduler::new(0, TIME_BASE_I64, 10, 0, false, None);
        let ts = 0u32;
        // The computed PktTsbpdTime should use 120 ms, not 10 ms.
        let expected = TIME_BASE + u64::from(ts) + TSBPD_DELAY_MIN_MS * 1000;
        assert_eq!(s.pkt_tsbpd_time(u64::from(ts)), expected);
    }

    #[test]
    fn tlpktdrop_disabled_never_drops() {
        let mut s = TsbpdScheduler::new(0, TIME_BASE_I64, DELAY_MS, 0, false, None);
        // Feed a packet very late but with tlpktdrop disabled.
        let ts = 0u32;
        let very_late_now =
            Duration::from_micros(TIME_BASE + u64::from(ts) + DELAY_MS * 1000 + 1_000_000);
        let outcome = s.feed_data(0, ts, very_late_now);
        // Should be delivered (not dropped) because tlpktdrop is disabled.
        assert_eq!(outcome.delivered, vec![0]);
        assert!(outcome.dropped.is_empty());
    }

    #[test]
    fn gap_blocks_delivery() {
        // Reliable mode (too-late drop disabled): a gap blocks delivery no
        // matter how far the clock advances — every payload must arrive.
        // With too-late drop enabled the gap is instead skipped at the
        // successor's play time (§4.6 / rule 21); see
        // `gap_skip_delivers_successor_at_its_play_time`.
        let mut s = TsbpdScheduler::new(0, TIME_BASE_I64, DELAY_MS, 0, false, None);
        // Feed packets 1 and 2 but not 0.
        s.feed_data(
            1,
            10_000,
            Duration::from_micros(TIME_BASE + 10_000 + DELAY_MS * 1000),
        );
        s.feed_data(
            2,
            20_000,
            Duration::from_micros(TIME_BASE + 20_000 + DELAY_MS * 1000),
        );
        assert!(s.has_gap());
        assert_eq!(s.buffered_count(), 2);

        // Even ticking far in the future should not deliver without packet 0.
        let outcome = s.tick(Duration::from_micros(TIME_BASE + 100_000 + DELAY_MS * 1000));
        assert!(outcome.delivered.is_empty());
        assert!(s.has_gap());
    }

    #[test]
    fn gap_skip_delivers_successor_at_its_play_time() {
        // §4.6 / rule 21: when the first buffered packet after a gap reaches
        // its play time, the missing positions before it are dropped and IT
        // is delivered — not thrown away with the gap.
        let mut s = sched(); // too-late drop enabled
        s.feed_data(1, 10_000, Duration::ZERO);
        s.feed_data(2, 20_000, Duration::ZERO);
        assert_eq!(s.buffered_count(), 2);

        let pkt1_tsbpd = TIME_BASE + 10_000 + DELAY_MS * 1000;
        let outcome = s.tick(Duration::from_micros(pkt1_tsbpd));
        assert_eq!(
            outcome.delivered,
            vec![1],
            "the successor must be delivered at its play time"
        );
        assert_eq!(
            outcome.dropped,
            vec![0],
            "the missing packet must be reported as dropped"
        );

        let pkt2_tsbpd = TIME_BASE + 20_000 + DELAY_MS * 1000;
        let outcome = s.tick(Duration::from_micros(pkt2_tsbpd));
        assert_eq!(outcome.delivered, vec![2]);
    }

    #[test]
    fn duplicate_arrival_does_not_advance_clock() {
        let mut s = sched();
        let ts = 0u32;
        let now = Duration::from_micros(TIME_BASE + DELAY_MS * 1000);
        s.feed_data(0, ts, now);
        assert_eq!(s.buffered_count(), 0); // delivered immediately

        // Same seq number again (duplicate retransmission) — should be a
        // no-op (already advanced past it).
        s.feed_data(0, ts, now);
        assert_eq!(s.buffered_count(), 0, "duplicate must not re-buffer");
        assert_eq!(s.next_release(), 1);
    }

    /// Feed `n` fresh in-sequence packets starting at `first_seq`: packet `i`
    /// (timestamp `i * 1000` µs) arrives `i * skew_us` after the instant its
    /// `TsbpdTimeBase + PKT_TIMESTAMP` predicts — a clock that runs
    /// `skew_us` per millisecond fast (positive) or slow (negative).
    fn feed_skewed_window(s: &mut TsbpdScheduler, first_seq: u32, n: u32, skew_us: i64) {
        for i in 0..n {
            let predicted = s.time_base_us() + i64::from(i) * 1000;
            let arrival = predicted + i64::from(i) * skew_us;
            s.feed_data(
                first_seq + i,
                i * 1000,
                Duration::from_micros(u64::try_from(arrival).unwrap()),
            );
        }
    }

    /// r08-SRT-W9, §4.7 rules 26-27: after one packet-count window of samples
    /// the average arrival error becomes `Drift`; the part beyond
    /// `DRIFT_MAX_US` moves the time base so `Drift` stays small. Window
    /// average here: `20 µs * (0 + 1 + ... + 999) / 1000` = 9990 µs.
    #[test]
    fn a_fast_sender_clock_is_corrected_after_one_window() {
        let mut s = TsbpdScheduler::new(0, TIME_BASE_I64, DELAY_MS, 0, false, None);
        feed_skewed_window(&mut s, 0, DRIFT_SAMPLE_COUNT, 20);
        assert_eq!(s.drift_us(), 5_000, "drift saturates at DRIFT_MAX_US");
        assert_eq!(
            s.time_base_us(),
            TIME_BASE_I64 + 4_990,
            "the 4 990 µs beyond the bound move the time base"
        );
        // The correction is applied to later packets' play time (rule 9).
        assert_eq!(
            s.pkt_tsbpd_time(0),
            u64::try_from(TIME_BASE_I64 + 4_990 + 5_000).unwrap() + DELAY_MS * 1000
        );
    }

    /// The same, for a slow clock: negative drift, negative base correction.
    #[test]
    fn a_slow_sender_clock_is_corrected_after_one_window() {
        let mut s = TsbpdScheduler::new(0, TIME_BASE_I64, DELAY_MS, 0, false, None);
        feed_skewed_window(&mut s, 0, DRIFT_SAMPLE_COUNT, -20);
        assert_eq!(s.drift_us(), -5_000);
        assert_eq!(s.time_base_us(), TIME_BASE_I64 - 4_990);
    }

    /// A small error (average 999 µs, within `DRIFT_MAX_US`) is all `Drift`,
    /// and nothing happens before the window is full.
    #[test]
    fn a_small_drift_is_applied_as_drift_only_and_only_at_the_window_boundary() {
        let mut s = TsbpdScheduler::new(0, TIME_BASE_I64, DELAY_MS, 0, false, None);
        feed_skewed_window(&mut s, 0, DRIFT_SAMPLE_COUNT - 1, 2);
        assert_eq!(s.drift_us(), 0, "999 samples is not a window yet");
        assert_eq!(s.time_base_us(), TIME_BASE_I64);
        // The 1000th sample (error 999 * 2 = 1998 µs) closes the window:
        // sum = 2 * (0 + ... + 999) = 999 000 → average 999.
        let predicted = s.time_base_us() + 999 * 1000;
        s.feed_data(
            DRIFT_SAMPLE_COUNT - 1,
            999 * 1000,
            Duration::from_micros(u64::try_from(predicted + 999 * 2).unwrap()),
        );
        assert_eq!(s.drift_us(), 999);
        assert_eq!(
            s.time_base_us(),
            TIME_BASE_I64,
            "within the bound: base untouched"
        );
    }

    /// Only a *fresh* packet (the very next sequence number) is a drift
    /// sample: a retransmission or a packet that jumped a gap arrives late for
    /// reasons that say nothing about the clocks.
    #[test]
    fn only_fresh_in_sequence_packets_are_drift_samples() {
        let mut s = TsbpdScheduler::new(0, TIME_BASE_I64, DELAY_MS, 0, false, None);
        let t = Duration::from_micros(u64::try_from(TIME_BASE_I64).unwrap());
        s.feed_data(0, 0, t);
        assert_eq!(s.drift_sample_count, 1, "the first packet is a sample");
        s.feed_data(5, 5_000, t); // jumps a gap
        assert_eq!(s.drift_sample_count, 1, "a gap-jumper is not a sample");
        s.feed_data(1, 1_000, t); // a late recovery of the gap
        assert_eq!(
            s.drift_sample_count, 1,
            "a recovered packet is not a sample"
        );
        s.feed_data(6, 6_000, t); // the next after the highest
        assert_eq!(s.drift_sample_count, 2);
    }

    /// The time base is signed (rule 12: `T_NOW - HSREQ_TIMESTAMP` is negative
    /// when the peer's clock is ahead): a negative base places the first
    /// packet's play time correctly, and saturates at "already due" rather
    /// than wrapping.
    #[test]
    fn a_negative_time_base_is_supported() {
        let s = TsbpdScheduler::new(0, -3_000_000, DELAY_MS, 0, false, None);
        assert_eq!(s.pkt_tsbpd_time(3_000_000), DELAY_MS * 1000);
        assert_eq!(
            s.pkt_tsbpd_time(0),
            0,
            "a play time before the epoch is `now`"
        );
    }

    /// r08-SRT-W8: the receiver's TLPKTDROP-enabled path across the 32-bit
    /// timestamp wrap. A packet stream whose raw timestamp wraps from near
    /// `u32::MAX` to near 0 arrives at a steady 10 ms network delay; every
    /// packet must be delivered. Without unwrapping, each post-wrap packet's
    /// play time lands ~71 minutes in the past and Too-Late Packet Drop
    /// discards it — the stream dies at the first wrap.
    #[test]
    fn a_stream_crossing_the_timestamp_wrap_is_delivered_not_dropped() {
        const PACKETS: u32 = 100;
        const START: u64 = 0xFFFF_F000; // 4096 µs before the wrap
        let mut s = TsbpdScheduler::new(0, 0, DELAY_MS, 0, true, None);
        let mut dropped = Vec::new();
        let mut last_now = 0;
        for i in 0..PACKETS {
            let true_ts = START + u64::from(i) * 1000;
            // The wire field is the true value modulo 2^32.
            let raw = u32::try_from(true_ts & 0xFFFF_FFFF).unwrap();
            last_now = true_ts + 10_000;
            let out = s.feed_data(i, raw, Duration::from_micros(last_now));
            dropped.extend(out.dropped);
        }
        let out = s.tick(Duration::from_micros(last_now + DELAY_MS * 1000));
        dropped.extend(out.dropped);
        assert!(dropped.is_empty(), "dropped across the wrap: {dropped:?}");
        assert_eq!(s.next_release(), PACKETS, "every packet was released");
        assert_eq!(s.buffered_count(), 0);
        // (Packets already due on arrival are delivered by `feed_data`; the
        // rest by the final tick. `next_release` and an empty buffer prove
        // all 100 went out in order.)
    }
}
