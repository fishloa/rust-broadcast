//! PCR 33-bit base wrap — real-fixture test (ISO/IEC 13818-1 §2.4.3.5).
//!
//! Verifies that `PcrRestamp::Interpolate` correctly handles a legal 33-bit PCR
//! base wrap without treating it as a discontinuity or corrupt observation.
//!
//! The fixture `fixtures/ts/pcr-wrap.ts` has 5 packets on PID 0x0100 with PCR
//! bases: `2^33-9000, 2^33-6000, 2^33-3000, 0, 3000` — each step is +3000 base
//! ticks, wrapping the 33-bit base between packet 2 and packet 3.
//! `discontinuity_indicator = 0` on every packet.
//!
//! Pre-fix: the wrap caused the `obs > anchor.last_obs_27mhz` check to fail,
//! so packet 3 was recomputed from the anchor instead of being preserved as a
//! small wrapped value, producing a bogus discontinuity.
//!
//! Post-fix: the wrap-aware forward-distance check (`fwd = obs.wrapping_sub(last)
//! % PCR_27MHZ_MODULUS`) recognises the small forward step and preserves the
//! observed wrapped value. The unrolled output sequence is monotonically
//! increasing by a constant per-packet step (900_000 27 MHz ticks = 3000 × 300).

use ts_fix::{PcrRestamp, TsFix};

const PKT: usize = 188;
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/pcr-wrap.ts");

/// Read the 27 MHz PCR value from a TS packet's adaptation field.
fn pcr_27mhz(p: &[u8]) -> Option<u64> {
    let afc = (p[3] >> 4) & 0x3;
    if afc != 2 && afc != 3 {
        return None;
    }
    if *p.get(4)? == 0 {
        return None;
    }
    if *p.get(5)? & 0x10 == 0 {
        return None;
    }
    let b = p.get(6..12)?;
    let base = ((b[0] as u64) << 25)
        | ((b[1] as u64) << 17)
        | ((b[2] as u64) << 9)
        | ((b[3] as u64) << 1)
        | ((b[4] as u64) >> 7);
    let ext = (((b[4] & 0x01) as u16) << 8) | (b[5] as u16);
    Some(base * 300 + ext as u64)
}

/// PCR 27 MHz modulus: 33-bit base × 300.
const PCR_27MHZ_MODULUS: u64 = (1u64 << 33) * 300;

fn pid(p: &[u8]) -> u16 {
    (((p[1] & 0x1f) as u16) << 8) | p[2] as u16
}

/// Unroll a PCR sequence across the PCR modulus for monotonicity checking.
fn unroll_pcrs(pcrs: &[u64]) -> Vec<u64> {
    let mut out = Vec::with_capacity(pcrs.len());
    for &v in pcrs {
        let prev = *out.last().unwrap_or(&0);
        if v >= prev % PCR_27MHZ_MODULUS {
            out.push(prev + (v - (prev % PCR_27MHZ_MODULUS)));
        } else {
            out.push(prev + (PCR_27MHZ_MODULUS - (prev % PCR_27MHZ_MODULUS)) + v);
        }
    }
    out
}

#[test]
fn interpolate_preserves_33bit_pcr_base_wrap() {
    let input = std::fs::read(FIXTURE).unwrap_or_else(|e| panic!("fixture {FIXTURE}: {e}"));
    assert_eq!(input.len(), 5 * PKT, "fixture must have exactly 5 packets");

    // Verify the input has the expected wrap pattern.
    let in_pcrs: Vec<u64> = input.chunks_exact(PKT).filter_map(pcr_27mhz).collect();
    assert_eq!(in_pcrs.len(), 5, "all 5 packets carry a PCR");

    // The wrap should be visible in the raw 27 MHz values: a drop between
    // index 2 (near max) and index 3 (near 0).
    assert!(
        in_pcrs[3] < 300_000,
        "packet 3 (post-wrap) PCR 27mhz should be small, got {}",
        in_pcrs[3]
    );
    assert!(
        in_pcrs[2] > PCR_27MHZ_MODULUS - 1_000_000,
        "packet 2 (pre-wrap) PCR 27mhz should be near modulus max"
    );

    // Run through Interpolate restamp.
    let mut fix = TsFix::builder()
        .restamp_pcr(PcrRestamp::interpolate())
        .build()
        .expect("build");
    let mut out = Vec::with_capacity(input.len());
    for p in input.chunks_exact(PKT) {
        let _ = fix.push(p, |o| out.extend_from_slice(o));
    }
    fix.finish(|o| out.extend_from_slice(o));
    assert_eq!(out.len(), input.len(), "packet count preserved");

    // Extract output PCRs.
    let out_pcrs: Vec<u64> = out
        .chunks_exact(PKT)
        .filter(|p| pid(p) == 0x0100)
        .filter_map(pcr_27mhz)
        .collect();
    assert_eq!(out_pcrs.len(), 5, "all 5 output PCRs present");

    // THE KEY ASSERTION: The wrapped packet (index 3) preserves the small
    // wrapped value. If the pre-fix bug is present, it would be recomputed
    // from the anchor into a large pre-wrap value (~ max - something).
    assert!(
        out_pcrs[3] < 300_000,
        "post-wrap PCR (idx 3) must preserve the wrapped value, got {}",
        out_pcrs[3]
    );

    // Unroll across the modulus: the sequence must be monotonically increasing
    // by a constant step (900_000 27 MHz ticks per packet).
    let unrolled = unroll_pcrs(&out_pcrs);
    for w in unrolled.windows(2) {
        assert!(
            w[1] > w[0],
            "unrolled PCRs must be monotonically increasing: {} !> {}",
            w[1],
            w[0]
        );
    }
    // All steps should be equal (900000 = 3000 base * 300).
    let steps: Vec<u64> = unrolled.windows(2).map(|w| w[1] - w[0]).collect();
    for (i, &step) in steps.iter().enumerate() {
        assert_eq!(
            step, 900_000,
            "step[{i}] should be 900_000 (3000 base × 300), got {step}"
        );
    }
}
