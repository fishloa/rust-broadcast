//! Receiver-side ARQ (Automatic Repeat reQuest) reliability engine — VSF
//! TR-06-1:2020 §5.3 (NACK-Based Recovery). Curated transcription:
//! `docs/tr-06-1-simple-profile.md` §4 (§5.3.1 receiver buffers, §5.3.2
//! retransmission requests, §5.3.3 sender response, §5.3.4 burst control,
//! Appendix B suggested defaults).
//!
//! Sans-IO, like [`crate`] as a whole and architecturally mirroring
//! `srt-runtime::arq`: [`Receiver`] and [`Sender`] never read a wall clock —
//! every method that needs the current time takes a caller-supplied
//! `now: core::time::Duration` (elapsed time since a fixed epoch the caller
//! owns). Nothing here touches a socket; the wire codecs
//! ([`crate::RangeNack`] / [`crate::GenericNack`] / [`crate::RttEcho`] /
//! [`crate::RistReceiverCompound`]) live in [`crate::nack`] /
//! [`crate::rtt_echo`] / [`crate::compound`], and this module only decides
//! *when* to build one and *what* to do with one received.
//!
//! # Attribution — read this before touching any timing number below
//!
//! TR-06-1 states almost none of this engine's *timing* behaviour, and is
//! explicit that it doesn't:
//!
//! - §5.3.1 explicitly leaves *where* loss is detected to the implementation
//!   (a "minimum-delay" policy at the buffer's input vs. a bonding-capable
//!   policy sized for the worst-case path differential).
//! - §5.3.4 states outright that retransmission-request backoff and
//!   burst-suppression "are left to the discretion of the implementer" — no
//!   formula relating RTT to timing is given, and TR-06-2:2024 (the Main
//!   Profile) was checked and confirmed to add none either
//!   (`docs/tr-06-2-main-profile-timing.md`).
//! - Appendix B's only concrete numbers are flat, RTT-independent
//!   *suggested defaults* ("manually configured... in the absence of user
//!   input"): 1000 ms Receiver Buffer, 70 ms Reorder Section, 7
//!   retransmission requests per packet, and a *derived* (not independently
//!   stated) ~132 ms request interval (`(1000 - 70) / 7`).
//!
//! Given that gap, this engine's retry-scheduling algorithm is **not**
//! Appendix B's flat interval — it is modeled on **librist**
//! (`https://code.videolan.org/rist/librist.git`, BSD-2-Clause, the VSF
//! reference implementation; see `fixtures/rist/PROVENANCE.md` for the
//! licence text this crate already vendors a capture from). Concretely,
//! from librist `src/rist-common.c` and `src/flow.c` (read directly, not
//! taken on faith):
//!
//! - A smoothed RTT via an **8-sample EWMA** stored as an accumulator that
//!   is 8x the true value (`eight_times_rtt -= eight_times_rtt / 8;
//!   eight_times_rtt += sample;`, read back via `/ 8`) — [`rtt::RttEstimator`]
//!   copies this exact integer-tick shape rather than substituting a
//!   floating-point EWMA with a different smoothing factor.
//! - The **first** retransmission request for a newly-detected loss is
//!   scheduled at `now + rtt` (`src/flow.c` — `rist_receiver_missing`);
//!   every **subsequent** retry at `now + 1.1 * rtt`
//!   (`src/rist-common.c::rist_process_nack`) — an explicit "retry more as
//!   the deadline nears" proportional-ratio alternative exists in that same
//!   function, commented out and disabled upstream; this crate does not
//!   implement it either.
//! - The smoothed RTT is clamped to a configurable `[recovery_rtt_min,
//!   recovery_rtt_max]` range (librist defaults: 5 ms / 500 ms —
//!   `RIST_DEFAULT_RECOVERY_RTT_MIN`/`_MAX`) before use.
//! - A lost packet is given up on when **either** it has aged out of the
//!   recovery buffer (`now - insertion_time > recovery_buffer_ticks * 1.1`)
//!   **or** its retry count reaches a configured cap — age is checked
//!   first, matching this crate's [`Receiver::tick`] ordering.
//!
//! **librist is corroborating evidence that this is a workable real-world
//! policy — it is not the specification.** TR-06-1 Appendix B's numbers
//! (1000 ms / 70 ms / 7 requests / ~132 ms) remain this crate's *documented
//! fallback*, used only for the very first request of a loss detected
//! before this receiver has measured any real RTT sample yet — precisely
//! the "in the absence of user input" case Appendix B is written for. Once
//! an [`RttEcho`](crate::RttEcho) round trip completes and
//! [`Receiver::on_rtt_sample`] is called, RTT-driven scheduling takes over
//! and Appendix B's flat interval is no longer consulted. Every field this
//! divide applies to is documented at its own declaration in [`ArqConfig`]
//! and [`rtt`] — do not conflate a librist-sourced default with a
//! spec-stated one; that conflation is exactly what produced the three
//! fabricated Appendix B figures this crate's docs previously had to
//! correct (see `docs/tr-06-1-simple-profile.md` Appendix B's own
//! correction note).
//!
//! # Module map
//! - [`seq`] — wrap-safe 16-bit RTP sequence-number arithmetic
//!   (**implementation policy** — TR-06-1 doesn't specify a comparison
//!   algorithm; resolved the same way `srt_runtime::arq::seq` resolves the
//!   analogous gap for SRT's 31-bit space).
//! - [`rtt`] — [`rtt::RttEstimator`] (the librist-sourced 8-tap EWMA) and
//!   [`rtt::rtt_sample`] (turning a completed RTT Echo round trip, §5.2.6,
//!   into a `Duration` sample).
//! - [`Receiver`] — Reorder Section + Retransmission Reassembly Section
//!   (§5.3.1), loss detection, retry-capped NACK scheduling (§5.3.2, §5.3.4,
//!   Appendix B, librist-sourced timing per the Attribution section above).
//! - [`Sender`] — the sender-side NACK response (§5.3.3): locate a
//!   previously-sent packet by sequence number and hand it back for
//!   retransmission. §5.3.3 explicitly does not prescribe the lookup
//!   mechanism ("that storage/lookup mechanism is left to the
//!   implementation"), so the bounded ring buffer here is
//!   **implementation policy**, not a transcription.
//!
//! # Non-goals
//! - TR-06-2 Main Profile extended (32-bit) sequence numbers (§8.3-8.4) —
//!   out of scope; this engine is 16-bit-sequence Simple Profile only.
//! - Bonding (§5.4) — multi-path aggregation is not modeled.
//! - librist's disabled proportional-ratio backoff variant (a commented-out
//!   alternative in `rist_process_nack` that retries harder as the retry
//!   budget nears exhaustion) — noted in the Attribution section above,
//!   deliberately not ported since librist itself does not ship it.

mod receiver;
pub mod rtt;
mod sender;
pub mod seq;

pub use receiver::{DeliveryOutcome, Receiver, TickOutcome, ranges_to_fci, seqs_to_fci};
pub use sender::{Retransmission, Sender};

use core::time::Duration;

/// Cap on how many individual sequence numbers a single loss-range or
/// bitmask expansion will enumerate in one call.
///
/// TR-06-1 §5.3.4 (Burst Control, informative) calls this out explicitly: a
/// single Range-Based request field can nominally set `Additional = 0xFFFF`
/// and demand 65536 retransmissions from one 32-bit field, and "an
/// implementation must be prepared to throttle/reject this rather than
/// attempt it literally." This is that throttle — **implementation
/// policy** for the concrete cap value (`2^16`, matching
/// `srt_runtime::arq::sender::MAX_RANGE_EXPANSION`'s equivalent safety cap),
/// since §5.3.4 states the *need* for a limit but not a number.
pub(crate) const MAX_RANGE_EXPANSION: usize = 1 << 16;

/// Tunable ARQ timing/sizing parameters.
///
/// Fields are grouped by provenance — see each field's own doc comment, and
/// the module-level Attribution section for the full accounting. In short:
/// `receiver_buffer`/`reorder_section`/`max_retransmission_requests` are
/// TR-06-1 Appendix B's suggested defaults (informative, not normative);
/// `recovery_rtt_min`/`recovery_rtt_max` have no TR-06-1 equivalent at all
/// and are sourced from librist's own defaults as corroborating evidence for
/// a workable real-world clamp range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ArqConfig {
    /// Total receiver buffer depth: Reorder Section + Retransmission
    /// Reassembly Section combined (TR-06-1 Appendix B suggested default:
    /// 1000 ms).
    pub receiver_buffer: Duration,
    /// How long an out-of-order gap ages in the Reorder Section before
    /// being promoted into the Retransmission Reassembly Section — i.e.
    /// treated as a confirmed loss eligible for a NACK (§5.3.1; Appendix B
    /// suggested default: 70 ms).
    pub reorder_section: Duration,
    /// Maximum number of times a single lost packet is (re)requested before
    /// this engine gives up on it (Appendix B: "Number of Retransmission
    /// Requests per Packet", suggested default 7).
    pub max_retransmission_requests: u32,
    /// Floor for the RTT-driven retry-scheduling clamp (see the module
    /// Attribution section). **Not a TR-06-1 number** — sourced from
    /// librist's `RIST_DEFAULT_RECOVERY_RTT_MIN` (5 ms).
    pub recovery_rtt_min: Duration,
    /// Ceiling for the RTT-driven retry-scheduling clamp. **Not a TR-06-1
    /// number** — sourced from librist's `RIST_DEFAULT_RECOVERY_RTT_MAX`
    /// (500 ms).
    pub recovery_rtt_max: Duration,
}

impl ArqConfig {
    /// TR-06-1 Appendix B's suggested defaults for the three spec-stated
    /// fields, paired with librist's RTT-clamp defaults for the two fields
    /// TR-06-1 has no opinion on at all. See [`ArqConfig`]'s doc for the
    /// per-field provenance split.
    pub const fn appendix_b_defaults() -> Self {
        ArqConfig {
            receiver_buffer: Duration::from_millis(1000),
            reorder_section: Duration::from_millis(70),
            max_retransmission_requests: 7,
            recovery_rtt_min: Duration::from_millis(5),
            recovery_rtt_max: Duration::from_millis(500),
        }
    }

    /// The Appendix B *fallback* interval between successive retransmission
    /// requests, used only until a [`Receiver`] has measured its first real
    /// RTT sample (see the module Attribution section) — *derived*, not
    /// independently stated by TR-06-1: "the receiver buffer minus the
    /// reorder section divided by the number of retransmission requests."
    /// For the Appendix B defaults that is `(1000 ms - 70 ms) / 7 ≈
    /// 132.86 ms`, which Appendix B states rounded as "132 ms".
    pub fn fallback_retransmission_interval(&self) -> Duration {
        let denom = self.max_retransmission_requests.max(1);
        self.reassembly_budget() / denom
    }

    /// The Retransmission Reassembly Section's own time budget: the total
    /// receiver buffer minus the reorder section (Appendix B's "the
    /// remainder" — `1000 - 70 = 930 ms` for the defaults). A packet's
    /// give-up age check ([`Receiver::tick`]) is measured against this
    /// budget (scaled by librist's `* 1.1` margin), not the full
    /// `receiver_buffer`, because this engine's own Reorder Section has
    /// already consumed `reorder_section` of that packet's total lifetime
    /// before it is ever promoted into tracked-loss state.
    pub fn reassembly_budget(&self) -> Duration {
        self.receiver_buffer.saturating_sub(self.reorder_section)
    }
}

impl Default for ArqConfig {
    fn default() -> Self {
        Self::appendix_b_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appendix_b_defaults_derive_the_spec_stated_132ms_fallback_interval() {
        let cfg = ArqConfig::default();
        let interval = cfg.fallback_retransmission_interval();
        // Appendix B states the rounded outcome as "132 ms"; the exact
        // division is 930/7 ≈ 132.857ms — assert it lands in that
        // documented rounding window rather than hardcoding a rounded
        // magic number as if it were exact.
        assert!(interval >= Duration::from_millis(132));
        assert!(interval < Duration::from_millis(133));
    }

    #[test]
    fn reassembly_budget_is_the_stated_930ms_remainder() {
        let cfg = ArqConfig::default();
        assert_eq!(cfg.reassembly_budget(), Duration::from_millis(930));
    }
}
