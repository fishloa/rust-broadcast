//! `PcrRestamp::Interpolate` must keep ADOPTING observed PCRs after a legal
//! 33-bit PCR wrap (ISO/IEC 13818-1 §2.4.3.5), not just survive the wrap
//! packet itself.
//!
//! `tests/it/pcr_wrap.rs` only checks a constant-rate stream, where the old
//! `obs.wrapping_sub(last) % (2^33 * 300)` form passes by coincidence: the
//! forward distance came out as garbage (2^64 is not a multiple of the PCR
//! modulus), the packet was treated as "not a sane forward step" and
//! RECOMPUTED from the anchor's extrapolated rate, which for a constant-rate
//! stream equals the observation. Here the post-wrap step is 3100 base ticks
//! against 3000 before the wrap, so extrapolating the pre-wrap rate (the bug)
//! no longer reproduces the observed values, while correctly adopting them
//! does. The fixture's first packet is the template; PCRs are rewritten with
//! `mpeg_ts`'s own setter.

use mpeg_ts::owned::OwnedTsPacket;
use mpeg_ts::pid::Pid;
use mpeg_ts::ts::Pcr;
use ts_fix::{PcrRestamp, TsFix};

const PKT: usize = 188;
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/pcr-wrap.ts");
const WRAP_BASE: u64 = 1 << 33;

fn pcr_27mhz(p: &[u8]) -> u64 {
    let b = &p[6..12];
    let base = ((b[0] as u64) << 25)
        | ((b[1] as u64) << 17)
        | ((b[2] as u64) << 9)
        | ((b[3] as u64) << 1)
        | ((b[4] as u64) >> 7);
    let ext = (((b[4] & 0x01) as u64) << 8) | b[5] as u64;
    base * 300 + ext
}

#[test]
fn interpolate_adopts_observations_after_the_wrap() {
    let fixture = std::fs::read(FIXTURE).unwrap_or_else(|e| panic!("fixture {FIXTURE}: {e}"));
    let template: [u8; PKT] = fixture[..PKT].try_into().unwrap();

    // Pre-wrap +3000 base per packet, post-wrap +3100.
    let bases = [
        WRAP_BASE - 9000,
        WRAP_BASE - 6000,
        WRAP_BASE - 3000,
        0,
        3100,
        6200,
        9300,
    ];
    let mut input = Vec::new();
    for b in bases {
        let mut p = template;
        OwnedTsPacket::set_pcr(&mut p, Pcr::from_27mhz(b * 300)).unwrap();
        input.extend_from_slice(&p);
    }

    let mut fix = TsFix::builder()
        .restamp_pcr(PcrRestamp::interpolate())
        .build()
        .unwrap();
    let mut out = Vec::new();
    for p in input.chunks_exact(PKT) {
        let _ = fix.push(p, |o| out.extend_from_slice(o));
    }
    fix.finish(|o| out.extend_from_slice(o));

    let pid = |p: &[u8]| Pid::new((u16::from(p[1] & 0x1F) << 8) | u16::from(p[2]));
    let want = pid(&template);
    let out_pcrs: Vec<u64> = out
        .chunks_exact(PKT)
        .filter(|p| pid(p) == want)
        .map(pcr_27mhz)
        .collect();
    let in_pcrs: Vec<u64> = input.chunks_exact(PKT).map(pcr_27mhz).collect();
    assert_eq!(out_pcrs.len(), in_pcrs.len());
    // Every post-wrap packet is a sane forward observation (+3100 base) and
    // must be preserved, not replaced by the pre-wrap rate extrapolation.
    for i in 3..in_pcrs.len() {
        assert_eq!(
            out_pcrs[i], in_pcrs[i],
            "packet {i}: the observed post-wrap PCR must be adopted"
        );
    }
}
