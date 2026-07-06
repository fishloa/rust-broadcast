//! SRT File Transfer Congestion Control (FileCC) — window-based congestion
//! control, `draft-sharabayko-srt-01` §5.2 (curated at
//! `specs/rules/srt-congestion.md`).
//!
//! This is the **file/bulk-transfer mode** sibling of [`crate::livecc::LiveCC`]
//! (§5.1, the **live/streaming mode**'s pacing-only model). Unlike LiveCC,
//! which only paces `PKT_SND_PERIOD` from a configured `MAX_BW`, FileCC is "a
//! hybrid Additive Increase Multiplicative Decrease (AIMD) algorithm"
//! (`specs/rules/srt-congestion.md` L3294-3295) that also grows/shrinks a
//! congestion window (`CWND_SIZE`), in two strictly-sequential phases: Slow
//! Start (§5.2.1.1), then Congestion Avoidance (§5.2.1.2) — see [`Phase`].
//! `PKT_SND_PERIOD` and `CWND_SIZE` are computed independently of and do not
//! touch LiveCC's state — the two controllers are additive alternatives, not
//! layered.
//!
//! ## Usage
//!
//! ```rust
//! use srt_runtime::filecc::{FileCc, Phase};
//! use core::time::Duration;
//!
//! let mut cc = FileCc::new(0);
//! assert_eq!(cc.phase(), Phase::SlowStart);
//!
//! // On each full ACK, feed the receiver-reported rate/RTT samples:
//! cc.on_ack(Duration::from_millis(10), 16, 5_000, 1_000, Duration::from_millis(50));
//!
//! // A NAK (loss report) ends slow start on the first loss (rule 4/9):
//! cc.on_loss(8, 16, 0.5);
//! assert_eq!(cc.phase(), Phase::CongestionAvoidance);
//! ```
//!
//! ## Sans-IO contract
//!
//! Like every other engine in this crate, [`FileCc`] never reads a wall
//! clock: [`FileCc::on_ack`] takes an explicit `now: Duration`; [`FileCc::on_loss`]
//! and [`FileCc::on_timeout`] are driven purely by caller-reported events.
//! [`FileCc::tick`] is provided for forward compatibility (mirrors
//! [`crate::livecc::LiveCC::tick`]) but this section defines no periodic,
//! time-only state transition beyond the three events already covered.
//!
//! ## Shared state, not redefined here
//!
//! Per `specs/rules/srt-congestion.md`'s header note, the following are
//! cross-referenced, not redefined by this module:
//! - `RC_INTERVAL` / `SYN` = 10 ms — reused from [`crate::arq::FULL_ACK_PERIOD`]
//!   (`specs/rules/srt-arq.md` rule 11), which is the same 10 ms value
//!   (L3421-3425).
//! - The initial `RTT` estimate (100 ms) — reused from
//!   [`crate::arq::rtt::INITIAL_RTT`] (`specs/rules/srt-arq.md` rule 31)
//!   before the first ACK sample is fed.
//!
//! ## Two implementation-defined gaps, flagged (not fabricated)
//!
//! `specs/rules/srt-congestion.md` explicitly flags two points the draft
//! text leaves unspecified. Both are resolved here with a documented choice:
//!
//! 1. **EWMA weight for `RECEIVING_RATE`/`EST_LINK_CAPACITY` smoothing**
//!    (gap, L3678-3681 / doc's "Gaps" section). Chosen: **this engine does
//!    not smooth them itself** — [`FileCc::on_ack`] takes `receiving_rate_pps`
//!    and `est_link_capacity_pps` as already-current values and stores them
//!    verbatim, exactly the same treatment this section already gives `RTT`
//!    (cross-ref rule/L3409-3411: "receiver-reported and sender-smoothed",
//!    with the smoothing defined *elsewhere*, not here). This keeps the
//!    engine's formulas directly hand-computable from fed inputs (no hidden
//!    internal averaging to reverse-engineer), and leaves the actual
//!    smoothing decision to the caller/sender loop — consistent with how RTT
//!    is already layered in this crate ([`crate::arq::rtt::RttEstimator`] is
//!    a separate, explicit component, not folded into the ARQ sender).
//! 2. **Packet-pairs probing mechanics** (gap, L3686-3689). Out of scope:
//!    this module consumes `RECEIVING_RATE`/`EST_LINK_CAPACITY` as inputs: it
//!    does not implement the receiver-side inter-arrival-time measurement or
//!    packet-pairs probing described in §5.2.1.3 (rules 30-37) that would
//!    *produce* those inputs on a real receiver. That measurement pipeline is
//!    receiver-side and is not part of this sender-side congestion-control
//!    state machine.
//!
//! A third, smaller implementation choice not flagged as a spec gap: the
//! `DecRandom` *distribution* (uniform in `[1, AvgNAKNum]`, clamped to 1) is
//! given (rule 23), but no source of randomness is specified. This module
//! uses a small internal, deterministic xorshift PRNG instead of pulling in
//! a `rand` dependency for a `no_std` crate — true entropy is not required
//! for correctness here, `DecRandom` only staggers repeat-decrease timing
//! across congestion periods (rule 24). `DecRandom` is rounded to the
//! nearest whole number (`FileCc::next_dec_random`, internal): Step 4's gate
//! (`NAKCount == DecCount * DecRandom`) compares two integer counters, so a
//! fractional draw makes that equality unsatisfiable after the first check.
//!
//! ## A quirk of the draft's own Step 4 pseudocode (verified, not a bug)
//!
//! `NAKCount`/`DecCount` reset to 1 at the start of a congestion period and
//! are ONLY incremented again *inside* Step 4's own conditional (rule 28) —
//! never unconditionally per NAK. So once the immediate post-reset check
//! (`1 == 1*DecRandom`) fails — i.e. whenever the drawn `DecRandom != 1` —
//! neither counter ever moves again for the rest of the period, and Step 4
//! goes silent until the next congestion period redraws `DecRandom`. This is
//! a property of the draft text as transcribed (`specs/rules/srt-congestion.md`,
//! confirmed against the actual reference implementation, `libsrt`
//! `congctl.cpp`, which uses a different formulation — `NAKCount % DecRandom
//! == 0` with both counters incrementing on every same-period NAK
//! regardless of outcome — that does not share this one-shot property).
//! Adopting libsrt's formulation would be a spec-posture departure this
//! crate is not designated for; this implementation stays literal to the
//! curated draft text. See `filecc::tests::repeat_decrease_is_a_one_shot_per_period_once_dec_random_exceeds_one`.

use core::time::Duration;

use crate::arq::FULL_ACK_PERIOD;
use crate::arq::rtt::INITIAL_RTT;
use crate::arq::seq::{seq_diff, seq_gt};

/// `RC_INTERVAL` = `SYN` = 10 ms (`specs/rules/srt-congestion.md` L3421-3425),
/// reused from [`crate::arq::FULL_ACK_PERIOD`] (same value, `srt-arq.md` rule
/// 11) rather than redefined here.
const RC_INTERVAL: Duration = FULL_ACK_PERIOD;

/// `S` — "the SRT packet size (in terms of IP payload) in bytes. SRT treats
/// 1500 bytes as a standard packet size." (`specs/rules/srt-congestion.md`
/// L3539-3540, verbatim). A different quantity from LiveCC's EWMA-derived
/// `PktSize` — see the module doc's header note.
const S_BYTES: f64 = 1500.0;

/// Slow start's fixed `PKT_SND_PERIOD` — "1 microsecond ... in order to send
/// packets as fast as possible, but not at an infinite rate"
/// (`specs/rules/srt-congestion.md` L3338-3340, verbatim; rule 6).
const SLOW_START_PKT_SND_PERIOD_US: f64 = 1.0;

/// Slow start's initial `CWND_SIZE` — 16 packets (`specs/rules/srt-congestion.md`
/// L3340-3341, rule 7).
const INITIAL_CWND_SIZE: f64 = 16.0;

/// `LastDecPeriod`'s initial value — 1 microsecond (`specs/rules/srt-congestion.md`
/// L3521-3524).
const INITIAL_LAST_DEC_PERIOD_US: f64 = 1.0;

/// Default `MAX_CWND_SIZE` — the spec suggests "the maximum receiver buffer
/// size (12 MB)" as the threshold (`specs/rules/srt-congestion.md` L3344-3345,
/// rule 8), worded as settable/recommended, not a hardwired constant.
/// Expressed here in packets at the standard packet size `S` (1500 bytes):
/// `12_000_000 / 1500 = 8000` packets. Override with [`FileCc::set_max_cwnd_size`].
const DEFAULT_MAX_CWND_SIZE: f64 = 8_000.0;

/// The NAK-tolerance loss-ratio threshold — "less than 2%"
/// (`specs/rules/srt-congestion.md` L3572-3579, rule 16).
const LOSS_RATIO_TOLERANCE: f64 = 0.02;

/// The repeat-decrease rate-backoff multiplier — `1.03`
/// (`specs/rules/srt-congestion.md` L3602-3604 / L3652-3654, rules 19 & 27).
const RATE_BACKOFF_FACTOR: f64 = 1.03;

/// The `AvgNAKNum` EWMA weights — `0.97`/`0.03`
/// (`specs/rules/srt-congestion.md` L3606-3608, rule 20, verbatim).
const AVG_NAK_NUM_OLD_WEIGHT: f64 = 0.97;
const AVG_NAK_NUM_NEW_WEIGHT: f64 = 0.03;

/// The repeat-decrease `DecCount` ceiling — decreases stop once `DecCount`
/// exceeds 5 within a congestion period (`specs/rules/srt-congestion.md`
/// L3652-3658, rule "Step 4").
const MAX_DEC_COUNT: u32 = 5;

/// `inc`'s multiplier in the Step 5 rate-increase formula (`specs/rules/srt-congestion.md`
/// L3498-3517, verbatim: `0.0000015`).
const INC_SCALE: f64 = 0.0000015;

/// Microseconds per second — the units conversion every `PKT_SND_PERIOD`/rate
/// formula in this module divides or multiplies by (`specs/rules/srt-congestion.md`,
/// recurring throughout §5.2.1.2's Steps 3/5/6, sourced from the same
/// L3498-3517 code block as [`INC_SCALE`]).
const US_PER_SEC: f64 = 1_000_000.0;

/// `lossBandwidth`'s doubling factor in the Step 5 rate-increase formula
/// (`specs/rules/srt-congestion.md` L3498-3517 code block, verbatim:
/// `lossBandwidth = 2 * (1000000 / LastDecPeriod)`).
const LOSS_BANDWIDTH_FACTOR: f64 = 2.0;

/// The `linkCapacity / 9` clamp divisor in the Step 5 rate-increase formula
/// (`specs/rules/srt-congestion.md` L3498-3517 code block, verbatim:
/// `if (... && (linkCapacity / 9) < B) B = linkCapacity / 9;`) — a
/// spec-mandated tuning constant, not independently derived.
const LINK_CAPACITY_CLAMP_DIVISOR: f64 = 9.0;

/// Bits per byte, used to convert `S` (packet size in bytes) to bits in the
/// Step 5 rate-increase formula (`specs/rules/srt-congestion.md` L3498-3517
/// code block, verbatim: `inc = pow(10.0, ceil(log10(B * S * 8))) * 0.0000015 / S`).
const BITS_PER_BYTE: f64 = 8.0;

/// FileCC algorithm phase (`specs/rules/srt-congestion.md` rules 4-5). Slow
/// Start (§5.2.1.1) runs exactly once at the start of a connection; it
/// transitions to Congestion Avoidance (§5.2.1.2) on the first loss,
/// `CWND_SIZE` exceeding its maximum, or a timeout — and never transitions
/// back (L3323-3326).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Phase {
    /// §5.2.1.1 — probes for available bandwidth; runs exactly once.
    SlowStart,
    /// §5.2.1.2 — entered once slow start ends; the steady-state AIMD phase.
    CongestionAvoidance,
}

impl Phase {
    /// The §5.2 phase name.
    pub fn name(&self) -> &'static str {
        match self {
            Phase::SlowStart => "SlowStart",
            Phase::CongestionAvoidance => "CongestionAvoidance",
        }
    }
}

broadcast_common::impl_spec_display!(Phase);

/// A minimal, seedable xorshift64* PRNG — see the module doc's third
/// implementation-choice note (`DecRandom`'s source of randomness is
/// unspecified by the draft; only its distribution is given, rule 23).
#[derive(Debug, Clone, Copy)]
struct XorShift64(u64);

impl XorShift64 {
    /// A fixed, non-zero default seed. Deterministic on purpose: reproducible
    /// test/debug behavior, and true entropy is not required for correctness
    /// (see the module doc).
    const DEFAULT_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

    fn new(seed: u64) -> Self {
        XorShift64(if seed == 0 { Self::DEFAULT_SEED } else { seed })
    }

    /// Next pseudo-random value, uniform in `[0.0, 1.0)`.
    fn next_unit(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        let r = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        // Top 53 bits as a double in [0, 1).
        (r >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }
}

/// `min(a, b)` for `f64`, without relying on `f64::min` (kept as a plain
/// comparison so this module has no dependency on any transcendental-math
/// support — see [`next_power_of_10`]'s doc for why that matters for the
/// `no_std` build).
fn fmin(a: f64, b: f64) -> f64 {
    if a < b { a } else { b }
}

/// `max(a, b)` for `f64` — see [`fmin`].
fn fmax(a: f64, b: f64) -> f64 {
    if a > b { a } else { b }
}

/// Round-half-away-from-zero for `f64`, matching `f64::round`'s semantics —
/// without calling it, since `f64::round` is a `std`-only method (`core`
/// float types don't expose it; see [`fmin`]'s doc for why this module avoids
/// any transcendental/libm dependency). `as i64` float-to-int casts are
/// saturating in Rust, so this is safe for any finite input in `i64` range —
/// `DecRandom` values are always small and positive.
fn fround(x: f64) -> f64 {
    let truncated = x as i64 as f64;
    let diff = x - truncated;
    if diff >= 0.5 {
        truncated + 1.0
    } else if diff <= -0.5 {
        truncated - 1.0
    } else {
        truncated
    }
}

/// `pow(10.0, ceil(log10(x)))` for `x > 0` — the smallest power of 10 that is
/// `>= x` (`specs/rules/srt-congestion.md` Step 5, L3506, verbatim source:
/// `pow(10.0, ceil(log10(B * S * 8))) * 0.0000015 / S`).
///
/// Implemented via repeated multiplication/division instead of calling
/// `log10`/`powf` directly: those are `libm` functions not available in
/// `core` (this crate builds `no_std`, including on `thumbv7em-none-eabi`
/// with no libm), whereas this loop only uses basic arithmetic and
/// comparisons. Algebraically identical to the spec's formula for `x > 0`.
fn next_power_of_10(x: f64) -> f64 {
    debug_assert!(x > 0.0, "next_power_of_10 is only defined for x > 0");
    let mut p = 1.0_f64;
    while p < x {
        p *= 10.0;
    }
    while p / 10.0 >= x {
        p /= 10.0;
    }
    p
}

/// SRT File Transfer Congestion Control — sender-side window + pacing state
/// (`draft-sharabayko-srt-01` §5.2). See the module doc for the full formula
/// mapping, the sans-IO contract, and the two flagged spec gaps.
#[derive(Debug, Clone)]
pub struct FileCc {
    phase: Phase,
    /// `CWND_SIZE`, in packets (rule 7; Step 3 of both phases).
    cwnd_size: f64,
    /// `MAX_CWND_SIZE` (rule 8) — slow start ends once `cwnd_size` exceeds
    /// this.
    max_cwnd_size: f64,
    /// `PKT_SND_PERIOD`, in microseconds.
    pkt_snd_period_us: f64,
    /// `LastRCTime` — `None` until the first ACK (rate-control gate always
    /// passes on the very first call; the draft does not give an initial
    /// value, this is the implementation-defined resolution).
    last_rc_time: Option<Duration>,
    /// `LAST_ACK_SEQNO` — initialized to the caller-supplied `initial_seqno`
    /// (representing "no data packet acknowledged yet"; the draft does not
    /// give LAST_ACK_SEQNO's initial value either).
    last_ack_seqno: u32,
    /// `bLoss` — initial value `False` (rule 12).
    b_loss: bool,
    /// `LastDecPeriod`, in microseconds — initial value 1 microsecond
    /// (item under Step 5).
    last_dec_period_us: f64,
    /// `LastDecSeq` — `None` until the first Congestion-Avoidance-phase NAK
    /// (so the very first CA-phase loss always starts a new congestion
    /// period, rule 25).
    last_dec_seq: Option<u32>,
    /// `AvgNAKNum` — initial value 0 (rule "State variables", verbatim def).
    avg_nak_num: f64,
    /// `NAKCount` — initial value 0.
    nak_count: u32,
    /// `DecCount` — initial value 0.
    dec_count: u32,
    /// `DecRandom` — computed per congestion period (rule 23).
    dec_random: f64,
    /// `RECEIVING_RATE`, packets/sec — see the module doc's gap-1 resolution
    /// (stored verbatim from [`FileCc::on_ack`], not smoothed here).
    receiving_rate_pps: u64,
    /// `EST_LINK_CAPACITY`, packets/sec — see the module doc's gap-1
    /// resolution.
    est_link_capacity_pps: u64,
    /// `RTT`, sender-smoothed elsewhere (`crate::arq::rtt::RttEstimator`) and
    /// fed in via [`FileCc::on_ack`]. Initialized to
    /// [`crate::arq::rtt::INITIAL_RTT`] before the first ACK.
    rtt: Duration,
    /// `MAX_BW`, bytes/sec — Step 6 clamp (rule 15: file transfer only uses
    /// `MAXBW_SET`; `None` means unbounded, the default for file transfer).
    max_bw_bytes_per_sec: Option<u64>,
    rng: XorShift64,
}

impl FileCc {
    /// A fresh FileCC engine in Slow Start (rule 4: "runs exactly once at
    /// the beginning of a connection").
    ///
    /// `initial_seqno` seeds `LAST_ACK_SEQNO` (see the field doc) — pass the
    /// connection's initial sequence number (ISN) minus one, or the ISN
    /// itself if no data has been sent yet; either way the first ACK's
    /// `CWND_SIZE` growth (Step 3) reflects exactly the packets actually
    /// acknowledged since then.
    pub fn new(initial_seqno: u32) -> Self {
        FileCc {
            phase: Phase::SlowStart,
            cwnd_size: INITIAL_CWND_SIZE,
            max_cwnd_size: DEFAULT_MAX_CWND_SIZE,
            pkt_snd_period_us: SLOW_START_PKT_SND_PERIOD_US,
            last_rc_time: None,
            last_ack_seqno: initial_seqno,
            b_loss: false,
            last_dec_period_us: INITIAL_LAST_DEC_PERIOD_US,
            last_dec_seq: None,
            avg_nak_num: 0.0,
            nak_count: 0,
            dec_count: 0,
            dec_random: 1.0,
            receiving_rate_pps: 0,
            est_link_capacity_pps: 0,
            rtt: INITIAL_RTT,
            max_bw_bytes_per_sec: None,
            rng: XorShift64::new(XorShift64::DEFAULT_SEED),
        }
    }

    /// The current algorithm phase.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The current `CWND_SIZE`, in packets.
    pub fn cwnd_size(&self) -> f64 {
        self.cwnd_size
    }

    /// The current `PKT_SND_PERIOD`, in microseconds (as an `f64` — the
    /// Congestion Avoidance formulas are inherently fractional).
    pub fn pkt_snd_period_us(&self) -> f64 {
        self.pkt_snd_period_us
    }

    /// The current `PKT_SND_PERIOD` as a [`Duration`], for a sender to
    /// consult before transmitting the next packet (same role as
    /// [`crate::livecc::LiveCC::on_ack_received`]'s return value).
    ///
    /// Truncates the microsecond value toward zero (consistent with this
    /// crate's existing integer-truncation convention, e.g. LiveCC's
    /// `PKT_SND_PERIOD`).
    pub fn pkt_snd_period(&self) -> Duration {
        Duration::from_micros(self.pkt_snd_period_us as u64)
    }

    /// The current `MAX_CWND_SIZE` (rule 8), in packets.
    pub fn max_cwnd_size(&self) -> f64 {
        self.max_cwnd_size
    }

    /// Reconfigure `MAX_CWND_SIZE` — rule 8 calls the 12 MB-derived default
    /// a settable/recommended value, not a hardwired constant.
    pub fn set_max_cwnd_size(&mut self, packets: f64) {
        self.max_cwnd_size = packets;
    }

    /// The current `MAX_BW` clamp, in bytes/sec (`None` = unbounded).
    pub fn max_bw_bytes_per_sec(&self) -> Option<u64> {
        self.max_bw_bytes_per_sec
    }

    /// Reconfigure `MAX_BW` (rule 15: `MAXBW_SET` mode only applies to file
    /// transfer; there is no default, `None` is unbounded).
    pub fn set_max_bw_bytes_per_sec(&mut self, max_bw: Option<u64>) {
        self.max_bw_bytes_per_sec = max_bw;
    }

    /// The last `RECEIVING_RATE` fed via [`FileCc::on_ack`], packets/sec.
    pub fn receiving_rate_pps(&self) -> u64 {
        self.receiving_rate_pps
    }

    /// The last `EST_LINK_CAPACITY` fed via [`FileCc::on_ack`], packets/sec.
    pub fn est_link_capacity_pps(&self) -> u64 {
        self.est_link_capacity_pps
    }

    /// The last `RTT` fed via [`FileCc::on_ack`] (or the initial 100 ms
    /// default before the first ACK).
    pub fn rtt(&self) -> Duration {
        self.rtt
    }

    /// `bLoss` — `true` if a loss has been reported since the last rate
    /// increase (rule 11-12).
    pub fn b_loss(&self) -> bool {
        self.b_loss
    }

    /// `AvgNAKNum` — the average number of NAKs per congestion period
    /// (rule 20's EWMA).
    pub fn avg_nak_num(&self) -> f64 {
        self.avg_nak_num
    }

    /// `NAKCount` — NAKs received so far in the current congestion period.
    pub fn nak_count(&self) -> u32 {
        self.nak_count
    }

    /// `DecCount` — rate decreases applied so far in the current congestion
    /// period.
    pub fn dec_count(&self) -> u32 {
        self.dec_count
    }

    /// `LastDecSeq` — the largest sent sequence number at the last rate
    /// decrease / congestion-period boundary, or `None` if no
    /// Congestion-Avoidance-phase loss has occurred yet.
    pub fn last_dec_seq(&self) -> Option<u32> {
        self.last_dec_seq
    }

    /// `LastDecPeriod`, in microseconds (item under Step 5; initial value 1
    /// microsecond).
    pub fn last_dec_period_us(&self) -> f64 {
        self.last_dec_period_us
    }

    /// On full-ACK packet reception (`specs/rules/srt-congestion.md`,
    /// §5.2.1.1 "(1) On ACK packet reception" / §5.2.1.2 "(1) On ACK packet
    /// reception"). Only full ACKs trigger a rate increase (rule 3,
    /// L3313-3314) — a light ACK must not be passed to this method.
    ///
    /// `now` — the current time (Step 1's `currTime`).
    /// `ack_seqno` — the ACK's acknowledged sequence number (`ACK_SEQNO`).
    /// `receiving_rate_pps` / `est_link_capacity_pps` — the ACK-carried,
    /// receiver-reported rate estimates (§5.2.1.3); see the module doc's
    /// gap-1 resolution for why these are stored verbatim, not smoothed
    /// here.
    /// `rtt` — the current (already-smoothed elsewhere) RTT estimate.
    pub fn on_ack(
        &mut self,
        now: Duration,
        ack_seqno: u32,
        receiving_rate_pps: u64,
        est_link_capacity_pps: u64,
        rtt: Duration,
    ) {
        self.receiving_rate_pps = receiving_rate_pps;
        self.est_link_capacity_pps = est_link_capacity_pps;
        self.rtt = rtt;

        // Step 1 (identical gate in both phases, L3365-3371 / L3455-3461):
        // if (currTime - LastRCTime < RC_INTERVAL) { keep; stop; }
        if let Some(last) = self.last_rc_time {
            if now.saturating_sub(last) < RC_INTERVAL {
                return;
            }
        }
        // Step 2: LastRCTime = currTime.
        self.last_rc_time = Some(now);

        match self.phase {
            Phase::SlowStart => self.on_ack_slow_start(ack_seqno),
            Phase::CongestionAvoidance => self.on_ack_congestion_avoidance(),
        }
    }

    /// Slow start's ACK handling, Steps 3-5 (L3381-3401).
    fn on_ack_slow_start(&mut self, ack_seqno: u32) {
        // Step 3: CWND_SIZE += ACK_SEQNO - LAST_ACK_SEQNO (wrap-safe delta —
        // reusing arq::seq, not redefined here, per the crate's existing
        // sequence-arithmetic convention).
        let delta = seq_diff(ack_seqno, self.last_ack_seqno);
        self.cwnd_size += f64::from(delta);
        // Step 4: LAST_ACK_SEQNO = ACK_SEQNO.
        self.last_ack_seqno = ack_seqno;

        // Step 5: CWND_SIZE exceeding MAX_CWND_SIZE ends slow start.
        if self.cwnd_size > self.max_cwnd_size {
            self.end_slow_start();
        }
    }

    /// Congestion Avoidance's ACK handling, Steps 3-6 (L3479-3559).
    fn on_ack_congestion_avoidance(&mut self) {
        // Step 3: CWND_SIZE = RECEIVING_RATE*(RTT+RC_INTERVAL)/1000000 + 16
        // (recomputed directly, not incrementally, unlike slow start).
        self.cwnd_size = self.receiving_rate_pps as f64 * self.rtt_plus_rc_interval_us()
            / US_PER_SEC
            + INITIAL_CWND_SIZE;

        // Step 4: loss-in-flight guard.
        if self.b_loss {
            self.b_loss = false;
            return;
        }

        // Step 5: rate-increase formula (L3498-3517, verbatim).
        let loss_bandwidth = LOSS_BANDWIDTH_FACTOR * (US_PER_SEC / self.last_dec_period_us);
        let link_capacity = fmin(loss_bandwidth, self.est_link_capacity_pps as f64);
        let mut b = link_capacity - US_PER_SEC / self.pkt_snd_period_us;
        if self.pkt_snd_period_us > self.last_dec_period_us
            && (link_capacity / LINK_CAPACITY_CLAMP_DIVISOR) < b
        {
            b = link_capacity / LINK_CAPACITY_CLAMP_DIVISOR;
        }
        let inc = if b <= 0.0 {
            1.0 / S_BYTES
        } else {
            let raw = next_power_of_10(b * S_BYTES * BITS_PER_BYTE) * INC_SCALE / S_BYTES;
            fmax(raw, 1.0 / S_BYTES)
        };
        let rc_interval_us = RC_INTERVAL.as_micros() as f64;
        self.pkt_snd_period_us = (self.pkt_snd_period_us * rc_interval_us)
            / (self.pkt_snd_period_us * inc + rc_interval_us);

        // Step 6: MAX_BW clamp, if configured (rule 15).
        if let Some(max_bw) = self.max_bw_bytes_per_sec {
            let min_period_us = US_PER_SEC / (max_bw as f64 / S_BYTES);
            if self.pkt_snd_period_us < min_period_us {
                self.pkt_snd_period_us = min_period_us;
            }
        }
    }

    /// `RTT + RC_INTERVAL`, in microseconds — shared by the CWND_SIZE
    /// formula (Step 3, both event handlers that use it).
    fn rtt_plus_rc_interval_us(&self) -> f64 {
        self.rtt.as_micros() as f64 + RC_INTERVAL.as_micros() as f64
    }

    /// Ends slow start, computing `PKT_SND_PERIOD` per the shared Step 5
    /// formula (L3392-3401, reused verbatim by rules 9 and 10 — NAK and RTO
    /// during slow start).
    fn end_slow_start(&mut self) {
        self.phase = Phase::CongestionAvoidance;
        self.pkt_snd_period_us = if self.receiving_rate_pps > 0 {
            US_PER_SEC / self.receiving_rate_pps as f64
        } else {
            self.cwnd_size / self.rtt_plus_rc_interval_us()
        };
    }

    /// On a loss report (NAK) packet reception.
    ///
    /// - During Slow Start (rule 9): ends slow start; `PKT_SND_PERIOD` is
    ///   set exactly as in the ACK Step 5 formula. `lost_seqno`,
    ///   `largest_sent_seqno`, and `loss_ratio` are not consulted by this
    ///   phase's handling (the draft's rule 9 gives no further formula).
    /// - During Congestion Avoidance (§5.2.1.2 "(2)", L3568-3658): runs the
    ///   full bLoss/loss-ratio-tolerance/congestion-period/repeat-decrease
    ///   state machine.
    ///
    /// `lost_seqno` — the sequence number reported lost by this NAK.
    /// `largest_sent_seqno` — the largest sequence number sent so far
    /// (recorded as the new `LastDecSeq` on a decrease, rules 22/29).
    /// `loss_ratio` — the sender's current estimated loss ratio (rule 16's
    /// "less than 2%" tolerance check), e.g. lost/sent over a recent window.
    pub fn on_loss(&mut self, lost_seqno: u32, largest_sent_seqno: u32, loss_ratio: f64) {
        match self.phase {
            Phase::SlowStart => self.end_slow_start(),
            Phase::CongestionAvoidance => {
                self.on_loss_congestion_avoidance(lost_seqno, largest_sent_seqno, loss_ratio)
            }
        }
    }

    /// Congestion Avoidance's NAK handling (L3568-3658).
    fn on_loss_congestion_avoidance(
        &mut self,
        lost_seqno: u32,
        largest_sent_seqno: u32,
        loss_ratio: f64,
    ) {
        // Step 1: bLoss = True.
        self.b_loss = true;

        // Step 2: loss-ratio tolerance (rule 16-17).
        if loss_ratio < LOSS_RATIO_TOLERANCE {
            self.last_dec_period_us = self.pkt_snd_period_us;
            return;
        }

        // Step 3: new congestion period? (rule 25: the lost seq is greater
        // than LastDecSeq).
        let is_new_period = match self.last_dec_seq {
            None => true,
            Some(last) => seq_gt(lost_seqno, last),
        };

        if is_new_period {
            self.last_dec_period_us = self.pkt_snd_period_us;
            self.pkt_snd_period_us *= RATE_BACKOFF_FACTOR;
            // rule 20: AvgNAKNum = 0.97*AvgNAKNum + 0.03*NAKCount (using the
            // just-finished period's NAKCount, before it is reset below).
            self.avg_nak_num = AVG_NAK_NUM_OLD_WEIGHT * self.avg_nak_num
                + AVG_NAK_NUM_NEW_WEIGHT * self.nak_count as f64;
            self.nak_count = 1;
            self.dec_count = 1;
            self.last_dec_seq = Some(largest_sent_seqno);
            self.dec_random = self.next_dec_random();
            return;
        }

        // Step 4: repeat decrease within the same congestion period
        // (rule "Step 4": DecCount<=5 && NAKCount==DecCount*DecRandom).
        if self.dec_count <= MAX_DEC_COUNT
            && self.nak_count as f64 == self.dec_count as f64 * self.dec_random
        {
            self.pkt_snd_period_us *= RATE_BACKOFF_FACTOR;
            self.dec_count += 1;
            self.nak_count += 1;
            self.last_dec_seq = Some(largest_sent_seqno);
        }
    }

    /// `DecRandom` — "a random number between 1 and the average number of
    /// NAKs per congestion period (AvgNAKNum)", clamped to 1 if the draw is
    /// below 1 (rule 23). See the module doc for the PRNG source note.
    ///
    /// Rounded to the nearest whole number: Step 4's repeat-decrease gate
    /// (`NAKCount == DecCount * DecRandom`) compares against the integer
    /// counters `NAKCount`/`DecCount`, so it only means "every `DecRandom`-th
    /// NAK" — and is satisfiable — when `DecRandom` is itself integer-valued.
    /// A raw fractional draw (`AvgNAKNum > 1`) makes that equality a
    /// measure-zero float comparison that (almost) never holds again after
    /// the congestion period's first decrease, silently disabling repeat
    /// decrease for the rest of the period.
    fn next_dec_random(&mut self) -> f64 {
        let hi = fmax(self.avg_nak_num, 1.0);
        let r = self.rng.next_unit();
        let val = fround(1.0 + r * (hi - 1.0));
        fmax(val, 1.0)
    }

    /// On a retransmission timeout (RTO) event.
    ///
    /// During Slow Start (rule 10): ends slow start, `PKT_SND_PERIOD` set
    /// exactly as in the ACK Step 5 formula (same as [`FileCc::on_loss`]'s
    /// slow-start handling). Once in Congestion Avoidance, this section does
    /// not describe an RTO-specific FileCC reaction (RTO-driven
    /// retransmission itself is the ARQ engine's concern, `srt-arq.md`) —
    /// calling this while already in Congestion Avoidance is a no-op.
    pub fn on_timeout(&mut self) {
        if self.phase == Phase::SlowStart {
            self.end_slow_start();
        }
    }

    /// Time-driven tick — currently a no-op (mirrors
    /// [`crate::livecc::LiveCC::tick`]). Provided for forward compatibility:
    /// this section's algorithm is driven entirely by the three named
    /// events (send / ACK / timeout, rule 5); there is no additional
    /// periodic-only state transition to run here.
    pub fn tick(&mut self, _now: Duration) {}
}

impl Default for FileCc {
    /// A fresh engine with `initial_seqno = 0` (see [`FileCc::new`]).
    fn default() -> Self {
        FileCc::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_in_slow_start_with_spec_initial_values() {
        let cc = FileCc::new(0);
        assert_eq!(cc.phase(), Phase::SlowStart);
        assert_eq!(cc.cwnd_size(), 16.0);
        assert_eq!(cc.pkt_snd_period_us(), 1.0);
        assert_eq!(cc.max_cwnd_size(), 8_000.0);
        assert!(!cc.b_loss());
        assert_eq!(cc.avg_nak_num(), 0.0);
        assert_eq!(cc.nak_count(), 0);
        assert_eq!(cc.dec_count(), 0);
        assert_eq!(cc.last_dec_seq(), None);
        assert_eq!(cc.last_dec_period_us(), 1.0);
        assert_eq!(cc.rtt(), Duration::from_millis(100));
        assert_eq!(cc.max_bw_bytes_per_sec(), None);
    }

    #[test]
    fn slow_start_cwnd_grows_by_ack_seqno_delta() {
        let mut cc = FileCc::new(0);
        cc.on_ack(
            Duration::from_millis(10),
            5,
            0,
            0,
            Duration::from_millis(100),
        );
        assert_eq!(cc.cwnd_size(), 21.0); // 16 + (5 - 0)
        assert_eq!(
            cc.pkt_snd_period_us(),
            1.0,
            "fixed at 1us during slow start"
        );
    }

    #[test]
    fn rate_control_gate_blocks_updates_within_rc_interval() {
        let mut cc = FileCc::new(0);
        cc.on_ack(
            Duration::from_millis(10),
            5,
            0,
            0,
            Duration::from_millis(100),
        );
        assert_eq!(cc.cwnd_size(), 21.0);
        // Only 1ms later (< RC_INTERVAL = 10ms): must be a no-op.
        cc.on_ack(
            Duration::from_millis(11),
            999,
            0,
            0,
            Duration::from_millis(100),
        );
        assert_eq!(
            cc.cwnd_size(),
            21.0,
            "gate must block an ACK inside RC_INTERVAL"
        );
    }

    #[test]
    fn loss_during_slow_start_transitions_to_congestion_avoidance() {
        let mut cc = FileCc::new(0);
        assert_eq!(cc.phase(), Phase::SlowStart);
        cc.on_loss(10, 10, 0.9);
        assert_eq!(cc.phase(), Phase::CongestionAvoidance);
    }

    #[test]
    fn timeout_during_slow_start_transitions_to_congestion_avoidance() {
        let mut cc = FileCc::new(0);
        cc.on_timeout();
        assert_eq!(cc.phase(), Phase::CongestionAvoidance);
    }

    #[test]
    fn timeout_during_congestion_avoidance_is_a_no_op() {
        let mut cc = FileCc::new(0);
        cc.on_loss(10, 10, 0.9);
        assert_eq!(cc.phase(), Phase::CongestionAvoidance);
        let period = cc.pkt_snd_period_us();
        cc.on_timeout();
        assert_eq!(cc.pkt_snd_period_us(), period);
    }

    #[test]
    fn cwnd_exceeding_max_ends_slow_start() {
        let mut cc = FileCc::new(0);
        cc.set_max_cwnd_size(20.0);
        cc.on_ack(
            Duration::from_millis(10),
            10,
            0,
            0,
            Duration::from_millis(100),
        );
        // CWND = 16 + 10 = 26 > 20 -> slow start ends.
        assert_eq!(cc.phase(), Phase::CongestionAvoidance);
    }

    #[test]
    fn max_bw_clamp_floors_pkt_snd_period() {
        let mut cc = FileCc::new(0);
        cc.on_loss(1, 1, 0.9); // -> Congestion Avoidance
        cc.set_max_bw_bytes_per_sec(Some(1)); // absurdly low MAX_BW
        cc.on_ack(
            Duration::from_millis(10),
            1,
            1_000,
            1_000,
            Duration::from_millis(50),
        );
        // MIN_PERIOD = 1_000_000 / (1 / 1500) = 1_500_000_000 us.
        assert!(cc.pkt_snd_period_us() >= 1_500_000_000.0);
    }

    #[test]
    fn next_power_of_10_matches_known_values() {
        assert_eq!(next_power_of_10(1.0), 1.0);
        assert_eq!(next_power_of_10(9.9), 10.0);
        assert_eq!(next_power_of_10(10.0), 10.0);
        assert_eq!(next_power_of_10(10.1), 100.0);
        assert_eq!(next_power_of_10(100.0), 100.0);
        assert_eq!(next_power_of_10(0.05), 0.1);
    }

    #[test]
    fn xorshift_produces_values_in_unit_range() {
        let mut rng = XorShift64::new(1);
        for _ in 0..100 {
            let v = rng.next_unit();
            assert!((0.0..1.0).contains(&v), "value out of range: {v}");
        }
    }

    #[test]
    fn dec_random_is_one_when_avg_nak_num_is_zero() {
        let mut cc = FileCc::new(0);
        assert_eq!(cc.avg_nak_num(), 0.0);
        assert_eq!(cc.next_dec_random(), 1.0);
    }

    /// `DecRandom` must be integer-valued once `AvgNAKNum > 1` (a fractional
    /// draw makes Step 4's `NAKCount == DecCount * DecRandom` gate a
    /// measure-zero float comparison that — after the congestion period's
    /// first decrease — essentially never holds again, silently disabling
    /// repeat decrease for the rest of the period). This is the bug a
    /// pre-tag audit found: the un-rounded draw only ever produced an
    /// integer in the degenerate `AvgNAKNum <= 1` case, which is the only
    /// case the pre-existing test suite exercised.
    #[test]
    fn dec_random_is_integer_valued_once_avg_nak_num_exceeds_one() {
        let mut cc = FileCc::new(0);
        cc.avg_nak_num = 6.0;
        for _ in 0..200 {
            let v = cc.next_dec_random();
            assert_eq!(
                v,
                v.round(),
                "DecRandom must be a whole number (got {v}) so Step 4's \
                 NAKCount == DecCount * DecRandom gate is actually satisfiable"
            );
            assert!((1.0..=6.0).contains(&v), "DecRandom out of range: {v}");
        }
    }

    /// Documents a genuine quirk of the draft's own literal Step 4
    /// pseudocode (verified against `specs/rules/srt-congestion.md`, not
    /// assumed): `NAKCount`/`DecCount` are reset to 1 on a new congestion
    /// period and are ONLY ever touched again *inside* Step 4's own
    /// `NAKCount == DecCount * DecRandom` conditional (rule 28: "Increase
    /// DecCount and NAKCount each by 1" — inside the `if`, not unconditional
    /// per-NAK). So the very next same-period NAK checks `1 == 1*DecRandom`;
    /// if `DecRandom != 1` that fails, and since neither counter has moved,
    /// the SAME false check repeats for every subsequent NAK in the period —
    /// Step 4 cannot fire again until the next congestion period redraws
    /// `DecRandom`. This is a property of the literal spec text as
    /// transcribed, not a Rust-side bug: the reference implementation
    /// (libsrt `congctl.cpp`) uses a different formulation (`NAKCount %
    /// DecRandom == 0`, incrementing both counters on every same-period NAK
    /// regardless of the check's outcome) that does NOT have this one-shot
    /// property — but adopting that would be a spec-posture departure this
    /// crate isn't designated for (see `CLAUDE.md`), so this implementation
    /// stays literal to the curated draft text. Confirmed here with a fixed
    /// `dec_random` (bypassing the RNG) so the assertion is deterministic.
    #[test]
    fn repeat_decrease_is_a_one_shot_per_period_once_dec_random_exceeds_one() {
        let mut cc = FileCc::new(0);
        cc.on_loss(10, 10, 0.9); // end slow start
        cc.on_loss(20, 20, 0.9); // new congestion period, big decrease
        assert_eq!(cc.dec_count(), 1);

        // Force a specific integer DecRandom > 1 for this period (bypassing
        // the RNG so the check below is deterministic, not seed-dependent).
        cc.dec_random = 3.0;
        let after_first = cc.pkt_snd_period_us();

        // Many same-period NAKs: per the literal spec pseudocode, NONE of
        // them can fire Step 4 again, because NAKCount/DecCount are frozen
        // at (1, 1) and 1 == 1*3 is false — and stays false forever, since
        // nothing outside Step 4's own conditional advances either counter.
        for sent in 21..=60u32 {
            cc.on_loss(15, sent, 0.9);
        }

        assert_eq!(
            cc.dec_count(),
            1,
            "with the literal spec's Step 4 pseudocode, DecCount must stay \
             frozen at 1 for the rest of the period once the immediate \
             post-reset check (NAKCount==DecCount*DecRandom, i.e. 1==1*3) \
             fails — this is the draft's own one-shot property, not a bug"
        );
        assert_eq!(
            cc.pkt_snd_period_us(),
            after_first,
            "PKT_SND_PERIOD must not change further once Step 4 has gone \
             silent for the rest of the period"
        );
    }

    /// The complementary, working case: when the drawn integer `DecRandom`
    /// rounds to exactly `1`, the immediate post-reset check `1 == 1*1`
    /// holds, so Step 4 fires on every subsequent same-period NAK (this is
    /// the case the pre-existing integration test
    /// `repeated_decrease_backs_off_by_1_03_bounded_by_dec_count` already
    /// covers end-to-end with `AvgNAKNum == 0`; this unit test additionally
    /// proves it holds even when `AvgNAKNum` is nonzero but rounds to 1).
    #[test]
    fn repeat_decrease_fires_repeatedly_when_dec_random_rounds_to_one() {
        let mut cc = FileCc::new(0);
        cc.avg_nak_num = 1.2; // rounds to 1 via next_dec_random's `.round()`
        cc.on_loss(10, 10, 0.9);
        cc.on_loss(20, 20, 0.9);
        assert_eq!(cc.dec_random, 1.0);
        assert_eq!(cc.dec_count(), 1);

        for sent in 21..=25u32 {
            cc.on_loss(15, sent, 0.9);
        }
        assert_eq!(
            cc.dec_count(),
            6,
            "DecRandom==1 must let Step 4 fire on every same-period NAK"
        );
    }

    #[test]
    fn default_matches_new_zero() {
        let a = FileCc::default();
        let b = FileCc::new(0);
        assert_eq!(a.cwnd_size(), b.cwnd_size());
        assert_eq!(a.phase(), b.phase());
    }
}
