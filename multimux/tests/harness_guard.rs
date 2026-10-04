//! W2b-1 Task 10 (spec §5): guards THIS crate's test harness against the two
//! failure modes the W2b-1 harness work removed —
//!
//! - **Reserve-then-rebind ports** (`reserve_tcp_addr`/`reserve_udp_addr`/
//!   `free_tcp_addr`/`free_port`). The HTTP media/admin listeners now bind
//!   `127.0.0.1:0` and hand the live listener in (`serve_*_on`). The few
//!   remaining helpers exist because the address is a route-internal bind the
//!   test cannot otherwise observe (WHEP/WHIP/RTMP/UDP inputs, and an external
//!   `mediamtx` whose config takes a port) — allowlisted per file below.
//! - **Bare fixed sleeps** in `tests/**` outside the files that predate this
//!   task. A NEW sleep in a NEW test file fails here, so the pattern does not
//!   creep back. (Pacing sleeps inside a sender task, and hang-guard polls,
//!   are the allowlisted exception.)
//!
//! **This is a lexical tripwire, not a proof.** Code review is the real control.

use std::fs;
use std::path::Path;

/// Port-reservation helper names that must NOT be (re)introduced.
const PORT_HELPERS: &[&str] = &[
    "fn reserve_tcp_addr",
    "fn reserve_udp_addr",
    "fn free_tcp_addr",
    "fn free_port",
];

/// Files allowed to still define a port helper, with the reason the address is
/// genuinely unobservable without it (W2b-1 Task 4 deviation).
const PORT_HELPER_ALLOW: &[(&str, &str)] = &[
    (
        "push_rtsp.rs",
        "external `mediamtx` config takes a numeric port; port 0 is impossible for a third-party process",
    ),
    (
        "admin_api.rs",
        "the `whep_addr` is a route-internal WHEP listen the test must name",
    ),
    (
        "dispatch_ingest.rs",
        "the TsUdp/RTMP input address is a route-internal bind the test must name",
    ),
    (
        "smooth_oracle.rs",
        "the TsUdp input address is a route-internal bind the test must name; \
         `serve_smooth_until_fragment` retries the whole attempt on a lost race \
         (the safety net the old reserve-then-rebind loop provided)",
    ),
    (
        "ts_hls_oracle.rs",
        "the TsUdp input address is a route-internal bind the test must name; \
         `serve_ts_hls_until_extinf` retries the whole attempt on a lost race \
         (the safety net the old reserve-then-rebind loop provided)",
    ),
    (
        "whep_egress.rs",
        "the WHIP/WHEP listen addresses are route-internal binds the test must name",
    ),
    (
        "whip_ingest.rs",
        "the WHIP listen address is a route-internal bind the test must name",
    ),
];

/// Files allowed to contain a bare `tokio::time::sleep(` / `thread::sleep(`.
/// Each predates the W2b-1 harness work; a NEW file (or a new line in a file
/// not listed here) is a failure. `wait_for_rebind`'s `yield_now` is the
/// sanctioned bounded-wait helper.
const SLEEP_ALLOW: &[&str] = &[
    "accept_lifecycle.rs",
    "admin_api.rs",
    "dispatch_ingest.rs",
    "file_reader.rs",
    "file_route.rs",
    "glass_to_glass.rs",
    "golden_gate.rs",
    "lldash_dashjs.rs",
    "push_negotiate.rs",
    "push_rtsp.rs",
    "rtsp_ingest.rs",
    "server_timeouts.rs",
    "smooth_oracle.rs",
    "ts_hls_oracle.rs",
    "whep_egress.rs",
    "whip_ingest.rs",
    "whip_whep_timers.rs",
    // W2b-1 Task 8 test: its sleeps are TIMING, not synchronisation — they
    // give the running task time to reach the connect/backoff before
    // cancelling it, which a condition wait cannot observe. (`no_slot_park.rs`
    // is NOT listed: it runs under `start_paused` and no longer sleeps.)
    "shutdown_cancel.rs",
];

fn test_files() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut out = Vec::new();
    collect(&dir, &mut out);
    out
}

fn collect(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).expect("read tests dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push((
                path.file_name()
                    .expect("file name")
                    .to_string_lossy()
                    .into_owned(),
                fs::read_to_string(&path).expect("read test file"),
            ));
        }
    }
}

#[test]
fn no_new_reserve_then_rebind_port_helpers() {
    let mut hits = Vec::new();
    for (file, src) in test_files() {
        if file == "harness_guard.rs" {
            continue;
        }
        for needle in PORT_HELPERS {
            if src.contains(needle) && !PORT_HELPER_ALLOW.iter().any(|(f, _)| f == &file) {
                hits.push(format!("{file}: {needle}"));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "reserve-then-rebind port helper reintroduced (bind `127.0.0.1:0` and pass the live \
         listener via `serve_*_on`, or allowlist with a reason):\n{}",
        hits.join("\n")
    );
}

#[test]
fn no_new_bare_sleeps_in_the_test_harness() {
    let mut hits = Vec::new();
    for (file, src) in test_files() {
        if file == "harness_guard.rs" || SLEEP_ALLOW.contains(&file.as_str()) {
            continue;
        }
        for (n, line) in src.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            if line.contains("tokio::time::sleep(")
                || line.contains("thread::sleep(")
                || line.contains("std::thread::sleep(")
            {
                hits.push(format!("{file}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "a bare fixed sleep was introduced in the test harness (use a condition wait or a \
         paused clock; if it is genuinely pacing, add the file to SLEEP_ALLOW with a reason):\n{}",
        hits.join("\n")
    );
}
