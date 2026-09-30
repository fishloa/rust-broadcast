//! Real-fixture tests for `PcrCheck` — TR 101 290 PCR diagnostics on genuine
//! broadcast captures (false-positive check + discontinuity honouring).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use media_doctor::{Diagnostic, PcrCheck, Report, Severity};

fn read(rel: &str) -> Vec<u8> {
    let path = format!("{}/../fixtures/{}", env!("CARGO_MANIFEST_DIR"), rel);
    fs::read(&path).unwrap_or_else(|e| panic!("read fixture {path}: {e}"))
}

fn pcr_errors(report: &Report) -> Vec<&media_doctor::Finding> {
    report
        .findings()
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .collect()
}

/// A clean real multi-PCR-PID broadcast capture must produce no PCR *errors*
/// (the false-positive check synthetic packets can't give).
#[test]
fn pcr_clean_real_stream_no_errors() {
    let ts = read("france-tnt-pcr.ts");
    let mut report = Report::new();
    PcrCheck.run(&ts, &mut report);
    let errs = pcr_errors(&report);
    assert!(
        errs.is_empty(),
        "clean france-tnt-pcr.ts should yield no PCR errors, got {}: {:?}",
        errs.len(),
        errs
    );
}

/// `france-pcr-discontinuity.ts` carries a *signalled* system-time-base
/// discontinuity (discontinuity_indicator=1) + a +10s PCR jump on PID 0x208.
/// A correct PcrCheck honours the flag and does NOT raise an error — if it
/// flagged the jump this would fail (the bite).
#[test]
fn pcr_signalled_discontinuity_not_flagged() {
    let ts = read("ts/france-pcr-discontinuity.ts");
    let mut report = Report::new();
    PcrCheck.run(&ts, &mut report);
    let errs = pcr_errors(&report);
    assert!(
        errs.is_empty(),
        "signalled discontinuity must not be flagged as a PCR error, got {}: {:?}",
        errs.len(),
        errs
    );
}

// -------------------------------------------------------------------------
// Issue #1112 review: 2.3a/2.3b against the TR 101 290 definition
// -------------------------------------------------------------------------

/// TR 101 290 v1.4.1 Table 5.0b, transcribed at
/// `dvb-conformance/docs/tr_101_290.md:51-52`:
///
/// - **2.3a `PCR_repetition_error`**: "Time interval between two consecutive
///   PCR values more than 100 ms" (note 2: the 40 ms limitation was removed
///   in 2005).
/// - **2.3b `PCR_discontinuity_indicator_error`**: "The difference between
///   two consecutive PCR values (PCR_(i+1) - PCR_i) is outside the range of
///   0...100 ms without the discontinuity_indicator set".
const TR290_PCR_REPETITION_MS: f64 = 100.0;
const TR290_PCR_DISCONTINUITY_MS: f64 = 100.0;

/// One PID's PCR measurements, taken by a re-parse that shares no code with
/// `PcrCheck`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PcrStats {
    /// Largest **forward** step between consecutive PCRs, in milliseconds.
    max_forward_ms: f64,
    /// Largest backward step, in milliseconds (0 when none).
    max_backward_ms: f64,
    /// Whether any PCR-bearing packet on the PID set the discontinuity
    /// indicator.
    discontinuity_indicator_seen: bool,
    /// Number of PCRs seen on the PID.
    pcr_count: usize,
}

/// Independently re-derive every PID's PCR statistics from the raw TS packet
/// lattice: walk 188-byte packets, read the 6-byte PCR field of any
/// adaptation field, and take the modular step between consecutive PCRs.
///
/// This is the oracle the assertions below are written against — it uses only
/// `mpeg-ts`'s packet/adaptation-field accessors (the same layer a demuxer
/// would), never `media_doctor::PcrCheck`.
fn pcr_stats(ts: &[u8]) -> Vec<(u16, PcrStats)> {
    const PCR_MODULUS_27MHZ: u128 = (1u128 << 33) * 300;
    const CLOCK_27MHZ: f64 = 27_000_000.0;
    const TS_PACKET_SIZE: usize = 188;

    let mut per_pid: BTreeMap<u16, Vec<(u64, bool)>> = BTreeMap::new();
    for packet in ts.chunks_exact(TS_PACKET_SIZE) {
        let Ok(pkt) = mpeg_ts::ts::TsPacket::parse(packet) else {
            continue;
        };
        if !pkt.header.has_adaptation {
            continue;
        }
        let Some(Ok(af)) = pkt.adaptation_field() else {
            continue;
        };
        let Some(pcr) = af.pcr else { continue };
        per_pid
            .entry(pkt.header.pid)
            .or_default()
            .push((pcr.as_27mhz(), af.discontinuity_indicator));
    }

    let mut out = Vec::new();
    for (pid, seq) in per_pid {
        let mut max_forward_ms = 0.0f64;
        let mut max_backward_ms = 0.0f64;
        let mut disc_seen = false;
        for (a, b) in seq.iter().zip(seq.iter().skip(1)) {
            disc_seen = disc_seen || a.1 || b.1;
            let step = (u128::from(b.0) + PCR_MODULUS_27MHZ - u128::from(a.0)) % PCR_MODULUS_27MHZ;
            if step <= PCR_MODULUS_27MHZ / 2 {
                max_forward_ms = max_forward_ms.max(step as f64 * 1000.0 / CLOCK_27MHZ);
            } else {
                let back = (PCR_MODULUS_27MHZ - step) as f64 * 1000.0 / CLOCK_27MHZ;
                max_backward_ms = max_backward_ms.max(back);
            }
        }
        out.push((
            pid,
            PcrStats {
                max_forward_ms,
                max_backward_ms,
                discontinuity_indicator_seen: disc_seen,
                pcr_count: seq.len(),
            },
        ));
    }
    out
}

/// Every committed `.ts` fixture in the workspace, as paths relative to
/// `CARGO_MANIFEST_DIR`.
///
/// Enumerated by walking the fixture directories rather than hard-coded, so a
/// newly committed capture is covered automatically.
fn every_ts_fixture() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut roots = vec![
        root.join("fixtures"),
        root.join("media-doctor/tests/fixtures"),
        root.join("scte35-splice/tests/fixtures"),
        root.join("transmux/tests/fixtures"),
    ];
    roots.retain(|p| p.exists());

    let mut found = Vec::new();
    let mut stack = roots;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "ts") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// The oracle's per-fixture verdict must match `PcrCheck`'s, for every
/// committed capture:
///
/// - a `pcr-discontinuity` **Error** exactly when some PID has a forward step
///   over 100 ms or any backward step, with no `discontinuity_indicator`;
/// - a `pcr-repetition` finding exactly when some PID has a forward step over
///   100 ms (and it is not already the 2.3b case above).
///
/// Per-fixture permission is granted only through [`ALLOWED`], each entry
/// explained. Everything else must be exactly right, in both directions — a
/// clean capture reporting an Error, and a violating capture reporting
/// nothing, are both failures.
#[test]
fn pcr_indicators_match_the_tr101290_definition_on_every_fixture() {
    let fixtures = every_ts_fixture();
    assert!(
        fixtures.len() > 40,
        "the fixture walk found only {} files — the roots are wrong and this          test would be vacuous",
        fixtures.len(),
    );

    let mut with_pcr = 0usize;
    let mut failures = Vec::new();

    for path in &fixtures {
        let rel = path
            .strip_prefix(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."))
            .unwrap_or(path)
            .display()
            .to_string();
        let Ok(ts) = fs::read(path) else {
            failures.push(format!("{rel}: unreadable"));
            continue;
        };

        let stats = pcr_stats(&ts);
        if stats.is_empty() {
            continue;
        }
        with_pcr += 1;

        // Oracle verdict.
        let expect_discontinuity = stats.iter().any(|(_, s)| {
            !s.discontinuity_indicator_seen
                && (s.max_forward_ms > TR290_PCR_DISCONTINUITY_MS || s.max_backward_ms > 0.0)
        });
        let expect_repetition = stats.iter().any(|(_, s)| {
            !s.discontinuity_indicator_seen
                && s.max_forward_ms > TR290_PCR_REPETITION_MS
                && s.max_forward_ms <= TR290_PCR_DISCONTINUITY_MS
        });

        let mut report = Report::new();
        PcrCheck.run(&ts, &mut report);
        let got_discontinuity = report
            .findings()
            .iter()
            .any(|f| f.rule_id == "pcr-discontinuity" && f.severity == Severity::Error);
        let got_repetition = report
            .findings()
            .iter()
            .any(|f| f.rule_id == "pcr-repetition");

        let allowed = ALLOWED.iter().find(|(name, _, _)| *name == rel);
        let (d_ok, r_ok) = match allowed {
            Some((_, ad, ar)) => (got_discontinuity == *ad, got_repetition == *ar),
            None => (
                got_discontinuity == expect_discontinuity,
                got_repetition == expect_repetition,
            ),
        };
        if !d_ok || !r_ok {
            failures.push(format!(
                "{rel}: oracle discontinuity={expect_discontinuity} repetition={expect_repetition},                  code discontinuity={got_discontinuity} repetition={got_repetition},                  stats={stats:?}",
            ));
        }
    }

    assert!(
        with_pcr > 15,
        "only {with_pcr} fixtures carry PCRs — the oracle is not looking at          enough data to be meaningful",
    );
    assert!(
        failures.is_empty(),
        "PcrCheck disagrees with the TR 101 290 definition on {} fixture(s):
{}",
        failures.len(),
        failures.join(
            "
"
        ),
    );
}

/// Fixtures whose `PcrCheck` verdict intentionally differs from the raw
/// oracle, with the reason. `(relative path, expect discontinuity, expect
/// repetition)`.
const ALLOWED: &[(&str, bool, bool)] = &[
    // `france-pcr-discontinuity.ts` sets `discontinuity_indicator` on the
    // +10 s jump itself, and the oracle only records that *some* packet on
    // the PID set the flag (it is a per-PID summary). The code decides per
    // packet, so the jump is correctly not flagged while the oracle's
    // summary says "seen". Both agree on the substance: a signalled
    // discontinuity is not an error.
    ("fixtures/ts/france-pcr-discontinuity.ts", false, false),
];

// -------------------------------------------------------------------------
// Synthetic cases the fixture corpus cannot express
// -------------------------------------------------------------------------

/// A real PCR value gap must still be reported. Built by replacing every
/// PCR-bearing packet between two PCRs of a clean capture with a PCR-free
/// packet on the null PID, so the surviving consecutive PCRs are genuinely
/// far apart in PCR time — not a synthetic tick offset.
#[test]
fn real_pcr_value_gap_is_still_reported() {
    let ts = read("ts/h264_aac.ts");
    const TS_PACKET_SIZE: usize = 188;

    let mut pcr_packets: Vec<usize> = Vec::new();
    for (i, packet) in ts.chunks_exact(TS_PACKET_SIZE).enumerate() {
        if (packet[3] >> 4) & 0x02 == 0 || packet[4] == 0 || packet[5] & 0x10 == 0 {
            continue;
        }
        pcr_packets.push(i);
    }
    assert!(pcr_packets.len() >= 12, "fixture must carry enough PCRs");

    let keep = [pcr_packets[0], pcr_packets[1], *pcr_packets.last().unwrap()];
    let mut out = Vec::with_capacity(ts.len());
    for (i, packet) in ts.chunks_exact(TS_PACKET_SIZE).enumerate() {
        if pcr_packets.contains(&i) && !keep.contains(&i) {
            let mut replacement = vec![0x47u8; TS_PACKET_SIZE];
            replacement[1] = 0x1F;
            replacement[2] = 0xFF;
            replacement[3] = 0x10;
            out.extend_from_slice(&replacement);
            continue;
        }
        out.extend_from_slice(packet);
    }

    let mut report = Report::new();
    PcrCheck.run(&out, &mut report);
    assert!(
        report
            .findings()
            .iter()
            .any(|f| f.rule_id == "pcr-discontinuity"),
        "a real PCR value gap with no discontinuity_indicator must be \
         reported as 2.3b; got {:?}",
        report.findings(),
    );
}

/// A step the spec assigns to 2.3b must **not** also be reported as 2.3a.
///
/// A +600 s jump spliced into a real capture is one discontinuity, not a
/// repetition fault: reporting it as both inflates the count and
/// misdescribes it.
#[test]
fn discontinuity_step_is_not_also_reported_as_repetition() {
    const TS_PACKET_SIZE: usize = 188;
    const PCR_MODULUS_27MHZ: u128 = (1u128 << 33) * 300;
    const CLOCK_27MHZ: u128 = 27_000_000;

    let ts = read("ts/pcr-wrap.ts");
    let mut out = ts.clone();
    let packets = out.len() / TS_PACKET_SIZE;
    let midpoint = packets / 2;
    let mut shifted = 0usize;

    for i in 0..packets {
        let base = i * TS_PACKET_SIZE;
        let afc = (out[base + 3] >> 4) & 0x03;
        if afc & 0x02 == 0 || out[base + 4] == 0 || out[base + 5] & 0x10 == 0 || i < midpoint {
            continue;
        }
        let b = u128::from(out[base + 6]) << 25
            | u128::from(out[base + 7]) << 17
            | u128::from(out[base + 8]) << 9
            | u128::from(out[base + 9]) << 1
            | u128::from(out[base + 10]) >> 7;
        let e = u128::from(out[base + 10] & 0x01) << 8 | u128::from(out[base + 11]);
        // +600 s, modulo the 33-bit x 300 clock.
        let v = (b * 300 + e + 600 * CLOCK_27MHZ) % PCR_MODULUS_27MHZ;
        let (nb, ne) = (v / 300, v % 300);
        out[base + 6] = ((nb >> 25) & 0xFF) as u8;
        out[base + 7] = ((nb >> 17) & 0xFF) as u8;
        out[base + 8] = ((nb >> 9) & 0xFF) as u8;
        out[base + 9] = ((nb >> 1) & 0xFF) as u8;
        out[base + 10] = ((nb & 1) as u8) << 7 | 0x7E | ((ne >> 8) & 1) as u8;
        out[base + 11] = (ne & 0xFF) as u8;
        shifted += 1;
    }
    assert!(
        shifted > 0,
        "the fixture must carry PCRs after the midpoint"
    );

    let mut report = Report::new();
    PcrCheck.run(&out, &mut report);
    let discontinuity: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "pcr-discontinuity")
        .collect();
    assert_eq!(
        discontinuity.len(),
        1,
        "the +600 s step is exactly one 2.3b discontinuity; got {:?}",
        report.findings(),
    );
    assert!(
        report
            .findings()
            .iter()
            .all(|f| f.rule_id != "pcr-repetition"),
        "a step already reported as 2.3b must not also be reported as 2.3a; \
         got {:?}",
        report.findings(),
    );
}

/// A backward PCR step with no `discontinuity_indicator` is 2.3b: the
/// definition is "outside the range of 0...100 ms", which a negative
/// difference is.
#[test]
fn backward_pcr_step_is_a_discontinuity() {
    use mpeg_ts::Pcr;

    const CLOCK_27MHZ: u64 = 27_000_000;

    fn pcr_packet(pid: u16, ticks: u64) -> Vec<u8> {
        let mut pkt = vec![0x47u8; 188];
        pkt[1] = ((pid >> 8) as u8) & 0x1F;
        pkt[2] = (pid & 0xFF) as u8;
        pkt[3] = 0x30; // adaptation + payload
        pkt[4] = 7; // adaptation_field_length: flags + 6-byte PCR
        pkt[5] = 0x10; // PCR_flag
        pkt[6..12].copy_from_slice(&Pcr::from_27mhz(ticks).to_field_bytes());
        pkt
    }

    // 1.0 s -> 1.05 s (a legal 50 ms forward step) -> 0.85 s (200 ms
    // backwards). Only the backward step is a violation.
    let ts = [
        pcr_packet(0x0100, CLOCK_27MHZ),
        pcr_packet(0x0100, CLOCK_27MHZ + CLOCK_27MHZ * 5 / 100),
        pcr_packet(0x0100, CLOCK_27MHZ * 85 / 100),
    ]
    .concat();

    let mut report = Report::new();
    PcrCheck.run(&ts, &mut report);
    let discontinuity: Vec<_> = report
        .findings()
        .iter()
        .filter(|f| f.rule_id == "pcr-discontinuity")
        .collect();
    assert_eq!(
        discontinuity.len(),
        1,
        "a 200 ms backward step with no discontinuity_indicator is one 2.3b \
         error; got {:?}",
        report.findings(),
    );
    // The backward step is the third packet (index 2). Assert on *that*
    // finding, not merely on there being one: without backward-step
    // detection the modular forward distance still exceeds the limit, but it
    // is reported as a ~26.5 h forward delta rather than as a backwards jump.
    let at_backward_step = discontinuity
        .iter()
        .find(|f| f.location.packet == 2)
        .expect("a finding on the packet carrying the backward step");
    assert!(
        at_backward_step.message.contains("backwards"),
        "the finding for the backward step must say the PCR went backwards;          got {:?}",
        at_backward_step.message,
    );
}

/// Cross-check the in-test oracle against a second, independent one: a Python
/// script that shares no code with this crate or with `mpeg-ts`, walking the
/// raw packet lattice itself.
///
/// Skips **loudly** (naming the reason) when `python3` is not on `PATH`, so
/// the test is a no-op on a machine without it but never a silent pass that
/// looks like a check.
#[test]
fn python_oracle_agrees_with_the_in_test_oracle() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/tools/max_pcr_step.py");
    if !script.exists() {
        eprintln!(
            "SKIP python_oracle_agrees_with_the_in_test_oracle: {} is missing",
            script.display()
        );
        return;
    }

    let fixtures = every_ts_fixture();
    assert!(!fixtures.is_empty(), "the fixture walk found nothing");

    let output = match std::process::Command::new("python3")
        .arg(&script)
        .args(&fixtures)
        .output()
    {
        Ok(out) => out,
        Err(e) => {
            eprintln!(
                "SKIP python_oracle_agrees_with_the_in_test_oracle: python3 not runnable ({e})"
            );
            return;
        }
    };
    assert!(
        output.status.success(),
        "the oracle script must exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).expect("the oracle prints ASCII");
    assert!(
        !stdout.trim().is_empty(),
        "the oracle printed nothing — it is not looking at the fixtures",
    );

    // Parse the Python table into (path, pid, max_step, disc_seen).
    let mut py: BTreeMap<(String, u16), (f64, bool)> = BTreeMap::new();
    for line in stdout.lines() {
        let Some((path, rest)) = line.split_once(" pid=0x") else {
            continue;
        };
        let (pid_hex, rest) = rest
            .split_once(' ')
            .expect("pid field is followed by a space");
        let Some(step) = rest
            .split("max_step_ms=")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse::<f64>().ok())
        else {
            continue;
        };
        let disc = rest.contains("discontinuity_indicator_seen=yes");
        let pid = u16::from_str_radix(pid_hex, 16).expect("pid is hex");
        // The script echoes back whatever path it was given (absolute, here),
        // so key on the file name plus its parent directory, which is unique
        // across this workspace's fixtures.
        let key_path = PathBuf::from(path)
            .parent()
            .and_then(|p| p.file_name())
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        let key_path = format!(
            "{key_path}/{}",
            PathBuf::from(path)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        py.insert((key_path, pid), (step, disc));
    }
    assert!(
        py.len() > 15,
        "the python oracle only reported {} PIDs across every fixture — it is \
         not being driven correctly",
        py.len(),
    );

    // Same table from the in-test oracle, and compare the disagreement set.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut disagreements = Vec::new();
    let mut compared = 0usize;
    for path in &fixtures {
        let Ok(ts) = fs::read(path) else { continue };
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .display()
            .to_string();
        // Match the python oracle's key shape: parent directory + file name.
        let key = format!(
            "{}/{}",
            path.parent()
                .and_then(|p| p.file_name())
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path.file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        for (pid, stats) in pcr_stats(&ts) {
            let Some(&(py_step, py_disc)) = py.get(&(key.clone(), pid)) else {
                disagreements.push(format!("{rel} pid 0x{pid:04X}: missing from python oracle"));
                continue;
            };
            compared += 1;
            // The python oracle reports the largest *absolute* step as a
            // signed value; the in-test one splits forward and backward.
            let rust_magnitude = stats.max_forward_ms.max(stats.max_backward_ms);
            if (rust_magnitude - py_step).abs() > 0.5 {
                disagreements.push(format!(
                    "{rel} pid 0x{pid:04X}: rust {rust_magnitude:.3} ms vs python {py_step:.3} ms",
                ));
            }
            if py_disc != stats.discontinuity_indicator_seen {
                disagreements.push(format!(
                    "{rel} pid 0x{pid:04X}: discontinuity seen rust={} python={py_disc}",
                    stats.discontinuity_indicator_seen,
                ));
            }
        }
    }
    assert!(
        compared > 15,
        "only {compared} (fixture, PID) pairs were compared — the two oracles \
         are not looking at the same data",
    );
    assert!(
        disagreements.is_empty(),
        "the two independent PCR oracles disagree on {} point(s):\n{}",
        disagreements.len(),
        disagreements.join("\n"),
    );
}
