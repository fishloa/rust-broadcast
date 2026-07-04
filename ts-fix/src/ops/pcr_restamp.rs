//! PCR restamp operation.
//!
//! Recomputes the 42-bit Program Clock Reference, **per PCR PID**, using a
//! timing model based on bitrate (robust repair) or interpolation between
//! observed PCRs (best-effort smoothing). A transport stream may carry several
//! programs, each with its own PCR PID; every PCR PID is restamped
//! independently from its own anchor.
//!
//! # Discontinuity re-anchor (ITU-T H.222.0 §2.4.3.5)
//!
//! When a packet on a PCR_PID has adaptation-field
//! [`discontinuity_indicator == 1`], it signals a **system-time-base
//! discontinuity**: the next PCR on that PID samples a new clock. The restamp
//! MUST NOT interpolate or smooth across this boundary — it resets that PID's
//! anchor to the observed PCR, so the two segments are restamped independently
//! from their own bases.
//!
//! # Genuine, unflagged discontinuities (#562)
//!
//! A break with **no** `discontinuity_indicator` is not a legal re-anchor
//! point — it is the defect this operation exists to fix. `Interpolate` mode
//! classifies every observed forward jump using
//! [`super::pcr_conformance::PcrDiscDetector`] (which reuses
//! `dvb_conformance::ConformanceMonitor`'s ETSI TR 101 290 §5.2.2 indicator
//! 2.3b check — the 100 ms threshold is never re-derived here). A jump that
//! trips 2.3b is **not** adopted as a sane observation; the PID's anchor is
//! permanently "frozen" onto its pre-break rate from that point on (every
//! later value is still on the same unflagged, jumped clock and would
//! otherwise look like just another sane forward step one packet later), so
//! the restamped PCR stays on one continuous timeline across — and past —
//! the break. Only a legal flagged discontinuity un-freezes a PID (it
//! replaces the anchor outright). `FromBitrate` mode already ignores
//! observed values outside of flagged re-anchors, so it ships this same
//! guarantee for free. See `ts-fix/src/ops/pcr_honor.rs` for the alternative
//! **honor** repair, which flags such a break instead of rewriting it.
//!
//! # PCR 33-bit base wrap
//!
//! The PCR is a 42-bit field: 33-bit base (90 kHz) × 300 + 9-bit extension,
//! so the full 27 MHz value wraps at `2^33 × 300` (PCR\_27MHZ\_MODULUS).
//! The Interpolate mode handles a legal wrap (where the raw observed value
//! appears to decrease) via modular forward-distance comparison. All computed
//! values are reduced modulo `PCR_27MHZ_MODULUS` so they wrap at the PCR
//! boundary rather than the u64 boundary.
//!
//! # Forward-compat note
//!
//! The PCR is set **in-place** via [`mpeg_ts::OwnedTsPacket::set_pcr`], which
//! overwrites the existing 6-byte field without re-serialising the adaptation
//! field (so length/stuffing are preserved). The shared
//! [`crate::ops::TimingContext`] in `StreamModel` is the forward-compat carrier
//! that v0.2's PTS/DTS-wrap op will reuse; PCR's per-PID anchors are local to
//! this op.
//!
//! # SCTE-35 splice PTS is intentionally NOT adjusted (#417)
//!
//! A SCTE-35 `splice_time.pts_time` (and `pts_adjustment`) is a **PTS** on the
//! program's 90 kHz **presentation** clock — the same timeline as the PES
//! `PTS`/`DTS`. The **PCR** is the independent transport-layer clock reference.
//! This op restamps only the PCR PID; it does **not** rewrite PES `PTS`/`DTS`, so
//! the presentation timeline is unchanged and the cue stays aligned to the media.
//! Shifting `splice_time.pts_time` by the PCR delta would therefore *desync* the
//! cue from the (unchanged) PES PTS. So SCTE-35 cues are left byte-identical
//! through a PCR restamp (see `tests/scte35_preserve.rs`). A splice-PTS
//! adjustment becomes correct only once a PES-PTS-rebase op exists (the v0.2
//! PTS/DTS-wrap op), at which point the cue would shift by the *same* PES-PTS delta.
//!
//! # Spec
//!
//! ISO/IEC 13818-1 (= ITU-T H.222.0) §2.4.3.5 (PCR semantics). PCR is
//! **per-program**: the PMT names a `PCR_PID` per program (§2.4.4.9), and
//! §2.7.2 requires the PCRs on "the PCR_PID **for each program**" — so a
//! multi-program TS carries multiple PCR PIDs, which is why this op anchors
//! and restamps each PCR PID independently.

use alloc::collections::BTreeMap;

use mpeg_ts::owned::OwnedTsPacket;
use mpeg_ts::ts::{Pcr, TS_PACKET_SIZE, TsPacket};

use crate::ops::pcr_conformance::PcrDiscDetector;
use crate::ops::{Op, StreamModel};

/// 27 MHz PCR wrap period: 33-bit base × 300 (ISO/IEC 13818-1 §2.4.3.5).
const PCR_27MHZ_MODULUS: u64 = (1u64 << 33) * 300;

/// PCR restamp mode.
///
/// `#[non_exhaustive]` — new modes (e.g. `from_external_clock`) may be added
/// in future releases without a breaking change.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum PcrRestamp {
    /// Interpolate PCRs from each PID's first anchor + observed inter-PCR rate
    /// (best-effort smoothing of jitter; preserves observed values where sane).
    Interpolate,
    /// Recompute PCRs from a fixed bitrate (bits per second), per PID:
    /// `PCR = anchor + (packets_since_anchor × 188 × 8 / bitrate) × 27_000_000`.
    /// Robust against corrupted PCR values (ignores the observed value).
    FromBitrate {
        /// Bitrate in bits per second.
        bps: u64,
    },
}

impl PcrRestamp {
    /// Interpolate PCRs from each PID's anchor + observed rate (jitter smoothing).
    ///
    /// # Example
    /// ```
    /// use ts_fix::PcrRestamp;
    /// let cfg = PcrRestamp::interpolate();
    /// ```
    pub fn interpolate() -> Self {
        Self::Interpolate
    }

    /// Recompute PCRs from a fixed bitrate (bits/second) — robust repair.
    ///
    /// # Example
    /// ```
    /// use ts_fix::PcrRestamp;
    /// let cfg = PcrRestamp::from_bitrate(27_000_000);
    /// ```
    pub fn from_bitrate(bps: u64) -> Self {
        Self::FromBitrate { bps }
    }
}

/// Per-PID PCR anchor + running rate (in 27 MHz ticks per TS packet).
#[derive(Clone, Copy)]
struct Anchor {
    /// 27 MHz value of the first PCR seen on this PID (preserved).
    anchor_27mhz: u64,
    /// `packet_count` at the anchor.
    anchor_pkt: u64,
    /// Last monotonic observation: (packet_count, 27 MHz) — for Interpolate rate.
    last_obs_pkt: u64,
    last_obs_27mhz: u64,
    /// #562: once `Interpolate` mode has hit a genuine, unflagged
    /// discontinuity on this PID, it is permanently "frozen" onto the
    /// pre-break rate — every later observation is on the (still unflagged)
    /// post-break clock and must never be re-adopted, or the output would
    /// carry the exact same jump one packet later. Cleared only by a legal
    /// flagged discontinuity, which replaces this `Anchor` outright.
    frozen: bool,
}

/// PCR restamp operation — restamps every PCR PID independently.
pub(crate) struct PcrRestampOp {
    anchors: BTreeMap<u16, Anchor>,
    mode: PcrRestamp,
    /// TR 101 290 §5.2.2 indicator 2.3b classifier (#562) — tells
    /// `Interpolate` mode whether an observed forward jump is a genuine,
    /// unflagged discontinuity (must NOT be adopted as a "sane" observation)
    /// rather than normal jitter or a legal 33-bit base wrap.
    disc_detector: PcrDiscDetector,
}

impl PcrRestampOp {
    pub(crate) fn new(mode: PcrRestamp) -> Self {
        Self {
            anchors: BTreeMap::new(),
            mode,
            disc_detector: PcrDiscDetector::new(),
        }
    }

    /// 27 MHz ticks per 188-byte packet at `bps` (min 1).
    fn ticks_per_packet(bps: u64) -> u64 {
        let num = 188u64 * 8 * 27_000_000u64;
        if bps == 0 || bps >= num {
            1
        } else {
            (num / bps).max(1)
        }
    }

    /// Read `(pid, pcr, discontinuity)` if this packet carries a PCR.
    ///
    /// `discontinuity` is `true` when `discontinuity_indicator == 1` in the
    /// adaptation field (ITU-T H.222.0 §2.4.3.5).
    fn read_pcr(packet: &[u8]) -> Option<(u16, Pcr, bool)> {
        let pkt = TsPacket::parse(packet).ok()?;
        let af = pkt.adaptation_field().and_then(|r| r.ok())?;
        let pcr = af.pcr?;
        Some((pkt.header.pid, pcr, af.discontinuity_indicator))
    }
}

impl Op for PcrRestampOp {
    fn process(&mut self, packet: &[u8], model: &mut StreamModel, out: &mut dyn FnMut(&[u8])) {
        if packet.len() != TS_PACKET_SIZE {
            out(packet);
            return;
        }
        let Some((pid, current, discontinuity)) = Self::read_pcr(packet) else {
            out(packet);
            return;
        };
        let now = model.packet_count;

        // TR 101 290 §5.2.2 indicator 2.3b classifier (#562): feed the
        // ORIGINAL observed PCR through the shared conformance detector on
        // every path so its per-PID PCR state tracks the true input
        // timeline. Only `Interpolate` mode consults the verdict below — a
        // `discontinuity`-flagged packet never raises 2.3b (it only fires
        // when `discontinuity_indicator == 0`), so this call is a no-op for
        // that branch.
        let is_genuine_unflagged_break = self.disc_detector.feed(packet).is_some();

        // System-time-base discontinuity (§2.4.3.5): re-anchor this PID to the
        // current observed PCR. The discontinuity packet itself passes through
        // unchanged (its discontinuity_indicator is in the AF flags byte, not
        // touched by set_pcr).
        if discontinuity {
            let a = Anchor {
                anchor_27mhz: current.as_27mhz(),
                anchor_pkt: now,
                last_obs_pkt: now,
                last_obs_27mhz: current.as_27mhz(),
                frozen: false,
            };
            self.anchors.insert(pid, a);
            model.timing.has_anchor = true;
            model.timing.clock_27mhz = current.as_27mhz();
            out(packet);
            return;
        }

        // First PCR on this PID → anchor, preserve as-is.
        let Some(anchor) = self.anchors.get_mut(&pid) else {
            let a = Anchor {
                anchor_27mhz: current.as_27mhz(),
                anchor_pkt: now,
                last_obs_pkt: now,
                last_obs_27mhz: current.as_27mhz(),
                frozen: false,
            };
            self.anchors.insert(pid, a);
            // Mark the shared timing context as anchored (forward-compat for v0.2).
            model.timing.has_anchor = true;
            model.timing.clock_27mhz = current.as_27mhz();
            out(packet);
            return;
        };

        let new_27mhz = match &self.mode {
            PcrRestamp::FromBitrate { bps } => {
                let delta = now.saturating_sub(anchor.anchor_pkt);
                anchor
                    .anchor_27mhz
                    .wrapping_add(Self::ticks_per_packet(*bps) * delta)
                    % PCR_27MHZ_MODULUS
            }
            PcrRestamp::Interpolate => {
                // Derive the rate from the last monotonic observation on this PID.
                let obs = current.as_27mhz();
                let pkt_delta = now.saturating_sub(anchor.last_obs_pkt);
                // Wrap-aware forward-distance check.  On a 33-bit PCR base wrap
                // the raw `obs` is smaller than `last_obs_27mhz`, but the forward
                // distance modulo the PCR modulus is a small positive step.
                let fwd = obs.wrapping_sub(anchor.last_obs_27mhz) % PCR_27MHZ_MODULUS;
                // #562: a genuine, unflagged discontinuity (TR 101 290 §5.2.2
                // indicator 2.3b) is a forward jump too, but it must NOT be
                // adopted as a "sane" observation — that would carry the
                // defect straight into the restamped output. Once one is seen
                // on this PID, `frozen` stays set: every later observation is
                // still on the (still unflagged) post-break clock relative to
                // the pre-break anchor, so it would look like just another
                // "sane forward jump" one packet later and re-introduce the
                // exact same break. Freezing keeps the PID on the pre-break
                // rate indefinitely — one continuous timeline across (and
                // past) the break — until a legal flagged discontinuity
                // replaces the anchor outright.
                if is_genuine_unflagged_break {
                    anchor.frozen = true;
                }
                if !anchor.frozen && fwd > 0 && fwd < PCR_27MHZ_MODULUS / 2 && pkt_delta > 0 {
                    // Sane forward observation (possibly across a wrap): trust it,
                    // advance the anchor's observation window.
                    anchor.last_obs_pkt = now;
                    anchor.last_obs_27mhz = obs;
                    obs
                } else {
                    // Frozen, or a non-monotonic/corrupt observation: recompute
                    // from the anchor using the last known (pre-break) rate.
                    let span_pkt = anchor.last_obs_pkt.saturating_sub(anchor.anchor_pkt).max(1);
                    let span_ticks = anchor.last_obs_27mhz.saturating_sub(anchor.anchor_27mhz);
                    let rate = (span_ticks / span_pkt).max(1);
                    let delta = now.saturating_sub(anchor.anchor_pkt);
                    anchor.anchor_27mhz.wrapping_add(rate * delta) % PCR_27MHZ_MODULUS
                }
            }
        };

        let mut buf = [0u8; TS_PACKET_SIZE];
        buf.copy_from_slice(packet);
        if OwnedTsPacket::set_pcr(&mut buf, Pcr::from_27mhz(new_27mhz)).is_ok() {
            out(&buf);
        } else {
            out(packet);
        }
    }

    fn flush(&mut self, _model: &mut StreamModel, _out: &mut dyn FnMut(&[u8])) {
        // PCR restamp is stateless across packets beyond its per-PID anchors;
        // nothing is buffered, so there is nothing to flush.
    }
}
