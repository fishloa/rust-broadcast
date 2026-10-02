//! SRT ARQ (Automatic Repeat reQuest) reliability engine —
//! `draft-sharabayko-srt-01` §4.8 (Acknowledgement and Lost Packet
//! Handling), §4.8.1 (Packet Acknowledgement — ACKs, ACKACKs), §4.8.2
//! (Packet Retransmission — NAKs), §4.10 (Round-Trip Time Estimation).
//! Curated behavioural rules: `specs/rules/srt-arq.md`. The wire field
//! layouts this module drives (ACK/NAK/ACKACK) are the existing
//! [`crate::packet`] codecs — this module never re-encodes them, it only
//! decides *when* to build one and *what* to do with one received.
//!
//! Sans-IO, like the rest of this crate: [`Sender`] and [`Receiver`] never
//! read a wall clock. All timing is driven by a caller-supplied
//! `now: core::time::Duration` (elapsed time since a fixed epoch the caller
//! owns) passed to `tick`/`feed_data`/`on_data`/`on_ack`/`on_ackack`.
//!
//! # Module map
//! - [`seq`] — wrap-safe 31-bit sequence-number arithmetic (not itself a
//!   `srt-arq.md` rule — the draft does not specify a comparison algorithm;
//!   see the module doc there for the resolution).
//! - [`rtt::RttEstimator`] — the rule 29-31 RTT/RTTVar EWMA, shared by both
//!   roles (rule 34: a socket's RTT state is really one estimator usable by
//!   both a sender and a receiver path).
//! - [`Sender`] — send buffer, NAK-driven retransmit queue, ACK/ACKACK
//!   handling (rules 1, 3, 5, 7-10, 15-18, 23-24, 33).
//! - [`Receiver`] — loss detection, Full/Light ACK generation, periodic NAK,
//!   ACKACK-driven RTT measurement (rules 4, 8, 11-14, 21-22, 26-30, 32).
//!
//! # Non-goals (explicit follow-ups, not curated as ARQ rules here)
//! - Deciding *when* to apply the TLPKTDROP fake-ACK skip (rule 13) is
//!   `srt-tsbpd.md` scope: the receiver can skip a range, the integration layer
//!   (the tokio adapter) decides when, and only if TLPKTDROP was negotiated.
//! - RTO-based periodic retransmission without a NAK / congestion control
//!   (§5, FileCC) — srt-arq.md is explicit that the RTO formula is
//!   congestion-control scope, not curated there.
//! - Send-queue overflow / unsent-packet drop sizing (rules 19-20) — the
//!   latency-window math is §4.4/§4.5 scope.

mod receiver;
pub mod rtt;
mod sender;
pub mod seq;

pub use receiver::{FeedOutcome, Receiver};
pub use rtt::RttEstimator;
pub use sender::Sender;

use core::time::Duration;

/// Full ACK timer period — 10 milliseconds (`specs/rules/srt-arq.md` rule
/// 11, quoting `draft-sharabayko-srt-01` L2874-2876: "the ACK period or
/// synchronization time interval SYN").
pub const FULL_ACK_PERIOD: Duration = Duration::from_millis(10);

/// Light-ACK packet-count threshold — 64 packets (`specs/rules/srt-arq.md`
/// rule 12, L2877-2883): if this many packets have been sent/received
/// within the Full ACK period, the receiver sends a Light ACK early.
pub const LIGHT_ACK_THRESHOLD: u32 = 64;

/// `NAKInterval` floor — 20 milliseconds (`specs/rules/srt-arq.md` rule 22,
/// L2953-2960 / L3276): `NAKInterval = max((RTT + 4*RTTVar) / 2, 20ms)`.
pub const NAK_INTERVAL_FLOOR: Duration = Duration::from_millis(20);

/// Convert a [`Duration`] to the wire `Timestamp` field's microsecond
/// `u32` (`draft-sharabayko-srt-01` §3: "microseconds elapsed since the SRT
/// connection was established"), **wrapping** modulo `2^32` once `d` exceeds
/// `u32::MAX` microseconds (about 71.58 minutes) since the caller's epoch —
/// not clamping to `u32::MAX`.
///
/// A clamp (this function's pre-#1063 behavior) makes every packet sent
/// after that point carry the *same* fixed timestamp forever, instead of one
/// that keeps counting up (mod 2^32) the way the field is defined. A real
/// peer's `PktTsbpdTime` formula (`TsbpdTimeBase + PKT_TIMESTAMP + ...`,
/// `specs/rules/srt-tsbpd.md` rule 9) would then compute the same constant
/// play time for every such packet — which the peer's receiver clock keeps
/// advancing past, so every packet after the clamp point looks
/// arbitrarily-far too late and gets Too-Late-Packet-Dropped. The receiving
/// side of the same problem — correctly *un*wrapping a peer's wrapped
/// timestamp back into an always-increasing value for `PktTsbpdTime`
/// purposes — is `crate::tsbpd::TsbpdScheduler`'s `unwrap_timestamp`, not
/// this function (this one only ever produces an in-range wire value; it
/// does not need to remember any wrap-count itself, so plain truncation of
/// the microsecond count into 32 bits is exactly "mod 2^32").
pub(crate) fn duration_to_wire_us(d: Duration) -> u32 {
    // Keep exactly the low 32 bits: the wire field is "microseconds mod 2^32".
    u32::try_from(d.as_micros() & u128::from(u32::MAX)).unwrap_or(0)
}

/// `NAKInterval = max((RTT + 4 * RTTVar) / 2, 20 ms)` (`specs/rules/srt-arq.md`
/// rule 22, verbatim from `draft-sharabayko-srt-01` L2953-2960, unit
/// resolution cross-referenced from §5.2's L3276 restatement).
pub(crate) fn nak_interval(rtt: Duration, rtt_var: Duration) -> Duration {
    let sum = rtt + rtt_var * 4;
    let half = sum / 2;
    if half < NAK_INTERVAL_FLOOR {
        NAK_INTERVAL_FLOOR
    } else {
        half
    }
}
