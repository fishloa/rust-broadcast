#!/usr/bin/env python3
"""Apply each mutant of docs/mutation-table.md (design step W-b) to the production source, run the
tests that cover it, and report whether any test failed. Every edit is reverted.

    python3 broadcast-client/docs/mutants.py --dry     # check every pattern matches once
    python3 broadcast-client/docs/mutants.py           # run all
    python3 broadcast-client/docs/mutants.py W05 W19   # run some

Commit first: a crash mid-run could leave a mutated file behind (it is restored in a finally block,
but `git diff` is the check). A mutant is KILLED when at least one test fails. "recompiled" shows the
edited crate was rebuilt for that run.
"""
import json
import os
import pathlib
import re
import subprocess
import sys

ROOT = str(pathlib.Path(__file__).resolve().parents[2])
BC = "broadcast-client/src/"
DR = "dash-runtime/src/"

BC_TESTS = ["cargo", "test", "-p", "broadcast-client", "--all-features", "--locked", "--no-fail-fast",
            "--lib", "--test", "dash_clock_surface"]
DR_TESTS = ["cargo", "test", "-p", "dash-runtime", "--locked", "--no-fail-fast",
            "--test", "client_config", "--test", "datetime", "--test", "label_coverage"]

M = [
 ("W01", "Retry-After status gate: && -> || (always 0)", BC+"source/fetch/retry.rs",
  "if status != STATUS_TOO_MANY_REQUESTS && status != STATUS_SERVICE_UNAVAILABLE {",
  "if status != STATUS_TOO_MANY_REQUESTS || status != STATUS_SERVICE_UNAVAILABLE {"),
 ("W02", "503 constant -> 502", BC+"source/fetch/retry.rs",
  "const STATUS_SERVICE_UNAVAILABLE: u16 = 503;", "const STATUS_SERVICE_UNAVAILABLE: u16 = 502;"),
 ("W03", "60 s clamp removed", BC+"source/fetch/retry.rs",
  "wait_us.min(i128::from(MAX_RETRY_AFTER_SECS * MICROS_PER_SECOND))", "wait_us.min(i128::MAX)"),
 ("W04", "60 s clamp -> 61 s", BC+"source/fetch/retry.rs",
  "const MAX_RETRY_AFTER_SECS: u64 = 60;", "const MAX_RETRY_AFTER_SECS: u64 = 61;"),
 ("W05", "delta beyond u64 ignored instead of saturated", BC+"source/fetch/retry.rs",
  "value.parse::<u64>().unwrap_or(u64::MAX)", "value.parse::<u64>().unwrap_or(0)"),
 ("W06", "delta-seconds test: all digits -> any digit", BC+"source/fetch/retry.rs",
  "value.bytes().all(|b| b.is_ascii_digit())", "value.bytes().any(|b| b.is_ascii_digit())"),
 ("W07", "empty value counts as delta-seconds", BC+"source/fetch/retry.rs",
  "if !value.is_empty() && value.bytes()", "if value.bytes()"),
 ("W08", "HTTP-date wait sign flipped (now - date)", BC+"source/fetch/retry.rs",
  "i128::from(until) - now_unix", "now_unix - i128::from(until)"),
 ("W09", "anchor ignored, Date header always used", BC+"source/fetch/retry.rs",
  "Some(wall) => Some(wall.unix_at(now_us)),", "Some(_) => header_value(headers, HEADER_DATE).and_then(|date| parse_http_date(date).ok()).map(i128::from),"),
 ("W10", "no-anchor fallback to the Date header removed", BC+"source/fetch/retry.rs",
  "None => header_value(headers, HEADER_DATE)\n                .and_then(|date| parse_http_date(date).ok())\n                .map(i128::from),",
  "None => None,"),
 ("W11", "Date header name misspelt", BC+"source/fetch/retry.rs",
  'const HEADER_DATE: &str = "date";', 'const HEADER_DATE: &str = "dat";'),
 ("W12", "Retry-After header name misspelt", BC+"source/fetch/retry.rs",
  'const HEADER_RETRY_AFTER: &str = "retry-after";', 'const HEADER_RETRY_AFTER: &str = "retry_after";'),
 ("W13", "header lookup case-sensitive", BC+"source/fetch/retry.rs",
  ".find(|(k, _)| k.eq_ignore_ascii_case(name))", ".find(|(k, _)| k == name)"),
 ("W14", "Retry-After value not trimmed", BC+"source/fetch/retry.rs",
  ".map(str::trim) else", ".map(|v| v) else"),
 ("W16", "session never stores the anchor", BC+"session.rs",
  "self.wall = Some(WallClock::new(now_us, unix_us));", "let _ = (now_us, unix_us);"),
 ("W17", "session stores the anchor arguments swapped", BC+"session.rs",
  "self.wall = Some(WallClock::new(now_us, unix_us));", "self.wall = Some(WallClock::new(unix_us, now_us));"),
 ("W18", "session does not pass the anchor to HLS", BC+"session.rs",
  "                self.wall.as_ref(),\n", "                None,\n"),
 ("W19", "Config::validate skips the dash check", BC+"events.rs",
  "if let Err(e) = self.dash.validate() {", "if let Err(e) = Ok::<(), dash_runtime::client::config::DashConfigError>(()) {"),
 ("W20", "dash field names lose their prefix", BC+"events.rs",
  '"utc_timeout_us" => "dash.utc_timeout_us",', '"utc_timeout_us" => "utc_timeout_us",'),
 ("W21", "ClockSynced equality: && -> ||", BC+"events.rs",
  "=> *sa == *sb && *ua == *ub,", "=> *sa == *sb || *ua == *ub,"),
 ("W22", "ErrorKind label mpd_parse misspelt", BC+"events.rs",
  'Self::MpdParse { .. } => "mpd_parse",', 'Self::MpdParse { .. } => "mpd-parse",'),
 ("W23", "ErrorKind label live_edge_lost -> clock_unavailable", BC+"events.rs",
  'Self::LiveEdgeLost => "live_edge_lost",', 'Self::LiveEdgeLost => "clock_unavailable",'),
 ("W24", "Default dash config replaced by a zeroed timeout", BC+"events.rs",
  "dash: DashConfig::default(),", "dash: { let mut d = DashConfig::default(); d.early_retry_us = 0; d },"),
 ("W25", "wait-shorter-than-timeout check: < -> <=", DR+"client/config.rs",
  "if self.utc_wait_us < self.utc_timeout_us {", "if self.utc_wait_us <= self.utc_timeout_us {"),
 ("W26", "wait-shorter-than-timeout check removed", DR+"client/config.rs",
  "if self.utc_wait_us < self.utc_timeout_us {", "if false {"),
 ("W27", "zero utc_timeout_us accepted", DR+"client/config.rs",
  "if self.utc_timeout_us == 0 {", "if false {"),
 ("W28", "zero utc_max_rtt_us accepted", DR+"client/config.rs",
  "if self.utc_max_rtt_us == 0 {", "if false {"),
 ("W29", "zero utc_resync_us accepted", DR+"client/config.rs",
  "if self.utc_resync_us == 0 {", "if false {"),
 ("W30", "zero early_retry_us accepted", DR+"client/config.rs",
  "if self.early_retry_us == 0 {", "if false {"),
 ("W31", "zero min_mpd_refresh_us accepted", DR+"client/config.rs",
  "if self.min_mpd_refresh_us == 0 {", "if false {"),
 ("W32", "default utc_wait 5 s -> 3 s", DR+"client/config.rs",
  "pub const DEFAULT_UTC_WAIT_US: u64 = 5 * MICROS_PER_SECOND;", "pub const DEFAULT_UTC_WAIT_US: u64 = 3 * MICROS_PER_SECOND;"),
 ("W33", "default utc_resync 600 s -> 60 s", DR+"client/config.rs",
  "pub const DEFAULT_UTC_RESYNC_US: u64 = 600 * MICROS_PER_SECOND;", "pub const DEFAULT_UTC_RESYNC_US: u64 = 60 * MICROS_PER_SECOND;"),
 ("W34", "default cross_origin_urls true -> false", DR+"client/config.rs",
  "cross_origin_urls: true,", "cross_origin_urls: false,"),
 ("W35", "default early_grace 1 s -> 0", DR+"client/config.rs",
  "pub const DEFAULT_EARLY_GRACE_US: u64 = MICROS_PER_SECOND;", "pub const DEFAULT_EARLY_GRACE_US: u64 = 0;"),
 ("W36", "clock policy label prefer_host misspelt", DR+"client/config.rs",
  'Self::PreferHost => "prefer_host",', 'Self::PreferHost => "prefer-host",'),
 ("W37", "RFC 850 two-digit year read as four-digit", DR+"timing/datetime.rs",
  'const HTTP_DATE_RFC850: &str = "%A, %d-%b-%y %H:%M:%S GMT";', 'const HTTP_DATE_RFC850: &str = "%A, %d-%b-%Y %H:%M:%S GMT";'),
 ("W38", "asctime day read zero-padded", DR+"timing/datetime.rs",
  'const HTTP_DATE_ASCTIME: &str = "%a %b %e %H:%M:%S %Y";', 'const HTTP_DATE_ASCTIME: &str = "%a %b %d %H:%M:%S %Y";'),
 ("W39", "obsolete HTTP-date forms not tried", DR+"timing/datetime.rs",
  "[HTTP_DATE_RFC850, HTTP_DATE_ASCTIME]", "[HTTP_DATE_RFC850; 0]"),
 ("W40", "HTTP-date input not trimmed", DR+"timing/datetime.rs",
  "    let text = text.trim();\n    if let Ok(t) = jiff::fmt::rfc2822", "    if let Ok(t) = jiff::fmt::rfc2822"),
]
M += [
 ("W15", "anchor mapping: mono instant added instead of subtracted", BC+"source/fetch/wall.rs",
  "i128::from(self.unix_us) + i128::from(now_us) - i128::from(self.mono_us)",
  "i128::from(self.unix_us) + i128::from(now_us) + i128::from(self.mono_us)"),
 ("W41", "anchor mapping: now and mono swapped", BC+"source/fetch/wall.rs",
  "i128::from(self.unix_us) + i128::from(now_us) - i128::from(self.mono_us)",
  "i128::from(self.unix_us) - i128::from(now_us) + i128::from(self.mono_us)"),
]


def run(cmd):
    return subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)


def main():
    dry = "--dry" in sys.argv
    only = [a for a in sys.argv[1:] if a.startswith("W")]
    results = []
    for mid, desc, f, old, new in sorted(M):
        if only and mid not in only:
            continue
        path = os.path.join(ROOT, f)
        src = open(path).read()
        n = src.count(old)
        if n != 1:
            print(f"{mid}: PATTERN COUNT {n} in {f}")
            results.append((mid, desc, "BAD-PATTERN", [], False))
            continue
        if dry:
            print(mid, "ok")
            continue
        crate = "dash-runtime" if f.startswith("dash-runtime") else "broadcast-client"
        open(path, "w").write(src.replace(old, new))
        try:
            outs = []
            for cmd in ([DR_TESTS, BC_TESTS] if crate == "dash-runtime" else [BC_TESTS]):
                r = run(cmd)
                out = r.stdout + r.stderr
                outs.append(out)
                if re.search(r"^test (\S+) \.\.\. FAILED", out, re.M) or "could not compile" in out:
                    break
            out = "\n".join(outs)
            compiled = f"Compiling {crate}" in out
            if "could not compile" in out and "FAILED" not in out:
                status, killers = "COMPILE-ERROR", []
            else:
                killers = sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED", out, re.M)))
                status = "KILLED" if killers else "SURVIVED"
            results.append((mid, desc, status, killers, compiled))
            print(f"{mid}: {status} recompiled={compiled} {killers[:4]}", flush=True)
        finally:
            open(path, "w").write(src)
    print(json.dumps(results))


main()
