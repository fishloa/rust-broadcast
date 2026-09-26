//! `dvb-tools pids <file.ts>` — PID table + bitrate estimate.
//!
//! For each aligned 188-byte packet, parses the TS header and tallies packets
//! per PID. The bitrate is estimated from the PCRs carried in adaptation
//! fields: difference_in_packets × 188 × 8, scaled by 27 MHz /
//! delta_pcr_27mhz.
//!
//! PCR is the *Program Clock Reference* (ISO/IEC 13818-1 §2.4.3.5). The
//! bitrate over the file is a lower-bound estimate — accurate to a single
//! constant bitrate carry; useful as a sanity check, not a measurement.
use std::collections::HashMap;
use std::process::ExitCode;

use mpeg_ts::ts::{TS_PACKET_SIZE, TsPacket};

use crate::util::{for_each_packet, read_file};

/// PCR clock frequency on which PCR_27mhz ticks (ISO/IEC 13818-1 §2.4.3.5).
const PCR_CLOCK_HZ: u64 = 27_000_000;

/// The PCR modulus on the 27 MHz clock: the 33-bit `program_clock_reference_base`
/// (90 kHz) wraps every `2^33` ticks (ISO/IEC 13818-1 §2.4.2.2), and each base
/// tick spans 300 27 MHz ticks (§2.4.3.5's `base * 300 + extension`), so the
/// combined 27 MHz value wraps every `2^33 * 300` ticks (~26.5 hours). No
/// crate this one already depends on exports this constant (it is not the
/// same as `broadcast_common::clock33::WRAP_33BIT`, the 90 kHz PTS/DTS
/// modulus), so it is named here.
const PCR_MODULUS_27MHZ: u128 = (1u128 << 33) * 300;
/// Half the modulus — the threshold distinguishing a genuine wrap from an
/// out-of-order/duplicate PCR (mirrors `broadcast_common::clock33`'s
/// half-modulus convention).
const PCR_MODULUS_27MHZ_HALF: u128 = PCR_MODULUS_27MHZ / 2;

/// Bits carried by one 188-byte TS packet (`TS_PACKET_SIZE * 8`).
const BITS_PER_PACKET: u64 = (TS_PACKET_SIZE as u64) * 8;

/// One per-PID row in the output table.
#[derive(Clone)]
struct PidRow {
    /// 13-bit PID value.
    pid: u16,
    /// Number of packets observed on this PID.
    packets: u64,
}

/// Estimate the multiplex bitrate (Mbit/s) from the first and last observed
/// PCR, each a `(packet_index, pcr_27mhz)` pair. The elapsed wall-clock between
/// them is `(pcr_last − pcr_first) / 27 MHz` seconds, carrying
/// `(idx_last − idx_first)` packets, so
/// `bitrate = packets · 188 · 8 · 27 MHz / Δpcr`. Returns `None` when fewer than
/// two PCRs were seen, or when the PCR did not advance (wrap / duplicate),
/// which would make the estimate meaningless.
fn estimate_bitrate_mbps(first: Option<(u64, u64)>, last: Option<(u64, u64)>) -> Option<f64> {
    match (first, last) {
        (Some((first_idx, first_v)), Some((last_idx, last_v)))
            if last_v > first_v && last_idx > first_idx =>
        {
            let packets_between = last_idx - first_idx;
            let delta = last_v - first_v;
            let bps =
                (packets_between * BITS_PER_PACKET) as f64 * (PCR_CLOCK_HZ as f64) / (delta as f64);
            Some(bps / 1_000_000.0)
        }
        _ => None,
    }
}

/// Per-PID PCR tracking: first/last `(packet_index, pcr_27mhz)` seen on
/// *this* PID only, plus a sample count. A multiplex carries one PCR PID
/// per program (ISO/IEC 13818-1 §2.4.4.9), each running its own independent
/// clock — mixing first/last across two different PCR PIDs compares two
/// unrelated clocks and produces a meaningless (or spuriously "wrapped")
/// delta. Tracking per-PID keeps the estimate on a single, consistent clock.
///
/// `first`/`last` store **unwrapped** 27 MHz values (ever-growing, not
/// reduced mod [`PCR_MODULUS_27MHZ`]) so a capture whose PCR crosses the
/// ~26.5h wrap still yields a monotonic `last - first`; `prev_raw` /
/// `prev_unwrapped` are the running state [`PcrTrack::push`] needs to detect
/// the next wrap.
#[derive(Default)]
struct PcrTrack {
    first: Option<(u64, u64)>,
    last: Option<(u64, u64)>,
    samples: u64,
    prev_raw: Option<u64>,
    prev_unwrapped: u64,
}

impl PcrTrack {
    /// Fold in one more PCR sample (`packet_index`, raw wire-order 27 MHz
    /// value), correcting for a single forward wrap since the previous
    /// sample on this same PID.
    fn push(&mut self, packet_index: u64, pcr_27: u64) {
        let unwrapped = match self.prev_raw {
            None => pcr_27,
            Some(prev_raw) => {
                let mut delta = i128::from(pcr_27) - i128::from(prev_raw);
                // A delta that looks like a huge backward jump is actually a
                // forward wrap past `PCR_MODULUS_27MHZ`; a huge forward jump
                // (the symmetric case) is an out-of-order/duplicate PCR, not
                // a legitimate wrap, so it is left uncorrected (the caller's
                // `estimate_bitrate_mbps` monotonicity check rejects it).
                if delta < -(PCR_MODULUS_27MHZ_HALF as i128) {
                    delta += PCR_MODULUS_27MHZ as i128;
                }
                (i128::from(self.prev_unwrapped) + delta) as u64
            }
        };
        if self.first.is_none() {
            self.first = Some((packet_index, unwrapped));
        }
        self.last = Some((packet_index, unwrapped));
        self.prev_raw = Some(pcr_27);
        self.prev_unwrapped = unwrapped;
        self.samples += 1;
    }
}

pub fn run(path: &str) -> ExitCode {
    let data = match read_file(path, "dvb-tools pids") {
        Ok(d) => d,
        Err(code) => return code,
    };

    let mut counts: HashMap<u16, u64> = HashMap::new();
    let mut total_packets: u64 = 0;
    let mut pcr_tracks: HashMap<u16, PcrTrack> = HashMap::new();

    for (idx, packet) in for_each_packet(&data).enumerate() {
        total_packets = idx as u64 + 1;
        // Every `for_each_packet` chunk should be parseable (sync-byte checked,
        // length is exactly 188 bytes). Keep going on a parse error instead
        // of failing the whole CLI over one malformed packet.
        let Ok(parsed) = TsPacket::parse(&packet) else {
            continue;
        };
        let pid = parsed.header.pid;
        *counts.entry(pid).or_insert(0) += 1;

        if let Some(Ok(af)) = parsed.adaptation_field()
            && let Some(pcr) = af.pcr
        {
            // `as_27mhz()` unwraps the 33-bit base / 9-bit extension pair
            // into a single 27 MHz tick count (ISO/IEC 13818-1 §2.4.3.5);
            // `PcrTrack::push` separately unwraps that value's own ~26.5h
            // rollover (`PCR_MODULUS_27MHZ`, §2.4.2.2) across samples.
            let pcr_27 = pcr.as_27mhz();
            pcr_tracks
                .entry(pid)
                .or_default()
                .push(total_packets, pcr_27);
        }
    }

    // Pick one PCR PID and use only its own first/last PCR. Prefer the PID
    // with the most PCR samples (the one that ran across the largest span
    // of the capture), breaking ties on ascending PID for determinism.
    let mut best: Option<(u16, &PcrTrack)> = None;
    for (pid, track) in &pcr_tracks {
        if estimate_bitrate_mbps(track.first, track.last).is_none() {
            continue;
        }
        best = Some(match best {
            Some((best_pid, best_track))
                if best_track.samples > track.samples
                    || (best_track.samples == track.samples && best_pid <= *pid) =>
            {
                (best_pid, best_track)
            }
            _ => (*pid, track),
        });
    }
    let (first_pcr, last_pcr, pcr_pid) = match best {
        Some((pid, track)) => (track.first, track.last, Some(pid)),
        None => (None, None, None),
    };

    if total_packets == 0 {
        eprintln!("dvb-tools pids: no packets found");
        return ExitCode::SUCCESS;
    }

    // Build the per-PID row set and sort by descending packet count, breaking
    // ties on PID ascending so the output is stable.
    let mut rows: Vec<PidRow> = counts
        .into_iter()
        .map(|(pid, packets)| PidRow { pid, packets })
        .collect();
    rows.sort_by(|a, b| b.packets.cmp(&a.packets).then(a.pid.cmp(&b.pid)));

    for row in &rows {
        let pct = (row.packets as f64) * 100.0 / (total_packets as f64);
        println!(
            "pid=0x{:04X}  packets={}  {:.2}%",
            row.pid, row.packets, pct
        );
    }

    let bitrate_mbps = estimate_bitrate_mbps(first_pcr, last_pcr);

    let pcr_label = match (bitrate_mbps, pcr_pid) {
        (Some(mbps), Some(pid)) => format!("{mbps:.2} Mbit/s (PCR from pid 0x{pid:04X})"),
        _ => "n/a (<=1 PCR seen)".to_string(),
    };
    eprintln!("-- total_packets={total_packets}  bitrate={pcr_label}");
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitrate_from_two_pcrs() {
        // 10_000 packets carried across exactly one second of PCR (27 MHz):
        // 10_000 * 188 * 8 bits / 1 s = 15.04 Mbit/s.
        let first = Some((10, 0));
        let last = Some((10_010, PCR_CLOCK_HZ));
        let mbps = estimate_bitrate_mbps(first, last).expect("two valid PCRs");
        assert!(
            (mbps - 15.04).abs() < 1e-9,
            "expected 15.04 Mbit/s, got {mbps}"
        );
    }

    #[test]
    fn bitrate_none_with_fewer_than_two_pcrs() {
        assert_eq!(estimate_bitrate_mbps(None, None), None);
        // A single PCR sets first == last (same index + value) → not estimable.
        assert_eq!(estimate_bitrate_mbps(Some((5, 100)), Some((5, 100))), None);
    }

    #[test]
    fn bitrate_none_when_pcr_does_not_advance() {
        // PCR wrapped or duplicated (last_v <= first_v) → meaningless, reject.
        assert_eq!(
            estimate_bitrate_mbps(Some((10, 500)), Some((20, 400))),
            None
        );
        assert_eq!(
            estimate_bitrate_mbps(Some((10, 500)), Some((20, 500))),
            None
        );
    }

    // Coordinator follow-up on W-DT-1: a PCR wrap (2^33 * 300 27 MHz ticks,
    // ISO/IEC 13818-1 §2.4.2.2/§2.4.3.5) must not make `PcrTrack` see a
    // spurious backward step. Spec-derived worked example: two samples 5
    // ticks apart that straddle the wrap (last raw value near the top of
    // the range, next raw value small) must unwrap to a small *forward*
    // step, not report `last < first`.
    #[test]
    fn pcr_track_unwraps_a_forward_wrap() {
        let mut track = PcrTrack::default();
        let near_top = (PCR_MODULUS_27MHZ - 3) as u64;
        track.push(1, near_top);
        track.push(2, 2); // wraps forward by 5 ticks: near_top -> 0 -> 2
        let (first_idx, first_v) = track.first.expect("first sample recorded");
        let (last_idx, last_v) = track.last.expect("last sample recorded");
        assert_eq!(first_idx, 1);
        assert_eq!(last_idx, 2);
        assert!(
            last_v > first_v,
            "unwrapped last ({last_v}) must exceed unwrapped first ({first_v}) across a wrap"
        );
        assert_eq!(
            last_v - first_v,
            5,
            "wrap must unwrap to a 5-tick forward step"
        );
    }

    #[test]
    fn pcr_track_no_wrap_is_unaffected() {
        let mut track = PcrTrack::default();
        track.push(1, 1_000);
        track.push(2, 1_100);
        assert_eq!(track.first, Some((1, 1_000)));
        assert_eq!(track.last, Some((2, 1_100)));
    }
}
