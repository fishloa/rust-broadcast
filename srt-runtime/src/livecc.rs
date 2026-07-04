//! SRT Live Congestion Control — packet pacing (`draft-sharabayko-srt-01` §5.1,
//! curated at `specs/rules/srt-livecc.md`).
//!
//! A sans-IO, `no_std` sender-side pacing controller that computes the minimum
//! inter-packet send period (`PKT_SND_PERIOD`) from a configured maximum
//! bandwidth (`MAX_BW`) and a running EWMA of the average payload size
//! (`AvgPayloadSize`).
//!
//! ## Usage
//!
//! ```rust
//! use srt_runtime::livecc::{LiveCC, MaxBwConfig};
//! use core::time::Duration;
//!
//! let mut cc = LiveCC::new(Default::default());
//! // Application sends a data packet with 1316 bytes of payload:
//! cc.on_data_packet(1316);
//! // On ACK reception, recompute the send period:
//! let period = cc.on_ack_received();
//! assert!(period > Duration::ZERO);
//! ```
//!
//! ## Spec grounding
//!
//! - `specs/rules/srt-livecc.md` §5.1.1 — MAX_BW configuration modes (§5.1.1,
//!   lines L3116-L3202).
//! - `specs/rules/srt-livecc.md` §5.1.2 — LiveCC algorithm (lines L3203-L3238):
//!   - EWMA formula: L3219 (`AvgPayloadSize = 7/8 * AvgPayloadSize + 1/8 * PacketPayloadSize`)
//!   - Initial value cap: L3222-3223 (max 1456 bytes).
//!   - PktSize formula: L3227-3229 (`PktSize = AvgPayloadSize + SRT header size`).
//!   - PKT_SND_PERIOD formula: L3234 (`PktSize * 1000000 / MAX_BW`).
//! - `specs/rules/srt-livecc.md` — SYN = 0.01 s (imported from §5.2.1,
//!   L3421-3423, restated at L197-L199 of the curated doc).

use core::time::Duration;

/// The fixed SRT header size (16 bytes), defined in `draft-sharabayko-srt-01` §3,
/// Figure 2. Used by the PktSize formula (`specs/rules/srt-livecc.md` §5.1.2
/// step 2, L3227-3229).
const SRT_HEADER_SIZE: u64 = 16;

/// Initial cap for `AvgPayloadSize`: the maximum allowed packet payload size,
/// which cannot be larger than 1456 bytes (`specs/rules/srt-livecc.md` §5.1.2,
/// L3222-3223).
const INITIAL_AVG_PAYLOAD_SIZE_CAP: u64 = 1456;

/// Default MAX_BW for MAXBW_SET mode — 1 Gbps (`specs/rules/srt-livecc.md`
/// §5.1.1, L3122-3123). Stored in bytes per second (L3156-3157): 1 Gbps =
/// 125_000_000 bytes/s.
const DEFAULT_MAX_BW_BYTES_PER_SEC: u64 = 125_000_000;

/// Maximum bandwidth configuration mode (`specs/rules/srt-livecc.md` §5.1.1,
/// lines L3116-L3202).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaxBwConfig {
    /// **MAXBW_SET** (§5.1.1, L3120-3128): MAX_BW set explicitly (bytes/sec).
    /// Default: 1 Gbps = 125_000_000 bytes/sec.
    Set(u64),
    /// **INPUTBW_SET** (§5.1.1, L3130-3146): input rate (bytes/sec) + overhead
    /// percentage. MAX_BW = `input_bw * (1 + overhead / 100)` (L3143).
    InputBased {
        /// The sender's input rate, in bytes per second.
        input_bw: u64,
        /// Overhead percentage (e.g. 25 means 25%).
        overhead: u64,
    },
    /// **INPUTBW_ESTIMATED** (§5.1.1, L3148-3184): measured input rate +
    /// overhead. MAX_BW = `est_input_bw * (1 + overhead / 100)` (L3154).
    ///
    /// Unlike [`MaxBwConfig::InputBased`], `est_input_bw` is updated
    /// externally (e.g. the encoder's measured bitrate). The formula is
    /// identical.
    Estimated {
        /// The estimated sender input rate, in bytes per second.
        est_input_bw: u64,
        /// Overhead percentage (e.g. 25 means 25%).
        overhead: u64,
    },
    /// Unbounded — infinite bandwidth. No packet pacing is applied.
    ///
    /// Not a named mode in §5.1.1's table (L3197-3201), but is the implicit
    /// effect when MAX_BW is not set or is unlimited. Represented by a
    /// PKT_SND_PERIOD of 0 (send as fast as possible).
    Infinite,
}

impl Default for MaxBwConfig {
    /// Default: [`MaxBwConfig::Set`] at 1 Gbps (`specs/rules/srt-livecc.md`
    /// §5.1.1, L3122-3123).
    fn default() -> Self {
        MaxBwConfig::Set(DEFAULT_MAX_BW_BYTES_PER_SEC)
    }
}

impl MaxBwConfig {
    /// The §5.1.1 mode name.
    pub fn name(&self) -> &'static str {
        match self {
            MaxBwConfig::Set(_) => "MAXBW_SET",
            MaxBwConfig::InputBased { .. } => "INPUTBW_SET",
            MaxBwConfig::Estimated { .. } => "INPUTBW_ESTIMATED",
            MaxBwConfig::Infinite => "Infinite",
        }
    }

    /// Resolve the current `MAX_BW` value in bytes per second, or `None` for
    /// unbounded (infinite bandwidth).
    ///
    /// Per `specs/rules/srt-livecc.md` §5.1.1:
    /// - MAXBW_SET: returns the explicit value (L3120-3128).
    /// - INPUTBW_SET: returns `input_bw * (1 + overhead / 100)` (L3143).
    /// - INPUTBW_ESTIMATED: returns `est_input_bw * (1 + overhead / 100)` (L3154).
    /// - Infinite: returns `None`.
    pub fn max_bw_bytes_per_sec(&self) -> Option<u64> {
        match *self {
            MaxBwConfig::Set(bw) => Some(bw),
            MaxBwConfig::InputBased { input_bw, overhead } => {
                // L3143: MAX_BW = INPUT_BW * (1 + OVERHEAD / 100)
                Some(apply_overhead(input_bw, overhead))
            }
            MaxBwConfig::Estimated {
                est_input_bw,
                overhead,
            } => {
                // L3154: MAX_BW = EST_INPUT_BW * (1 + OVERHEAD / 100)
                Some(apply_overhead(est_input_bw, overhead))
            }
            MaxBwConfig::Infinite => None,
        }
    }
}

broadcast_common::impl_spec_display!(MaxBwConfig);

/// Apply `'MAX_BW = bw * (1 + overhead / 100)'` (`specs/rules/srt-livecc.md`
/// L3143/L3154, verbatim).
fn apply_overhead(bw: u64, overhead: u64) -> u64 {
    // `overhead / 100` is truncated integer division, but the spec formula
    // `(1 + OVERHEAD / 100)` is a real multiplier — we lose precision with
    // integer arithmetic. Compute as `bw + bw * overhead / 100` instead.
    bw + bw * overhead / 100
}

/// SRT Live Congestion Control — sender-side pacing state
/// (`draft-sharabayko-srt-01` §5.1.2).
///
/// Tracks the EWMA average payload size and recomputes the inter-packet send
/// period on each ACK.
///
/// Sans-IO: all timing is caller-driven — [`LiveCC::on_data_packet`],
/// [`LiveCC::on_ack_received`], and [`LiveCC::tick`] take explicit inputs and
/// return computed values; the controller never reads a wall clock internally.
#[derive(Debug, Clone)]
pub struct LiveCC {
    /// Average payload size (EWMA), in bytes — `AvgPayloadSize` from §5.1.2
    /// (L3219).
    avg_payload_size: u64,
    /// Maximum bandwidth configuration (§5.1.1).
    max_bw_config: MaxBwConfig,
}

impl LiveCC {
    /// Create a new LiveCC pacing controller.
    ///
    /// `max_bw_config` — the MAX_BW mode (§5.1.1). Use `Default::default()`
    /// for the default 1 Gbps fixed MAXBW_SET mode.
    ///
    /// Initial `AvgPayloadSize` is set to the initial cap (1456 bytes),
    /// per `specs/rules/srt-livecc.md` §5.1.2 (L3222-3223).
    pub fn new(max_bw_config: MaxBwConfig) -> Self {
        LiveCC {
            avg_payload_size: INITIAL_AVG_PAYLOAD_SIZE_CAP,
            max_bw_config,
        }
    }

    /// Update the EWMA average payload size on sending a data packet
    /// (`specs/rules/srt-livecc.md` §5.1.2, L3216-3223).
    ///
    /// `packet_payload_size` — the payload size (bytes) of the just-sent data
    /// packet (original or retransmitted, L3216-3217).
    ///
    /// Formula (L3219, verbatim):
    ///
    /// ```text
    /// AvgPayloadSize = 7/8 * AvgPayloadSize + 1/8 * PacketPayloadSize
    /// ```
    pub fn on_data_packet(&mut self, packet_payload_size: u64) {
        // L3219: AvgPayloadSize = 7/8 * AvgPayloadSize + 1/8 * PacketPayloadSize
        let old = self.avg_payload_size;
        self.avg_payload_size = (7 * old + packet_payload_size) / 8;
    }

    /// The current average payload size estimate, in bytes.
    pub fn avg_payload_size(&self) -> u64 {
        self.avg_payload_size
    }

    /// The current MAX_BW configuration.
    pub fn max_bw_config(&self) -> &MaxBwConfig {
        &self.max_bw_config
    }

    /// Reconfigure the MAX_BW mode at runtime.
    pub fn set_max_bw_config(&mut self, config: MaxBwConfig) {
        self.max_bw_config = config;
    }

    /// Compute the current inter-packet send period in microseconds
    /// (`specs/rules/srt-livecc.md` §5.1.2, L3225-3238).
    ///
    /// Call this on ACK reception (method (2), L3225). The period is also
    /// used after any state change (e.g. reconfiguration).
    ///
    /// Returns `Duration::ZERO` when bandwidth is unbounded (infinite mode),
    /// meaning "send as fast as possible".
    ///
    /// Step 1 — PktSize (L3227-3229, paraphrased):
    ///
    /// ```text
    /// PktSize = AvgPayloadSize + SRT header size
    /// ```
    ///
    /// Step 2 — PKT_SND_PERIOD (L3234, verbatim):
    ///
    /// ```text
    /// PKT_SND_PERIOD = PktSize * 1000000 / MAX_BW
    /// ```
    pub fn on_ack_received(&self) -> Duration {
        let Some(max_bw) = self.max_bw_config.max_bw_bytes_per_sec() else {
            // Infinite: no pacing.
            return Duration::ZERO;
        };
        if max_bw == 0 {
            return Duration::ZERO;
        }

        // L3227-3229: PktSize = AvgPayloadSize + SRT header size (16 bytes).
        let pkt_size = self.avg_payload_size + SRT_HEADER_SIZE;

        // L3234: PKT_SND_PERIOD = PktSize * 1000000 / MAX_BW
        //
        // pkt_size is in bytes, MAX_BW in bytes/sec. The factor 1_000_000
        // converts seconds to microseconds (L3237-3238).
        let period_us = pkt_size * 1_000_000 / max_bw;
        Duration::from_micros(period_us)
    }

    /// Time-driven tick — currently a no-op for LiveCC (the `§5.1.2` algorithm
    /// is driven by packet/ACK events only; an RTO timeout also triggers
    /// `on_data_packet`, per L3240-3241).
    ///
    /// Provided for forward compatibility: the sender loop should call this
    /// every cycle. Returns `None`.
    pub fn tick(&mut self) -> Option<Duration> {
        // Per spec, LiveCC reacts to events (data send, ACK, RTO — L3211-3214);
        // the RTO timeout also calls on_data_packet (L3240-3241). No periodic
        // internal computation needed.
        None
    }
}

impl Default for LiveCC {
    fn default() -> Self {
        LiveCC::new(Default::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_avg_payload_size_is_1456_cap() {
        // L3222-3223: initial AvgPayloadSize is the max allowed payload, max 1456.
        let cc = LiveCC::new(Default::default());
        assert_eq!(cc.avg_payload_size(), 1456);
    }

    #[test]
    fn default_bw_is_1_gbps() {
        // L3122-3123: recommended default is 1 Gbps.
        let cfg = MaxBwConfig::default();
        assert_eq!(cfg.max_bw_bytes_per_sec(), Some(125_000_000));
    }

    #[test]
    fn ewma_update_matches_hand_computed_formula() {
        // L3219: AvgPayloadSize = 7/8 * old + 1/8 * payload.
        // Initial: 1456. Feed a 100-byte payload.
        // new = (7 * 1456 + 100) / 8 = (10192 + 100) / 8 = 10292 / 8 = 1286
        let mut cc = LiveCC::new(Default::default());
        cc.on_data_packet(100);
        assert_eq!(cc.avg_payload_size(), 1286);
    }

    #[test]
    fn ewma_converges_toward_constant_payload() {
        // Feed a constant 1000-byte payload many times; the EWMA should
        // converge toward 1000.
        let mut cc = LiveCC::new(Default::default());
        for _ in 0..64 {
            cc.on_data_packet(1000);
        }
        let avg = cc.avg_payload_size();
        // Converged within 1 of 1000.
        assert!(avg.abs_diff(1000) <= 1, "expected ~1000, got {avg}");
    }

    #[test]
    fn ewma_respects_initial_cap_regardless_of_first_feed() {
        // The initial cap (1456) is separate from the first update.
        // Even if we feed a tiny payload, the init was 1456, not the fed value.
        let mut cc = LiveCC::new(Default::default());
        cc.on_data_packet(50);
        // new = (7 * 1456 + 50) / 8 = (10192 + 50) / 8 = 10242 / 8 = 1280
        assert_eq!(cc.avg_payload_size(), 1280);
    }

    #[test]
    fn pkt_snd_period_formula_matches_hand_computed() {
        // L3234: PKT_SND_PERIOD = PktSize * 1000000 / MAX_BW.
        //
        // With default MAX_BW = 125_000_000 bytes/sec and init AvgPayloadSize
        // = 1456:
        //   PktSize = 1456 + 16 = 1472
        //   period = 1472 * 1_000_000 / 125_000_000 = 1_472_000_000 / 125_000_000
        //          = 11 (integer division; 11.776 truncated to 11)
        let cc = LiveCC::new(Default::default());
        let period = cc.on_ack_received();
        assert_eq!(period, Duration::from_micros(11));
    }

    #[test]
    fn pkt_snd_period_with_different_payload_size() {
        // After feeding a 1316-byte payload seven times (to get close to
        // converged), compute the period.
        //
        // EWMA after 7 steps: we don't hand-compute the exact 8-step weight
        // here; instead we use a simpler scenario:
        //   initial: 1456
        //   feed 1316: avg = (7*1456 + 1316)/8 = 1438
        //   feed 1316: avg = (7*1438 + 1316)/8 = 1422
        //   PktSize = 1422 + 16 = 1438
        //   period = 1438 * 1_000_000 / 125_000_000 = 11
        let mut cc = LiveCC::new(Default::default());
        cc.on_data_packet(1316);
        cc.on_data_packet(1316);
        let period = cc.on_ack_received();
        assert_eq!(period, Duration::from_micros(11));
    }

    #[test]
    fn input_bw_mode_formula() {
        // L3143: MAX_BW = INPUT_BW * (1 + OVERHEAD / 100).
        // input_bw = 10_000_000 (10 MB/s), overhead = 25%.
        // MAX_BW = 10_000_000 + 10_000_000 * 25 / 100 = 12_500_000 bytes/sec.
        let cfg = MaxBwConfig::InputBased {
            input_bw: 10_000_000,
            overhead: 25,
        };
        assert_eq!(cfg.max_bw_bytes_per_sec(), Some(12_500_000));
    }

    #[test]
    fn estimated_mode_formula() {
        // L3154: same formula as INPUTBW_SET but with EST_INPUT_BW.
        let cfg = MaxBwConfig::Estimated {
            est_input_bw: 5_000_000,
            overhead: 10,
        };
        // MAX_BW = 5_000_000 + 5_000_000 * 10 / 100 = 5_500_000
        assert_eq!(cfg.max_bw_bytes_per_sec(), Some(5_500_000));
    }

    #[test]
    fn infinite_mode_returns_zero_period() {
        let cc = LiveCC::new(MaxBwConfig::Infinite);
        assert_eq!(cc.on_ack_received(), Duration::ZERO);
    }

    #[test]
    fn set_mode_period_scales_with_bw() {
        // MAXBW_SET at 62_500_000 bytes/sec (500 Mbps); init PktSize = 1472.
        // period = 1472 * 1_000_000 / 62_500_000 = 23 (23.552 truncated).
        let cc = LiveCC::new(MaxBwConfig::Set(62_500_000));
        assert_eq!(cc.on_ack_received(), Duration::from_micros(23));
    }

    #[test]
    fn runtime_reconfiguration_affects_period() {
        let mut cc = LiveCC::new(MaxBwConfig::Infinite);
        assert_eq!(cc.on_ack_received(), Duration::ZERO);

        cc.set_max_bw_config(MaxBwConfig::Set(125_000_000));
        assert_eq!(cc.on_ack_received(), Duration::from_micros(11));
    }

    #[test]
    fn zero_bw_returns_zero_period() {
        // Guard: if MAX_BW is 0, we'd divide by zero. Return ZERO instead.
        let cc = LiveCC::new(MaxBwConfig::Set(0));
        assert_eq!(cc.on_ack_received(), Duration::ZERO);
    }

    #[test]
    fn tick_is_noop() {
        let mut cc = LiveCC::new(Default::default());
        assert_eq!(cc.tick(), None);
        // Calling tick must not mutate state.
        assert_eq!(cc.avg_payload_size(), 1456);
    }

    #[test]
    fn max_bw_config_clone_and_eq() {
        let a = MaxBwConfig::Set(100);
        let b = MaxBwConfig::Set(100);
        assert_eq!(a, b);
        let c = MaxBwConfig::InputBased {
            input_bw: 1000,
            overhead: 20,
        };
        let d = MaxBwConfig::InputBased {
            input_bw: 1001,
            overhead: 20,
        };
        assert_ne!(c, d);
    }

    #[test]
    fn overhead_formula_edge_cases() {
        // overhead = 0: MAX_BW = bw + bw*0/100 = bw
        assert_eq!(
            MaxBwConfig::InputBased {
                input_bw: 10_000,
                overhead: 0,
            }
            .max_bw_bytes_per_sec(),
            Some(10_000)
        );
        // overhead = 100: MAX_BW = bw + bw*100/100 = 2*bw
        assert_eq!(
            MaxBwConfig::InputBased {
                input_bw: 10_000,
                overhead: 100,
            }
            .max_bw_bytes_per_sec(),
            Some(20_000)
        );
    }
}
