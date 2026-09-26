//! Independent-oracle test for `dvb-tools pids` bitrate estimation
//! (issue #1100 / audit W-DT-1, plus its PCR-wrap follow-up).
//!
//! `fixtures/france-tnt-pcr.ts` (workspace root) is a real multi-program
//! capture carrying five independent PCR PIDs (0x0078, 0x00DC, 0x026C,
//! 0x0208, 0x02D0 — one per service, ISO/IEC 13818-1 §2.4.4.9). Before the
//! fix, `pids.rs` tracked the *first* PCR seen on any PID and the *last* PCR
//! seen on any PID, which on this fixture compares two unrelated PCR clocks
//! and reports "n/a" (see the pre-fix run recorded below) instead of an
//! estimate.
//!
//! `dvb-tools/tests/fixtures/france-tnt-pcr-wrap.ts` isolates PID 0x00DC
//! alone (`tsp -P filter --pid 0x00DC`, so no *other* PCR PID can mask the
//! bug by acting as a fallback candidate) from the same capture, then
//! shifts its PCR values (via TSDuck's `pcredit --add-pcr`, which performs
//! correct modular PCR arithmetic) so they cross the 27 MHz PCR wrap
//! (`2^33 * 300` ticks, ISO/IEC 13818-1 §2.4.2.2/§2.4.3.5, ~26.5h) partway
//! through the file — see `tests/fixtures/README.md` for the exact
//! commands. Before the wrap-unwrap fix, comparing raw (wrapped)
//! first/last 27 MHz values on this fixture makes `last_v > first_v` false
//! and `estimate_bitrate_mbps` reports "n/a" even though a single PCR PID
//! was already correctly selected (verified: an earlier attempt that
//! shifted PID 0x00DC within the *original* 5-PCR-PID file didn't reliably
//! reproduce the bug, because the PID-selection fix simply fell back to a
//! different, still-unwrapped PCR PID and still reported a coincidentally
//! close bitrate).
//!
//! Oracle: TSDuck 3.44 `tsanalyze`'s own bitrate estimate for each
//! *unshifted* capture, independent of our parser and our own PCR unwrap:
//! ```text
//! $ tsanalyze fixtures/france-tnt-pcr.ts
//! Selected reference bitrate: ............. 24,323,948 b/s   (188 bytes/pkt)
//! $ tsanalyze france-tnt-pcr-pid00dc-only.ts   # PID 0x00DC isolated
//! Selected reference bitrate: ..............  4,113,084 b/s   (188 bytes/pkt)
//! ```
//! (the wrap fixture is a pure PCR-domain shift of the same isolated PID's
//! timestamps — packet count, sizes and spacing are untouched — so its real
//! bitrate is unchanged and the second oracle value still applies).

use std::process::{Command, Stdio};

fn fixture(rel: &str) -> String {
    format!("{}{}", env!("CARGO_MANIFEST_DIR"), rel)
}

fn run(args: &[&str]) -> (bool, String) {
    let bin = env!("CARGO_BIN_EXE_dvb-tools");
    let output = Command::new(bin)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn dvb-tools");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// tsanalyze's "Selected reference bitrate" (188 bytes/pkt basis) for
/// `fixtures/france-tnt-pcr.ts`, recorded verbatim from its own report.
const ORACLE_BPS: f64 = 24_323_948.0;

/// tsanalyze's "Selected reference bitrate" for the PID-0x00DC-only stream
/// `france-tnt-pcr-wrap.ts` is built from (before the PCR shift), recorded
/// verbatim from its own report.
const WRAP_FIXTURE_ORACLE_BPS: f64 = 4_113_084.0;

/// Run `pids` on `path` and assert its reported bitrate is within 1% of
/// `oracle_mbps`.
fn assert_bitrate_matches_oracle(path: &str, oracle_mbps: f64) {
    let (ok, stderr) = run(&["pids", path]);
    assert!(ok, "pids {path} failed: stderr={stderr}");

    let line = stderr
        .lines()
        .find(|l| l.contains("bitrate="))
        .unwrap_or_else(|| panic!("no bitrate line in stderr: {stderr}"));
    assert!(
        !line.contains("n/a"),
        "bitrate estimate was n/a instead of a number: {line}"
    );
    let mbps_str = line
        .split("bitrate=")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .unwrap_or_else(|| panic!("could not parse bitrate value from: {line}"));
    let mbps: f64 = mbps_str
        .parse()
        .unwrap_or_else(|e| panic!("bitrate {mbps_str:?} not a float: {e}"));

    let rel_err = (mbps - oracle_mbps).abs() / oracle_mbps;
    assert!(
        rel_err < 0.01,
        "bitrate {mbps} Mbit/s not within 1% of tsanalyze oracle {oracle_mbps} Mbit/s \
         (rel_err={rel_err})"
    );
}

#[test]
fn pids_bitrate_matches_tsanalyze_oracle_within_1_percent() {
    let path = fixture("/../fixtures/france-tnt-pcr.ts");
    assert_bitrate_matches_oracle(&path, ORACLE_BPS / 1_000_000.0);
}

#[test]
fn pids_bitrate_across_pcr_wrap_matches_tsanalyze_oracle_within_1_percent() {
    let path = fixture("/tests/fixtures/france-tnt-pcr-wrap.ts");
    assert_bitrate_matches_oracle(&path, WRAP_FIXTURE_ORACLE_BPS / 1_000_000.0);
}
