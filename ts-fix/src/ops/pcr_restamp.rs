//! PCR restamp operation.
//!
//! Recomputes the 42-bit Program Clock Reference, **per PCR PID**, using a
//! timing model based on bitrate (robust repair) or interpolation between
//! observed PCRs (best-effort smoothing). A transport stream may carry several
//! programs, each with its own PCR PID; every PCR PID is restamped
//! independently from its own anchor.
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
//! # Spec
//!
//! ISO/IEC 13818-1 (= ITU-T H.222.0) §2.4.3.5 (PCR semantics).

use alloc::collections::BTreeMap;

use mpeg_ts::owned::OwnedTsPacket;
use mpeg_ts::ts::{Pcr, TsPacket, TS_PACKET_SIZE};

use crate::ops::{Op, StreamModel};

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
}

/// PCR restamp operation — restamps every PCR PID independently.
pub(crate) struct PcrRestampOp {
    anchors: BTreeMap<u16, Anchor>,
    mode: PcrRestamp,
}

impl PcrRestampOp {
    pub(crate) fn new(mode: PcrRestamp) -> Self {
        Self {
            anchors: BTreeMap::new(),
            mode,
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

    /// Read `(pid, pcr)` if this packet carries a PCR.
    fn read_pcr(packet: &[u8]) -> Option<(u16, Pcr)> {
        let pkt = TsPacket::parse(packet).ok()?;
        let pcr = pkt
            .adaptation_field()
            .and_then(|r| r.ok())
            .and_then(|af| af.pcr)?;
        Some((pkt.header.pid, pcr))
    }
}

impl Op for PcrRestampOp {
    fn process(&mut self, packet: &[u8], model: &mut StreamModel, out: &mut dyn FnMut(&[u8])) {
        if packet.len() != TS_PACKET_SIZE {
            out(packet);
            return;
        }
        let Some((pid, current)) = Self::read_pcr(packet) else {
            out(packet);
            return;
        };
        let now = model.packet_count;

        // First PCR on this PID → anchor, preserve as-is.
        let Some(anchor) = self.anchors.get_mut(&pid) else {
            let a = Anchor {
                anchor_27mhz: current.as_27mhz(),
                anchor_pkt: now,
                last_obs_pkt: now,
                last_obs_27mhz: current.as_27mhz(),
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
            }
            PcrRestamp::Interpolate => {
                // Derive the rate from the last monotonic observation on this PID.
                let obs = current.as_27mhz();
                let pkt_delta = now.saturating_sub(anchor.last_obs_pkt);
                if obs > anchor.last_obs_27mhz && pkt_delta > 0 {
                    // Sane forward observation: trust it, advance the anchor's
                    // observation window (jitter smoothing keeps observed values).
                    anchor.last_obs_pkt = now;
                    anchor.last_obs_27mhz = obs;
                    obs
                } else {
                    // Non-monotonic / corrupt observation: recompute from the
                    // anchor using the last known rate.
                    let span_pkt = anchor.last_obs_pkt.saturating_sub(anchor.anchor_pkt).max(1);
                    let span_ticks = anchor.last_obs_27mhz.saturating_sub(anchor.anchor_27mhz);
                    let rate = (span_ticks / span_pkt).max(1);
                    let delta = now.saturating_sub(anchor.anchor_pkt);
                    anchor.anchor_27mhz.wrapping_add(rate * delta)
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
