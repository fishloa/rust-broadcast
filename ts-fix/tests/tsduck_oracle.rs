//! TSDuck as an independent oracle for `--regen-psi` (issue #1037).
//!
//! `ts-fix/tests/psi_regen.rs` validates `regen_psi`'s output with our own
//! `dvb_si::tables::pat::PatSection::parse` — the same crate whose builder
//! (`PatSection`/`SectionPacketiser`) `regen_psi` itself calls to construct
//! that output. A shared misreading of ISO/IEC 13818-1 (e.g. what
//! `continuity_counter` or `version_number` are supposed to do) would pass
//! both sides. TSDuck 3.44 (`tsanalyze`, `tsp -P tables`) is an independent
//! implementation and is used here as a genuine second opinion, exactly as
//! `media-doctor/tests/mediastreamvalidator_oracle.rs` uses Apple's own
//! `mediastreamvalidator` for HLS.
//!
//! ## Fixture: `tests/fixtures/pat-with-nit.ts`
//!
//! None of the committed fixtures under `fixtures/ts/` carry a `network_pid`
//! entry in their PAT (checked all of them with `tsp -P tables --pid 0`).
//! Per the W4 fixture-first fallback (no independent tool authors a whole
//! *broadcast capture* with a chosen NIT PID from scratch), this fixture is
//! `fixtures/ts/h264_aac.ts` (a real, already-committed capture) transformed
//! by TSDuck's own `pat` plugin — not our code — to add a `network_pid`
//! entry:
//!
//! ```text
//! tsp -I file fixtures/ts/h264_aac.ts -P pat --nit 16 -O file tests/fixtures/pat-with-nit.ts
//! ```
//!
//! (TSDuck 3.44-4676.) The result is still a real capture's actual PES/PSI
//! content; only the PAT gained one entry, via TSDuck, independent of our
//! code entirely. `tsanalyze --normalized` confirms the input fixture itself
//! has `discontinuities=0`/`duplicated=0` on every PID — the checks below are
//! about what `regen_psi` produces, not a pre-existing input defect.
//!
//! ## Pre-fix verification (not part of this file)
//!
//! Run manually against the pre-fix `ops/psi_regen.rs` (`git show
//! HEAD:ts-fix/src/ops/psi_regen.rs` swapped in over the working copy, then
//! restored — no `git stash`, which is shared across concurrent sessions in
//! a worktree): `tsanalyze --normalized` on the regenerated output reported
//! `pid=0: duplicated=25` (continuity_counter stuck at 0 across all 26
//! regenerated PAT packets) and `tsp -P tables --pid 0` showed a 16-byte PAT
//! section with the `network_pid` entry gone — both exactly the bugs this
//! test now gates.

use std::process::Command;

use ts_fix::TsFix;

/// `true` iff both TSDuck tools this file needs are on `PATH`.
fn tsduck_available() -> bool {
    Command::new("tsanalyze")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
        && Command::new("tsp")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
}

/// Skip this test cleanly, but LOUDLY: a silently-skipped oracle reads as a
/// pass, which is worse than no oracle at all (same rationale as
/// `media-doctor`'s `mediastreamvalidator_oracle.rs`).
macro_rules! skip_unless_tsduck_available {
    () => {
        if !tsduck_available() {
            eprintln!(
                "SKIP tsduck_oracle: `tsp`/`tsanalyze` not on PATH (TSDuck 3.44+; \
                 see CLAUDE.md's command list). This test is a no-op result on this \
                 host, not real coverage — install TSDuck to get the genuine check."
            );
            return;
        }
    };
}

fn fixture_path() -> String {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/pat-with-nit.ts"
    )
    .to_string()
}

/// Run the fixture through `regen_psi` and write the result to a fresh temp
/// file, returning its path. The file is not cleaned up on panic (left for
/// post-mortem inspection); it lives under the OS temp dir either way.
fn run_regen_psi_to_temp_file() -> std::path::PathBuf {
    let input = std::fs::read(fixture_path()).expect("pat-with-nit.ts fixture must be present");

    let mut engine = TsFix::builder()
        .regen_psi()
        .build()
        .expect("engine build must not fail");
    let mut output = Vec::with_capacity(input.len());
    for chunk in input.chunks_exact(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .expect("valid 188-byte packet");
    }
    engine.finish(|pkt| output.extend_from_slice(pkt));

    let out_path = std::env::temp_dir().join(format!(
        "ts-fix-tsduck-oracle-{}-{}.ts",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&out_path, &output).expect("write regen_psi output to temp file");
    out_path
}

/// Parse one `key=value` field out of a `tsanalyze --normalized` `pid:` line.
fn field(line: &str, key: &str) -> Option<i64> {
    line.split(':')
        .find_map(|kv| kv.strip_prefix(&format!("{key}=")))
        .and_then(|v| v.parse().ok())
}

/// Run `tsanalyze --normalized <path>` and return the `pid:` line for `pid`.
fn normalized_pid_line(path: &std::path::Path, pid: u16) -> String {
    let output = Command::new("tsanalyze")
        .arg("--normalized")
        .arg(path)
        .output()
        .expect("tsanalyze must run");
    assert!(
        output.status.success(),
        "tsanalyze failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find(|l| l.starts_with("pid:") && field(l, "pid") == Some(pid as i64))
        .unwrap_or_else(|| panic!("no tsanalyze pid: line for PID {pid} in:\n{text}"))
        .to_string()
}

/// Run `tsp -P tables --pid 0 --text-output -` and return its stdout (the
/// human-readable PAT dump).
fn dump_pat_table(path: &std::path::Path) -> String {
    let output = Command::new("tsp")
        .args(["-I", "file"])
        .arg(path)
        .args(["-P", "tables", "--pid", "0", "--text-output", "/dev/stdout"])
        .args(["-O", "drop"])
        .output()
        .expect("tsp must run");
    assert!(
        output.status.success(),
        "tsp failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// `tsanalyze` reports zero continuity anomalies on the PAT PID after
/// `regen_psi` — the pre-fix bug (a fresh `SectionPacketiser`, so
/// `continuity_counter` restarted at 0 on every regenerated PAT packet)
/// showed up there as `duplicated=25` (TSDuck's bucket for same-PID,
/// same-CC repeats — the CC never advanced across 26 emitted PAT packets).
#[test]
fn regen_psi_pat_continuity_is_clean_per_tsanalyze() {
    skip_unless_tsduck_available!();

    let out_path = run_regen_psi_to_temp_file();
    let pat_line = normalized_pid_line(&out_path, 0);

    assert_eq!(
        field(&pat_line, "discontinuities"),
        Some(0),
        "tsanalyze reported PAT continuity discontinuities: {pat_line}"
    );
    assert_eq!(
        field(&pat_line, "duplicated"),
        Some(0),
        "tsanalyze reported duplicated (stuck continuity_counter) PAT packets: {pat_line}"
    );

    let _ = std::fs::remove_file(&out_path);
}

/// `regen_psi` never touches the PMT PID's own packets, so its continuity
/// must be exactly as clean after regen as it was in the source fixture —
/// a non-regression check alongside the PAT one above.
#[test]
fn regen_psi_pmt_continuity_is_clean_per_tsanalyze() {
    skip_unless_tsduck_available!();

    const PMT_PID: u16 = 4096; // fixtures/ts/h264_aac.ts's program_map_PID.
    let out_path = run_regen_psi_to_temp_file();
    let pmt_line = normalized_pid_line(&out_path, PMT_PID);

    assert_eq!(
        field(&pmt_line, "discontinuities"),
        Some(0),
        "tsanalyze reported PMT continuity discontinuities: {pmt_line}"
    );
    assert_eq!(
        field(&pmt_line, "duplicated"),
        Some(0),
        "tsanalyze reported duplicated PMT packets: {pmt_line}"
    );

    let _ = std::fs::remove_file(&out_path);
}

/// `tsp -P tables` (an independent PSI decoder) confirms the regenerated PAT
/// still carries the `network_pid` entry from the original PAT — the
/// pre-fix bug dropped it (a `rebuild_pat` that only derives entries from
/// parsed PMT sections can never emit `program_number == 0`) — alongside the
/// still-correct program entry and a stable `version_number`.
#[test]
fn regen_psi_preserves_nit_and_version_per_tsp_tables() {
    skip_unless_tsduck_available!();

    let out_path = run_regen_psi_to_temp_file();
    let dump = dump_pat_table(&out_path);

    assert!(
        dump.contains("NIT:"),
        "tsp -P tables must show a NIT entry in the regenerated PAT, got:\n{dump}"
    );
    assert!(
        dump.contains("PID:   16 (0x0010)") || dump.contains("PID: 16 (0x0010)"),
        "the regenerated NIT entry must still point at PID 0x0010, got:\n{dump}"
    );
    assert!(
        dump.contains("Program:     1 (0x0001)  PID: 4096 (0x1000)"),
        "the regenerated PAT must still list program 1 -> PMT PID 0x1000, got:\n{dump}"
    );
    // Nothing in this fixture changes the discovered mapping mid-stream, so
    // version_number must stay at its initial value (0) throughout.
    assert!(
        dump.contains("Version: 0,"),
        "version_number must stay 0 when the mapping never changes, got:\n{dump}"
    );

    let _ = std::fs::remove_file(&out_path);
}
