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

use crate::support::pcr_27mhz;
use mpeg_ts::pid::Pid;
use ts_fix::{PcrRestamp, TsFix};

const PKT: usize = 188;
const FIXTURE: &str = "ts/pcr-wrap.ts";

/// PCR 27 MHz modulus: 33-bit base × 300.
const PCR_27MHZ_MODULUS: u64 = (1u64 << 33) * 300;

fn pid(p: &[u8]) -> Pid {
    Pid::from_wire_masked((((p[1] & 0x1f) as u16) << 8) | p[2] as u16)
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
    let input = std::fs::read(test_fixtures::path(FIXTURE))
        .unwrap_or_else(|e| panic!("fixture {FIXTURE}: {e}"));
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
        .filter(|p| pid(p) == Pid::from_const(0x0100))
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
