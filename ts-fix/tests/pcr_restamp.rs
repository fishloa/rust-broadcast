//! PCR restamp tests for `ts-fix`.
//!
//! Fault-inject: generate a synthetic TS stream with known PCRs on PID 100,
//! zero the PCR field bytes, run restamp with a known bitrate, and verify:
//!
//! 1. Output PCRs are monotonic non-decreasing.
//! 2. The rewritten PCR field round-trips through `Pcr::to_field_bytes` + decode.
//! 3. Output PCRs match the expected timeline (within tolerance).
//! 4. The test BITES: identity pass-through leaves zeroed (non-monotonic flatline) PCRs.

use mpeg_ts::ts::{Pcr, TS_PACKET_SIZE};
use mpeg_ts::OwnedTsPacket;

/// Manually decode a PCR from the 6-byte adaptation-field slot.
fn decode_pcr_field(buf: &[u8]) -> Option<Pcr> {
    if buf.len() < 6 {
        return None;
    }
    let base = ((buf[0] as u64) << 25)
        | ((buf[1] as u64) << 17)
        | ((buf[2] as u64) << 9)
        | ((buf[3] as u64) << 1)
        | (((buf[4] as u64) >> 7) & 0x01);
    let ext_hi = ((buf[4] as u16) & 0x01) << 8;
    let ext_lo = buf[5] as u16;
    let extension = ext_hi | ext_lo;
    Some(Pcr { base, extension })
}

fn extract_pcr(pkt: &[u8]) -> Option<Pcr> {
    if pkt.len() < 12 {
        return None;
    }
    if pkt[3] & 0x20 == 0 {
        return None;
    }
    if pkt[4] < 1 {
        return None;
    }
    if pkt[5] & 0x10 == 0 {
        return None;
    }
    decode_pcr_field(&pkt[6..12])
}

fn has_pcr_flag(pkt: &[u8]) -> bool {
    pkt.len() >= 6 && (pkt[3] & 0x20) != 0 && pkt[4] >= 1 && (pkt[5] & 0x10) != 0
}

/// Build a synthetic stream of `count` packets on PID 100 with PCRs every
/// `pcr_interval` packets.
fn build_pcr_stream(count: usize, pcr_interval: usize, ticks_per_pkt: u64) -> Vec<u8> {
    let mut stream = Vec::with_capacity(count * 188);
    for i in 0..count {
        let pid = 100u16;
        let cc = (i & 0x0F) as u8;

        if i % pcr_interval == 0 {
            let pcr_27mhz = (i as u64) * ticks_per_pkt;
            let pcr = Pcr::from_27mhz(pcr_27mhz);

            let mut pkt = [0u8; TS_PACKET_SIZE];
            pkt[0] = 0x47;
            pkt[3] = 0x30 | cc;
            pkt[4] = 7;
            pkt[5] = 0x10;
            pkt[6..12].copy_from_slice(&pcr.to_field_bytes());
            for b in &mut pkt[12..] {
                *b = 0xFF;
            }
            stream.extend_from_slice(&pkt);
        } else {
            stream.extend_from_slice(&OwnedTsPacket::serialize_with_payload(pid, i == 0, cc, &[]));
        }
    }
    stream
}

#[test]
fn pcr_restamp_from_bitrate_recovers_monotonic() {
    let ticks_per_pkt = 1504u64;
    let bitrate = 27_000_000u64;
    let pcr_interval = 10usize;
    let input = build_pcr_stream(500, pcr_interval, ticks_per_pkt);

    // ── 1. Record original PCRs and compute ideal values ─────────────────
    let original_pcrs: Vec<Pcr> = input.chunks(188).filter_map(extract_pcr).collect();
    assert!(original_pcrs.len() >= 5);
    let ideal_pcrs: Vec<u64> = (0..original_pcrs.len())
        .map(|n| n as u64 * pcr_interval as u64 * ticks_per_pkt)
        .collect();

    // ── 2. Zero the PCR field bytes to create worst-case corruption ───────
    let zeroed: Vec<u8> = input
        .chunks(188)
        .flat_map(|chunk| {
            let mut buf = [0u8; TS_PACKET_SIZE];
            buf.copy_from_slice(chunk);
            if has_pcr_flag(&buf) {
                buf[6..12].fill(0);
            }
            buf.to_vec()
        })
        .collect();

    // Verify zeroing worked: all PCRs are now Pcr { base: 0, ext: 0 }
    let zeroed_pcrs: Vec<Pcr> = zeroed.chunks(188).filter_map(extract_pcr).collect();
    assert!(zeroed_pcrs.len() >= 2);
    for pcr in &zeroed_pcrs {
        assert_eq!(pcr.as_27mhz(), 0, "zeroed PCRs must all be 0");
    }

    // ── 3. Run PCR restamp with known bitrate ─────────────────────────────
    let mut engine = ts_fix::TsFix::builder()
        .restamp_pcr(ts_fix::PcrRestamp::from_bitrate(bitrate))
        .build()
        .expect("restamp_pcr build should not fail");

    let mut output = Vec::with_capacity(zeroed.len());
    for chunk in zeroed.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .expect("valid 188-byte packet");
    }
    engine.finish(|pkt| output.extend_from_slice(pkt));

    assert_eq!(output.len(), zeroed.len());
    assert_eq!(output.len() % 188, 0);

    // ── 4. Verify output PCRs are monotonic non-decreasing ────────────────
    let output_pcrs: Vec<Pcr> = output.chunks(188).filter_map(extract_pcr).collect();
    assert_eq!(output_pcrs.len(), original_pcrs.len());

    let mut output_27mhz: Vec<u64> = output_pcrs.iter().map(|p| p.as_27mhz()).collect();
    output_27mhz.dedup();
    for pair in output_27mhz.windows(2) {
        assert!(
            pair[0] <= pair[1],
            "output PCRs must be monotonic non-decreasing: {} > {}",
            pair[0],
            pair[1]
        );
    }

    // ── 5. Verify round-trip via to_field_bytes ───────────────────────────
    for chunk in output.chunks(188) {
        if let Some(pcr) = extract_pcr(chunk) {
            let bytes = pcr.to_field_bytes();
            let reparsed = decode_pcr_field(&bytes).expect("PCR round-trip");
            assert_eq!(
                pcr, reparsed,
                "PCR must round-trip through to_field_bytes + decode"
            );
        }
    }

    // ── 6. Verify PCRs match ideal values (within 5%) ─────────────────────
    for (n, out_pcr) in output_pcrs.iter().enumerate() {
        let out = out_pcr.as_27mhz();
        let ideal = ideal_pcrs[n];
        if n == 0 {
            assert_eq!(out, 0, "first PCR must be preserved (was zeroed to 0)");
            continue;
        }
        let ratio = (out as f64) / (ideal as f64);
        assert!(
            (ratio - 1.0).abs() < 0.05,
            "output PCR {out} at index {n} deviates from ideal {ideal} by >5% (ratio={ratio})"
        );
    }

    // ── 7. BITES: identity must preserve zeroed PCRs (flatline) ──────────
    let mut identity = ts_fix::TsFix::builder().build().expect("identity build");

    let mut ident_out = Vec::with_capacity(zeroed.len());
    for chunk in zeroed.chunks(188) {
        identity
            .push(chunk, |pkt| ident_out.extend_from_slice(pkt))
            .unwrap();
    }
    identity.finish(|pkt| ident_out.extend_from_slice(pkt));

    let ident_pcrs: Vec<Pcr> = ident_out.chunks(188).filter_map(extract_pcr).collect();
    assert!(
        ident_pcrs.iter().all(|p| p.as_27mhz() == 0),
        "FATAL: identity pass-through produced non-zero PCRs from zeroed input"
    );
}
