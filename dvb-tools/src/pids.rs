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

use mpeg_ts::ts::{TsPacket, TS_PACKET_SIZE};

use crate::util::{for_each_packet, read_file};

/// PCR clock frequency on which PCR_27mhz ticks (ISO/IEC 13818-1 §2.4.3.5).
const PCR_CLOCK_HZ: u64 = 27_000_000;

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

pub fn run(path: &str) -> ExitCode {
    let data = match read_file(path, "dvb-tools pids") {
        Ok(d) => d,
        Err(code) => return code,
    };

    let mut counts: HashMap<u16, u64> = HashMap::new();
    let mut total_packets: u64 = 0;

    // (packet_index_of_first_pcr, pcr_27mhz) when first seen.
    let mut first_pcr: Option<(u64, u64)> = None;
    let mut last_pcr: Option<(u64, u64)> = None;
    let mut pcr_pid: Option<u16> = None;

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

        if let Some(Ok(af)) = parsed.adaptation_field() {
            if let Some(pcr) = af.pcr {
                let pcr_27 = pcr.as_27mhz();
                if first_pcr.is_none() {
                    first_pcr = Some((total_packets, pcr_27));
                    pcr_pid = Some(pid);
                }
                last_pcr = Some((total_packets, pcr_27));
            }
        }
    }

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
}
