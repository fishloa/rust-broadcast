//! PCR restamp operation.
//!
//! Recomputes the 42-bit Program Clock Reference on the PCR PID using a
//! timing model based on packet position, bitrate, or interpolation between
//! observed PCRs.
//!
//! # Forward-compat note
//!
//! This module sets the PCR **in-place** via
//! [`mpeg_ts::OwnedTsPacket::set_pcr`], which overwrites the existing 6-byte
//! PCR field inside the adaptation field without re-serialising the whole
//! adaptation field.  The timing model is stored in
//! [`crate::ops::TimingContext`] so that v0.2's PTS/DTS-wrap op can reuse the
//! same clock state.
//!
//! # Spec
//!
//! ISO/IEC 13818-1 (= ITU-T H.222.0) §2.4.3.5 (PCR semantics).

use mpeg_ts::owned::OwnedTsPacket;
use mpeg_ts::ts::{Pcr, TsHeader, TsPacket, TS_PACKET_SIZE};

use crate::ops::{Op, StreamModel};

/// PCR restamp mode.
///
/// `#[non_exhaustive]` — new modes (e.g. `from_external_clock`) may be added
/// in future releases without a breaking change.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum PcrRestamp {
    /// Interpolate PCRs between observed PCR anchors.
    ///
    /// The first PCR on the PCR PID is preserved as an anchor. Between anchors,
    /// PCR values are computed from the packet offset and accumulated 27 MHz
    /// ticks derived from the inter-anchor rate.
    Interpolate,
    /// Recompute PCRs from a fixed bitrate (bits per second).
    ///
    /// PCR = `(packet_index × 188 × 8 / bitrate) × 27_000_000`
    FromBitrate {
        /// Bitrate in bits per second.
        bps: u64,
    },
}

impl PcrRestamp {
    /// Interpolate PCRs between observed anchors.
    ///
    /// The first PCR on the PCR PID is preserved; subsequent PCRs are
    /// interpolated from the inter-anchor packet count and delta-PCR.
    ///
    /// # Example
    /// ```
    /// use ts_fix::PcrRestamp;
    /// let cfg = PcrRestamp::interpolate();
    /// ```
    pub fn interpolate() -> Self {
        Self::Interpolate
    }

    /// Recompute PCRs from a fixed bitrate.
    ///
    /// `bps` is the stream bitrate in bits per second (e.g. `27_000_000` for
    /// 27 Mbit/s).
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

/// PCR restamp operation.
///
/// Reads the existing PCR from each adaptation-field-bearing packet on the
/// PCR PID, recomputes it according to the configured mode, and writes it
/// back in-place.
pub(crate) struct PcrRestampOp {
    /// The PCR PID — determined from the first packet with a PCR.
    pcr_pid: Option<u16>,
    /// Configuration mode.
    mode: PcrRestamp,
}

impl PcrRestampOp {
    pub(crate) fn new(mode: PcrRestamp) -> Self {
        Self {
            pcr_pid: None,
            mode,
        }
    }

    /// Determine whether a packet carries a PCR by examining its adaptation field.
    fn has_pcr(packet: &[u8]) -> bool {
        // Quick header check without full parse.
        if packet.len() < 5 {
            return false;
        }
        // Adaptation field control must have adaptation (bit 5).
        if packet[3] & 0x20 == 0 {
            return false;
        }
        let af_len = packet[4] as usize;
        if af_len < 1 {
            return false;
        }
        // PCR flag in adaptation field flags byte (byte 5, bit 4).
        packet[5] & 0x10 != 0
    }
}

impl Op for PcrRestampOp {
    fn process(&mut self, packet: &[u8], model: &mut StreamModel, out: &mut dyn FnMut(&[u8])) {
        // Quick check: does this packet have a PCR at all?
        if !Self::has_pcr(packet) {
            out(packet);
            return;
        }

        // Determine PCR PID from first PCR-bearing packet.
        let header = match TsHeader::parse(&packet[..4]) {
            Ok(h) => h,
            Err(_) => {
                out(packet);
                return;
            }
        };

        let pid = header.pid;
        if self.pcr_pid.is_none() {
            self.pcr_pid = Some(pid);
        }
        if self.pcr_pid != Some(pid) {
            // Not the PCR PID — pass through.
            out(packet);
            return;
        }

        // Parse full packet to extract current PCR.
        let ts_pkt = match TsPacket::parse(packet) {
            Ok(p) => p,
            Err(_) => {
                out(packet);
                return;
            }
        };

        let current_pcr = match ts_pkt
            .adaptation_field()
            .and_then(|r| r.ok())
            .and_then(|af| af.pcr)
        {
            Some(pcr) => pcr,
            None => {
                out(packet);
                return;
            }
        };

        let new_pcr = match &self.mode {
            PcrRestamp::Interpolate => {
                if !model.timing.has_anchor {
                    // First PCR on this PID — establish anchor.
                    model.timing.clock_27mhz = current_pcr.as_27mhz();
                    model.timing.last_pcr_base = current_pcr.base;
                    model.timing.last_pcr_ext = current_pcr.extension;
                    model.timing.has_anchor = true;
                    model.timing.anchor_packet_index = model.packet_count;
                    model.timing.prev_packet_index = model.packet_count;

                    // Update bitrate estimate from the first PCR (no delta yet).
                    model.timing.interpolated_bitrate = None;

                    current_pcr // preserve as-is
                } else {
                    // We have an anchor: compute time delta from the anchor.
                    let packet_delta = model.packet_count - model.timing.anchor_packet_index;

                    // Compute the inter-anchor PCR rate from the two *current* PCRs
                    // (the one we just read and the one we last wrote).
                    //
                    // If we have enough data, update bitrate estimate.
                    let pcr_delta_27mhz = current_pcr
                        .as_27mhz()
                        .wrapping_sub(model.timing.clock_27mhz);
                    let pkt_delta_from_prev = model.packet_count - model.timing.prev_packet_index;

                    if pkt_delta_from_prev > 0 && pcr_delta_27mhz > 0 {
                        // Ticks per packet = pcr_delta / pkt_delta.
                        // Convert to bits/sec: (ticks_per_pkt * 8 * 188) / 27_000_000.
                        let ticks_per_pkt = pcr_delta_27mhz / pkt_delta_from_prev;
                        // Avoid division by zero / overflow.
                        if ticks_per_pkt > 0 && ticks_per_pkt < 27_000_000 {
                            // bits/sec = (27 MHz ticks per sec / ticks per pkt) * 188 * 8
                            // ≈ (27_000_000 / ticks_per_pkt) * 1504 (188*8)
                            // Only update if we have a reasonable estimate.
                            model.timing.interpolated_bitrate =
                                Some(27_000_000f64 / ticks_per_pkt as f64 * 1504.0);
                        }
                    }

                    // Interpolate: new PCR = anchor_pcr + (packet_delta × ticks_per_packet)
                    // Use the interpolated bitrate if available, else fall back
                    // to the current PCR's delta.
                    let new_ticks = if packet_delta > 0 {
                        if let Some(rate_bps) = model.timing.interpolated_bitrate {
                            // clock_advance = packet_delta * (1504 bytes/pkt / bitrate_bps) * 27 MHz
                            let bytes_per_pkt = 188f64;
                            let secs_per_pkt = bytes_per_pkt * 8.0 / rate_bps;
                            let ticks_per_pkt = (secs_per_pkt * 27_000_000.0) as u64;
                            ticks_per_pkt * packet_delta
                        } else {
                            // Fall back to using the delta from previous PCR.
                            // This happens before we have two PCRs to compute rate.
                            model.timing.clock_27mhz = current_pcr.as_27mhz();
                            model.timing.last_pcr_base = current_pcr.base;
                            model.timing.last_pcr_ext = current_pcr.extension;
                            model.timing.prev_packet_index = model.packet_count;
                            // Don't compute a new PCR — use the current one.
                            out(packet);
                            return;
                        }
                    } else {
                        // Same packet index (shouldn't happen, but be safe).
                        out(packet);
                        return;
                    };

                    let new_27mhz = model.timing.clock_27mhz.wrapping_add(new_ticks);

                    let new_pcr = Pcr::from_27mhz(new_27mhz);

                    // Update state.
                    model.timing.clock_27mhz = new_27mhz;
                    model.timing.last_pcr_base = new_pcr.base;
                    model.timing.last_pcr_ext = new_pcr.extension;
                    model.timing.prev_packet_index = model.packet_count;

                    new_pcr
                }
            }
            PcrRestamp::FromBitrate { bps } => {
                if *bps == 0 {
                    // Invalid bitrate; pass through unchanged.
                    out(packet);
                    return;
                }

                if !model.timing.has_anchor {
                    // First PCR — establish anchor.
                    model.timing.clock_27mhz = current_pcr.as_27mhz();
                    model.timing.last_pcr_base = current_pcr.base;
                    model.timing.last_pcr_ext = current_pcr.extension;
                    model.timing.has_anchor = true;
                    model.timing.anchor_packet_index = model.packet_count;
                    model.timing.prev_packet_index = model.packet_count;
                    model.timing.interpolated_bitrate = Some(*bps as f64);

                    current_pcr // preserve first PCR as-is
                } else {
                    // PCR = anchor_pcr + (packet_delta × ticks_per_packet)
                    let packet_delta = model.packet_count - model.timing.anchor_packet_index;
                    if packet_delta == 0 {
                        out(packet);
                        return;
                    }

                    // Ticks per 188-byte packet at this bitrate:
                    // ticks = (188 * 8 / bps) * 27_000_000 = 188 * 8 * 27_000_000 / bps
                    let num = 188u64 * 8 * 27_000_000u64;
                    let ticks_per_packet = if *bps >= num {
                        1 // minimum 1 tick to avoid zero
                    } else {
                        (num / *bps).max(1)
                    };

                    // Compute from anchor each time, keeping clock_27mhz as the
                    // anchor value (it is set once on first PCR and never changed).
                    let anchor_27mhz = model.timing.clock_27mhz;
                    let new_27mhz = anchor_27mhz.wrapping_add(ticks_per_packet * packet_delta);
                    let new_pcr = Pcr::from_27mhz(new_27mhz);

                    // Update state: keep clock_27mhz as the anchor (unchanged).
                    model.timing.last_pcr_base = new_pcr.base;
                    model.timing.last_pcr_base = new_pcr.base;
                    model.timing.last_pcr_ext = new_pcr.extension;
                    model.timing.prev_packet_index = model.packet_count;

                    new_pcr
                }
            }
        };

        // Write the new PCR in-place via mpeg-ts editor.
        let mut buf = [0u8; TS_PACKET_SIZE];
        buf.copy_from_slice(packet);
        match OwnedTsPacket::set_pcr(&mut buf, new_pcr) {
            Ok(()) => out(&buf),
            Err(_) => {
                // Fall back: pass through unchanged if set_pcr fails.
                out(packet);
            }
        }
    }

    fn flush(&mut self, _model: &mut StreamModel, _out: &mut dyn FnMut(&[u8])) {
        // Nothing buffered.
    }
}
