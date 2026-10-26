#!/usr/bin/env python3
"""Apply each mutant of docs/mutation-table-client.md to the DashClient source,
run the client test files, and report whether any test failed. Every edit is
reverted in a finally block.

    python3 dash-runtime/docs/mutants_client.py --dry     # every pattern matches exactly once
    python3 dash-runtime/docs/mutants_client.py           # run all
    python3 dash-runtime/docs/mutants_client.py C05 C19   # run some
    python3 dash-runtime/docs/mutants_client.py --json out.json

Commit first: `git diff` is the check that nothing was left mutated.
"""
import json
import os
import pathlib
import re
import subprocess
import sys

ROOT = str(pathlib.Path(__file__).resolve().parents[2])
E = "dash-runtime/src/client/engine.rs"
CFG = "dash-runtime/src/client/config.rs"
TESTS = ["client_static", "client_periods", "client_faults", "client_misc"]
M = [
 ("C01", "tail threshold `<` -> `>=`: a short last segment is not a tail", E, ".is_some_and(|s| s.duration_us < MISSING_TAIL_MAX_US)", ".is_some_and(|s| s.duration_us >= MISSING_TAIL_MAX_US)"),
 ("C02", "any segment, not only the last, may be a tail", E, "last == Some(slot.plan_index)", "true"),
 ("C03", "the missing tail is never recognised", E, "if matches!(status, Some(STATUS_NOT_FOUND | STATUS_GONE)) && self.is_missing_tail(id)", "if false && self.is_missing_tail(id)"),
 ("C04", "404/410 classified Server, not Missing", E, "Failure::Status(STATUS_NOT_FOUND | STATUS_GONE) => FailureKind::Missing,", "Failure::Status(STATUS_NOT_FOUND | STATUS_GONE) => FailureKind::Server,"),
 ("C05", "425 classified Server, not TooEarly", E, "Failure::Status(STATUS_TOO_EARLY) => FailureKind::TooEarly,", "Failure::Status(STATUS_TOO_EARLY) => FailureKind::Server,"),
 ("C06", "give-up `>=` -> `>` (cap off by one)", E, "if count >= self.max_failures(id) {", "if count > self.max_failures(id) {"),
 ("C07", "failure counter never grows", E, "*n = n.saturating_add(1);", "*n = 1;"),
 ("C08", "init/index use the segment cap", E, "ResourceId::Init { .. } | ResourceId::Index { .. } => self.cfg.max_init_failures,", "ResourceId::Init { .. } | ResourceId::Index { .. } => self.cfg.max_segment_failures,"),
 ("C09", "window `<` -> `<=` (one request too many)", E, "self.outstanding < self.cfg.window()", "self.outstanding <= self.cfg.window()"),
 ("C10", "window of 0 not treated as 1", CFG, "self.max_inflight_segments.max(1)", "self.max_inflight_segments"),
 ("C11", "alternate rotation after 3 failures, not 2", E, "if rep.cand_failures >= ROTATE_AFTER_FAILURES && rep.candidates.len() > 1 {", "if rep.cand_failures > ROTATE_AFTER_FAILURES && rep.candidates.len() > 1 {"),
 ("C12", "alternates never rotated", E, "rep.candidates.len() > 1 {", "rep.candidates.len() > 100 {"),
 ("C13", "consecutive-failure run not reset after a rotation", E, "rep.cand_failures = 0;", ""),
 ("C14", "continuity signal ignored", E, "&& continuity_signals(set, prev_id)", "&& true"),
 ("C15", "continuity ignores a changed init", E, "&& &rep.init_key == key", "&& true"),
 ("C16", "continuity ignores the AdaptationSet id", E, "&& lane.set_id == *set_id", "&& true"),
 ("C17", "no Discontinuity at a Period boundary", E, "if !first && !continuous {", "if false {"),
 ("C18", "Discontinuity before the very first Period", E, "if !first && !continuous {", "if !continuous {"),
 ("C19", "Init re-announced across a continuous boundary", E, "if !continuous && let Some(bytes)", "if let Some(bytes)"),
 ("C20", "init identity keeps the URL query", E, "state.init_key.url = without_query(&init_url);", "state.init_key.url = init_url;"),
 ("C21", "origin pin never refuses", E, "if !same_origin(doc, &parsed) {", "if false {"),
 ("C22", "origin comparison ignores the port", E, "\n        && a.port_or_known_default() == b.port_or_known_default()", ""),
 ("C23", "origin comparison ignores the scheme", E, "a.scheme() == b.scheme()\n        &&", "true\n        &&"),
 ("C24", "MPD URL query appended to init and index too", E, "if media && let Some(q) = &rep.query {", "if let Some(q) = &rep.query {"),
 ("C25", "a switch is always treated as aligned", E, "                set.segment_alignment\n                    && matches!(", "                true\n                    && matches!("),
 ("C26", "switch continues one segment late", E, "plan.index_at_or_before(switch_us).unwrap_or(0)", "plan.index_at_or_before(switch_us).map_or(0, |i| i + 1)"),
 ("C27", "window slot never released on delivery", E, "self.outstanding = self.outstanding.saturating_sub(1);", ""),
 ("C28", "skipped segment leaves no Discontinuity", E, "slot.state = SlotState::Skipped { marker: true };", "slot.state = SlotState::Skipped { marker: false };"),
 ("C29", "retry queued behind later requests", E, "self.queue.push_front(id);", "self.queue.push_back(id);"),
 ("C30", "newest segment requested first across lanes", E, "if pick.is_none_or(|(_, best)| seg.start_us < best) {", "if pick.is_none_or(|(_, best)| seg.start_us > best) {"),
 ("C31", "Period starts before its inits arrived", E, ".is_some_and(|e| e.state == InitState::Ready)", ".is_some_and(|_| true)"),
 ("C32", "second lane reuses the first lane's track ids", E, "base = base.saturating_add(n);", "base = base.saturating_add(0);"),
 ("C33", "segments held for an init are never demuxed", E, "let held = self.demux_held(&key);", "let held: Result<()> = Ok(());"),
 ("C34", "a second delivery is not detected", E, "if self.done.contains(&id) {", "if false {"),
 ("C35", "an unrequested delivery is accepted", E, "        if !self.inflight.contains(&id) {\n            return Err(Error::UnrequestedResource { id });\n        }\n        Ok(())", "        Ok(())"),
 ("C36", "PeriodChanged carries start 0", E, "start_us: p.start_us,", "start_us: 0,"),
 ("C37", "progressive MP4 segment not fatal", E, "if segment::progressive_movie(bytes) {", "if false {"),
 ("C38", "init without a complete moov accepted", E, "if !segment::progressive_movie(bytes) {", "if false {"),
 ("C39", "structural media check bypassed", E, "if !segment::media_fragment(bytes) {", "if false {"),
 ("C40", "size cap `>` -> `>=`", E, "if bytes.len() > self.cfg.max_segment_bytes {", "if bytes.len() >= self.cfg.max_segment_bytes {"),
 ("C41", "ranged response length not checked", E, "(u64::try_from(bytes.len()).ok() != Some(range.1)).then_some(Rejection::WrongLength)", "None::<Rejection>"),
 ("C42", "dynamic MPD accepted", E, "if mpd.kind == PresentationType::Dynamic {", "if false {"),
 ("C43", "track set re-announced every Period", E, "if self.announced_tracks.as_deref() != Some(fingerprint.as_str()) {", "if true {"),
 ("C44", "no encryption notice", E, "if encryption {", "if false {"),
 ("C45", "failed resource cannot be retried", E, "self.parked.insert(id);\n        FailureClass::Retry {", "FailureClass::Retry {"),
 ("C46", "byte ranges dropped from init/index requests", E, "range: range.and_then(|r| r.offset_len().ok()),", "range: None,"),
 ("C47", "lane exhausted one segment late", E, ".is_some_and(|p| rep.next_issue >= p.len().unwrap_or(0)),", ".is_some_and(|p| rep.next_issue > p.len().unwrap_or(0)),"),
 ("C48", "dropped sets are not reported", E, "for d in &selection.dropped {", "for d in selection.dropped.iter().take(0) {"),
]


def run(cmd):
    return subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, timeout=600)


def main():
    args = sys.argv[1:]
    dry = "--dry" in args
    out_json = None
    if "--json" in args:
        out_json = args[args.index("--json") + 1]
        args = [a for a in args if a != "--json" and a != out_json]
    only = [a for a in args if re.match(r"C\d+$", a)]
    results = []
    for mid, desc, f, old, new in M:
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
        open(path, "w").write(src.replace(old, new))
        try:
            cmd = ["cargo", "test", "-p", "dash-runtime", "--offline", "--no-fail-fast"]
            for t in TESTS:
                cmd += ["--test", t]
            r = run(cmd)
            out = r.stdout + r.stderr
            compiled = "Compiling dash-runtime" in out
            killers = sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED", out, re.M)))
            if "could not compile" in out and not killers:
                status = "COMPILE-ERROR"
            else:
                status = "KILLED" if killers else "SURVIVED"
            results.append((mid, desc, status, killers, compiled))
            print(f"{mid}: {status} recompiled={compiled} {killers[:3]}", flush=True)
        except subprocess.TimeoutExpired:
            results.append((mid, desc, "KILLED-BY-TIMEOUT", [], True))
            print(f"{mid}: KILLED-BY-TIMEOUT", flush=True)
        finally:
            open(path, "w").write(src)
    if out_json:
        json.dump(results, open(out_json, "w"))


main()
