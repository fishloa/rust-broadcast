//! PCR restamp — real-fixture fault-inject test (ISO/IEC 13818-1 §2.4.3.5).
//!
//! Oracle: the shared `fixtures/france-tnt-pcr.ts` — a 2688-packet slice of a
//! real French TNT mux carrying 24 PCRs across 5 PCR PIDs. We corrupt the PCRs
//! on the busiest PCR PID (keeping the first as the anchor), run
//! `restamp_pcr(from_bitrate)`, and assert the output PCRs are strictly
//! increasing and within tolerance of the original timeline — `from_bitrate`
//! recomputes them from packet position, ignoring the corruption.
//!
//! The test BITES: an identity pass-through leaves the corrupted, non-monotonic
//! PCRs (see `identity_leaves_corrupted_pcrs_nonmonotonic`).

use ts_fix::{PcrRestamp, TsFix};

const PKT: usize = 188;
// Shared workspace fixtures live one level up from the crate root (read at
// runtime via std::fs, so `cargo publish` — which never runs integration tests
// — is unaffected by the out-of-crate path).
const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/france-tnt-pcr.ts");

fn pid(p: &[u8]) -> u16 {
    (((p[1] & 0x1f) as u16) << 8) | p[2] as u16
}

/// `Some(pcr_27mhz)` if this packet carries a PCR in its adaptation field.
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

fn set_pcr_27mhz(p: &mut [u8], v: u64) {
    let base = v / 300;
    let ext = (v % 300) as u16;
    p[6] = (base >> 25) as u8;
    p[7] = (base >> 17) as u8;
    p[8] = (base >> 9) as u8;
    p[9] = (base >> 1) as u8;
    p[10] = (((base & 1) as u8) << 7) | 0x7e | (((ext >> 8) & 1) as u8);
    p[11] = (ext & 0xff) as u8;
}

fn load() -> Vec<u8> {
    std::fs::read(FIXTURE).unwrap_or_else(|e| panic!("fixture {FIXTURE}: {e}"))
}

/// The PCR PID with the most PCRs, and its (packet_index, pcr_27mhz) series.
fn busiest_pcr_pid(buf: &[u8]) -> (u16, Vec<(usize, u64)>) {
    use std::collections::BTreeMap;
    let mut by_pid: BTreeMap<u16, Vec<(usize, u64)>> = BTreeMap::new();
    for (i, p) in buf.chunks_exact(PKT).enumerate() {
        if let Some(v) = pcr_27mhz(p) {
            by_pid.entry(pid(p)).or_default().push((i, v));
        }
    }
    by_pid
        .into_iter()
        .max_by_key(|(_, v)| v.len())
        .expect("fixture must carry PCRs")
}

/// Zero every PCR on `pcr_pid` except the one at `keep_index` (the anchor).
fn corrupt_pcrs(buf: &mut [u8], pcr_pid: u16, keep_index: usize) {
    for (idx, p) in buf.chunks_exact_mut(PKT).enumerate() {
        if pid(p) == pcr_pid && pcr_27mhz(p).is_some() && idx != keep_index {
            set_pcr_27mhz(p, 0);
        }
    }
}

fn run<F: FnOnce(ts_fix::TsFixBuilder) -> ts_fix::TsFixBuilder>(input: &[u8], cfg: F) -> Vec<u8> {
    let mut fix = cfg(TsFix::builder()).build().expect("build");
    let mut out = Vec::with_capacity(input.len());
    for p in input.chunks_exact(PKT) {
        let _ = fix.push(p, |o| out.extend_from_slice(o));
    }
    fix.finish(|o| out.extend_from_slice(o));
    out
}

#[test]
fn from_bitrate_repairs_corrupted_pcrs_on_real_capture() {
    let original = load();
    let (pcr_pid, series) = busiest_pcr_pid(&original);
    assert!(series.len() >= 3, "need >=3 PCRs; got {}", series.len());

    // Derive the real bitrate from first..last original PCR on this PID.
    let (first_i, first_pcr) = series[0];
    let (last_i, last_pcr) = *series.last().unwrap();
    let pkt_span = (last_i - first_i) as u64;
    let pcr_span = last_pcr - first_pcr;
    let bps = (pkt_span * 188 * 8 * 27_000_000) / pcr_span;
    assert!(
        (1_000_000..100_000_000).contains(&bps),
        "sane bitrate, got {bps}"
    );

    let mut corrupted = original.clone();
    corrupt_pcrs(&mut corrupted, pcr_pid, first_i);

    // The corruption must break monotonicity, else the test cannot bite.
    let corr: Vec<u64> = corrupted
        .chunks_exact(PKT)
        .filter(|p| pid(p) == pcr_pid)
        .filter_map(pcr_27mhz)
        .collect();
    assert!(
        corr.windows(2).any(|w| w[1] < w[0]),
        "corruption must break monotonicity"
    );

    let out = run(&corrupted, |b| b.restamp_pcr(PcrRestamp::from_bitrate(bps)));
    assert_eq!(out.len(), corrupted.len(), "packet count preserved");

    let repaired: Vec<u64> = out
        .chunks_exact(PKT)
        .filter(|p| pid(p) == pcr_pid)
        .filter_map(pcr_27mhz)
        .collect();
    assert_eq!(repaired.len(), series.len(), "same PCR count out");
    for w in repaired.windows(2) {
        assert!(
            w[1] > w[0],
            "repaired PCRs strictly increasing: {} !> {}",
            w[1],
            w[0]
        );
    }
    // Within ~1ms + a per-index packet-time of the original (ticks/pkt rounding accrues).
    let ticks_per_pkt = 188 * 8 * 27_000_000 / bps;
    for (i, ((_, orig), got)) in series.iter().zip(repaired.iter()).enumerate() {
        let tol = 27_000 + ticks_per_pkt * (i as u64 + 2);
        assert!(
            orig.abs_diff(*got) <= tol,
            "PCR[{i}] off by {} (tol {tol})",
            orig.abs_diff(*got)
        );
    }
}

#[test]
fn identity_leaves_corrupted_pcrs_nonmonotonic() {
    // Proves the repair test bites: without restamp, the zeroed PCRs survive.
    let original = load();
    let (pcr_pid, series) = busiest_pcr_pid(&original);
    let mut corrupted = original.clone();
    corrupt_pcrs(&mut corrupted, pcr_pid, series[0].0);
    let out = run(&corrupted, |b| b); // no ops
    let pcrs: Vec<u64> = out
        .chunks_exact(PKT)
        .filter(|p| pid(p) == pcr_pid)
        .filter_map(pcr_27mhz)
        .collect();
    assert!(
        pcrs.windows(2).any(|w| w[1] < w[0]),
        "identity must leave the corruption"
    );
}
