#!/usr/bin/env python3
"""Apply each mutant of docs/mutation-table-dash-pull.md and run the dash_pull tests.

    python3 multimux/docs/mutants_dash_pull.py --dry   # every pattern matches exactly once
    python3 multimux/docs/mutants_dash_pull.py S01 D04

Commit first: each edit is reverted with `git checkout -- <file>`, which would also drop
uncommitted work in that file. A mutant that does not compile is reported `invalid`, not
`killed`, so a typo cannot pass for a kill."""
import pathlib
import subprocess
import sys

ROOT = str(pathlib.Path(__file__).resolve().parents[2])
SRC = "multimux/src/source/dash_pull/"

MULTIMUX = [
    "cargo", "nextest", "run", "-p", "multimux", "--all-features", "--locked", "--no-fail-fast",
    "-E", "test(/dash_pull/) | binary(dash_pull_route) | binary(time_codec) | binary(no_handroll_guard)",
]
TRANSMUX = [
    "cargo", "nextest", "run", "-p", "transmux", "--all-features", "--locked", "--no-fail-fast",
    "--test", "fmp4_trex_defaults",
]

# (id, file, old, new, command)
M = [
    # --- session.rs: what ends an attempt, what is announced
    ("S01", SRC + "session.rs", "const MAX_MPD_FAILURES: u32 = 8;", "const MAX_MPD_FAILURES: u32 = 1_000;", MULTIMUX),
    ("S02", SRC + "session.rs", "if !self.mpd_accepted || self.mpd_rejections >= MAX_MPD_FAILURES {", "if self.mpd_rejections >= MAX_MPD_FAILURES {", MULTIMUX),
    ("S03", SRC + "session.rs", "if !self.mpd_accepted || self.mpd_rejections >= MAX_MPD_FAILURES {", "if !self.mpd_accepted || self.mpd_rejections > MAX_MPD_FAILURES {", MULTIMUX),
    ("S04", SRC + "session.rs", "                self.mpd_rejections = 0;\n                self.mpd_accepted = true;", "                self.mpd_accepted = true;", MULTIMUX),
    ("S05", SRC + "session.rs", "if attempt >= MAX_MPD_FAILURES {", "if attempt > MAX_MPD_FAILURES {", MULTIMUX),
    ("S06", SRC + "session.rs", "                    self.established = true;\n", "", MULTIMUX),
    ("S07", SRC + "session.rs", "Output::Discontinuity => self.stitch.discontinuity(),", "Output::Discontinuity => {}", MULTIMUX),
    ("S08", SRC + "session.rs", "let event = if self.announced {", "let event = if !self.announced {", MULTIMUX),
    ("S09", SRC + "session.rs", "self.now_us = self.now_us.max(now_us);", "self.now_us = now_us;", MULTIMUX),
    ("S10", SRC + "session.rs", "FatalKind::MpdInvalid { reason } if self.encryption_seen => MultimuxError::Encrypted {", "FatalKind::MpdInvalid { reason } if false => MultimuxError::Encrypted {", MULTIMUX),
    ("S11", SRC + "session.rs", "                    self.encryption_seen = true;", "                    self.encryption_seen = false;", MULTIMUX),
    ("S12", SRC + "session.rs", "Output::EndOfStream => self.ended = true,", "Output::EndOfStream => {}", MULTIMUX),
    ("S13", SRC + "session.rs", "        self.client.set_wall_clock(now_us, unix_us);", "        let _ = unix_us;", MULTIMUX),
    ("S14", SRC + "session.rs", "config.cross_origin_urls = allow_cross_origin;", "config.cross_origin_urls = true;", MULTIMUX),
    ("S15", SRC + "session.rs", "limits.max_segment_failures = MAX_SEGMENT_FAILURES;", "limits.max_segment_failures = u32::MAX;", MULTIMUX),
    ("S16", SRC + "session.rs", "            attempt: self.mpd_rejections,", "            attempt: 0,", MULTIMUX),
    ("S17", SRC + "session.rs", "self.retries.push_back(Retry { target, attempt });", "self.retries.push_back(Retry { target, attempt: attempt + 1 });", MULTIMUX),
    ("S18", SRC + "session.rs", "                if specs.is_empty() {", "                if false {", MULTIMUX),
    ("S19", SRC + "session.rs", "                self.stitch.set_tracks(&specs);", "", MULTIMUX),
    ("S20", SRC + "session.rs", "                        retention: RetentionClass::Timed,", "                        retention: RetentionClass::Sparse,", MULTIMUX),
    # --- drive.rs: pacing, wire rules, the loop's terminal conditions
    ("D01", SRC + "drive.rs", "const RETRY_BASE: Duration = Duration::from_millis(500);", "const RETRY_BASE: Duration = Duration::from_millis(5);", MULTIMUX),
    ("D02", SRC + "drive.rs", "const RETRY_CAP: Duration = Duration::from_secs(4);", "const RETRY_CAP: Duration = Duration::from_secs(40);", MULTIMUX),
    ("D03", SRC + "drive.rs", "let doublings = attempt.saturating_sub(1).min(8);", "let doublings = attempt.min(8);", MULTIMUX),
    ("D04", SRC + "drive.rs", "                with_credentials: false,", "                with_credentials: true,", MULTIMUX),
    ("D05", SRC + "drive.rs", "UtcMethod::Head => Method::HEAD,", "UtcMethod::Head => Method::GET,", MULTIMUX),
    ("D06", SRC + "drive.rs", "conditional.push((IF_NONE_MATCH, etag));", "let _ = etag;", MULTIMUX),
    ("D07", SRC + "drive.rs", "conditional.push((IF_MODIFIED_SINCE, modified));", "let _ = modified;", MULTIMUX),
    ("D08", SRC + "drive.rs", "Range::bytes(offset..offset.saturating_add(length))", "Range::bytes(offset..offset.saturating_add(length + 1))", MULTIMUX),
    ("D09", SRC + "drive.rs", "(honoured && body.len() == length).then_some(body)", "(honoured && body.len() >= length).then_some(body)", MULTIMUX),
    ("D10", SRC + "drive.rs", "let honoured = content_range.is_none_or(|range| range == (offset, last));", "let honoured = content_range.is_none_or(|range| range.0 == offset);", MULTIMUX),
    ("D11", SRC + "drive.rs", "body.get(start..end).map(<[u8]>::to_vec)", "body.get(start..).map(<[u8]>::to_vec)", MULTIMUX),
    ("D12", SRC + "drive.rs", "if status == StatusCode::UNAUTHORIZED {", "if false && status == StatusCode::UNAUTHORIZED {", MULTIMUX),
    ("D13", SRC + "drive.rs", "if status == StatusCode::NOT_MODIFIED && req.target == FetchTarget::Mpd {", "if false && status == StatusCode::NOT_MODIFIED && req.target == FetchTarget::Mpd {", MULTIMUX),
    ("D14", SRC + "drive.rs", "return failed(fetch, Failure::Status(status.as_u16()), error);", "return failed(fetch, Failure::Transport, error);", MULTIMUX),
    ("D15", SRC + "drive.rs", "fetch.final_url = (response.url().as_str() != req.url).then(|| response.url().to_string());", "fetch.final_url = None;", MULTIMUX),
    ("D16", SRC + "drive.rs", "return failed(fetch, Failure::TooLarge, error);", "return failed(fetch, Failure::Transport, error);", MULTIMUX),
    ("D17", SRC + "drive.rs", "                reason: format!(\"dash-pull {} read: {e}\", req.what),\n            };\n            return failed(fetch, Failure::Transport, error);", "                reason: format!(\"dash-pull {} read: {e}\", req.what),\n            };\n            return failed(fetch, Failure::TooLarge, error);", MULTIMUX),
    ("D18", SRC + "drive.rs", "failed(DashFetch::new(req.target, sent_us), Failure::Timeout, error)", "failed(DashFetch::new(req.target, sent_us), Failure::Transport, error)", MULTIMUX),
    ("D19", SRC + "drive.rs", "let mut next_anchor_us = 0u64;", "let mut next_anchor_us = u64::MAX;", MULTIMUX),
    ("D20", SRC + "drive.rs", "if driver.session().ended() && scheduler.is_idle() {", "if false && driver.session().ended() && scheduler.is_idle() {", MULTIMUX),
    ("D21", SRC + "drive.rs", "if !driver.health().is_running() {", "if false && !driver.health().is_running() {", MULTIMUX),
    ("D22", SRC + "drive.rs", "if fetch.target == FetchTarget::Mpd && !driver.session().mpd_accepted() {", "if false && fetch.target == FetchTarget::Mpd && !driver.session().mpd_accepted() {", MULTIMUX),
    ("D23", SRC + "drive.rs", "            retries.insert(retry.target, at);", "            let _ = at;", MULTIMUX),
    ("D24", SRC + "drive.rs", "Outcome::Unauthorized => return Err(error),", "Outcome::Unauthorized => {}", MULTIMUX),
    ("D25", SRC + "drive.rs", "            driver.session_mut().retry(target);", "            let _ = target;", MULTIMUX),
    ("D26", SRC + "drive.rs", "fetch.etag = header_text(headers, &ETAG);", "fetch.etag = None;", MULTIMUX),
    ("D27", SRC + "drive.rs", "fetch.received_us = net.clock.mono_us();", "fetch.received_us = 0;", MULTIMUX),
    ("D28", SRC + "drive.rs", "fetch.date = header_text(headers, &DATE);", "fetch.date = None;", MULTIMUX),
    ("D29", SRC + "drive.rs", "super::session::dash_config(route.allow_cross_origin)", "super::session::dash_config(false)", MULTIMUX),
    # --- the shared authenticated GET
    ("H01", "multimux/src/source/http_auth.rs", "        for (name, value) in extra_headers {\n            req = req.header(name, value);\n        }\n        req\n", "        req\n", MULTIMUX),
    # --- stitch.rs: one timeline
    ("T01", SRC + "stitch.rs", "let stitch = self.tracks.values().filter_map(|t| t.end_us).max();", "let stitch = self.tracks.values().filter_map(|t| t.end_us).min();", MULTIMUX),
    ("T02", SRC + "stitch.rs", "self.offset_us = stitch - ticks_to_us(first, timescale);", "self.offset_us = ticks_to_us(first, timescale) - stitch;", MULTIMUX),
    ("T03", SRC + "stitch.rs", "                    self.generation += 1;\n", "", MULTIMUX),
    ("T04", SRC + "stitch.rs", "(us * i128::from(timescale) + HALF_SECOND_US)", "(us * i128::from(timescale))", MULTIMUX),
    ("T05", SRC + "stitch.rs", "sample.pts = sample.pts.map(|t| t.saturating_add(shift));", "let _ = shift;", MULTIMUX),
    ("T06", SRC + "stitch.rs", "            if timescale == 0 {", "            if false {", MULTIMUX),
    ("T07", SRC + "stitch.rs", "let end = dts.saturating_add(i64::from(sample.duration.unwrap_or(0)));", "let end = dts;", MULTIMUX),
    # --- the route's opt-in and the config surface
    ("W01", SRC + "mod.rs", "            allow_cross_origin: false,\n        }\n    }\n\n    /// Overrides", "            allow_cross_origin: true,\n        }\n    }\n\n    /// Overrides", MULTIMUX),
    ("W02", SRC + "mod.rs", "            self.dash,\n", "            dash_config(false),\n", MULTIMUX),
    ("W03", "multimux/src/origin/mod.rs", ".with_cross_origin_urls(*allow_cross_origin);", ".with_cross_origin_urls(false);", MULTIMUX),
    ("W04", "multimux/src/config.rs", "        #[serde(default)]\n        allow_cross_origin: bool,", "        allow_cross_origin: bool,", MULTIMUX),
    ("W05", SRC + "mod.rs", "    if !s.ends_with('Z') {", "    if false {", MULTIMUX),
    # --- transmux: trex defaults (fmp4_trex_defaults)
    ("X01", "transmux/src/media.rs", "        apply_trex_defaults(&mut moof, trex_defaults);\n", "", TRANSMUX),
    ("X02", "transmux/src/media.rs", "        tfhd.default_sample_duration\n            .get_or_insert(defaults.default_sample_duration);\n", "", TRANSMUX),
    ("X03", "transmux/src/media.rs", "        tfhd.default_sample_flags\n            .get_or_insert(defaults.default_sample_flags);\n", "", TRANSMUX),
]


def main():
    args = sys.argv[1:]
    dry = "--dry" in args
    only = [a for a in args if a != "--dry"]
    for mid, f, old, new, cmd in M:
        if only and mid not in only:
            continue
        path = pathlib.Path(ROOT) / f
        src = path.read_text()
        count = src.count(old)
        if count != 1:
            print(mid, "pattern matches", count, flush=True)
            continue
        if dry:
            print(mid, "ok", flush=True)
            continue
        path.write_text(src.replace(old, new, 1))
        try:
            p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
        finally:
            subprocess.run(["git", "checkout", "--", f], cwd=ROOT, check=True)
        out = p.stdout + p.stderr
        if "could not compile" in out or "error[E" in out:
            status = "invalid"
        elif p.returncode:
            names = sorted({l.split()[-1] for l in out.splitlines() if l.strip().startswith("FAIL") and "(" in l})
            status = "killed " + ", ".join(n.split("::")[-1] for n in names)
        else:
            status = "SURVIVED"
        print(mid, status, flush=True)


main()
