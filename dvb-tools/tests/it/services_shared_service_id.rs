//! Independent-oracle test for `dvb-tools services` keying (issue #1100 /
//! audit W-DT-2).
//!
//! `tests/fixtures/sdt_nit_two_ts_shared_service_id.ts` (see that
//! directory's README for the exact `tstabcomp`/`tsp` generation command)
//! is a synthetic capture built from TSDuck-compiled tables: two
//! transport streams — `(onid=1, tsid=6)` and `(onid=1, tsid=7)` — each
//! carry a service numbered `service_id = 0x0301` (ETSI EN 300 468
//! §5.2.2/§5.2.3: `service_id` is only unique within one
//! `(original_network_id, transport_stream_id)`), with distinct names and
//! distinct NIT-assigned LCNs (101 for TS 6, 202 for TS 7).
//!
//! Oracle: TSDuck's own `tstables` decode of the same fixture (independent
//! of our parser) confirms both services and both LCNs are present. Before
//! the fix, `services.rs` keyed its maps by `service_id` alone, so the
//! second SDT entry silently overwrote the first in the `services` map and
//! `lookup_lcn` matched whichever LCN entry was inserted last — collapsing
//! two distinct services into one.

use std::process::{Command, Stdio};

fn fixture(rel: &str) -> String {
    format!("{}{}", env!("CARGO_MANIFEST_DIR"), rel)
}

fn run(args: &[&str]) -> (bool, String, String) {
    let bin = env!("CARGO_BIN_EXE_dvb-tools");
    let output = Command::new(bin)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn dvb-tools");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn services_keeps_same_service_id_on_different_transport_streams_distinct() {
    let path = fixture("/tests/fixtures/sdt_nit_two_ts_shared_service_id.ts");
    let (ok, stdout, stderr) = run(&["services", &path]);
    assert!(ok, "services fixture failed: stderr={stderr}");

    // TSDuck `tstables` (independent decode) confirms:
    //   SDT actual (onid=1, tsid=6): service 0x0301 "Service A"
    //   SDT other  (onid=1, tsid=7): service 0x0301 "Service B"
    //   NIT actual: tsid=6 -> LCN 101, tsid=7 -> LCN 202
    assert!(
        stdout.contains("Service A") && stdout.contains("Service B"),
        "both same-numbered services from different TSs must be listed, \
         got stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("tsid=0x0006") && stdout.contains("tsid=0x0007"),
        "output must disambiguate by transport_stream_id, got stdout:\n{stdout}"
    );

    let line_a = stdout
        .lines()
        .find(|l| l.contains("Service A"))
        .expect("Service A line");
    let line_b = stdout
        .lines()
        .find(|l| l.contains("Service B"))
        .expect("Service B line");
    assert!(
        line_a.contains("LCN  101") && line_a.contains("tsid=0x0006"),
        "Service A must carry its own TS's LCN (101): {line_a}"
    );
    assert!(
        line_b.contains("LCN  202") && line_b.contains("tsid=0x0007"),
        "Service B must carry its own TS's LCN (202), not Service A's: {line_b}"
    );

    // -- services=2 ... : the pre-fix map collapsed the two same-id
    // services into one entry.
    assert!(
        stderr.contains("services=2"),
        "expected both services counted distinctly, got stderr:\n{stderr}"
    );
}
