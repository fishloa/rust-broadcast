#!/usr/bin/env python3
"""Apply each mutant of docs/mutation-table-dash.md (design step W-e) to the production source, run the
DASH test binaries that cover it, and report whether any test failed. Every edit is reverted.

    python3 broadcast-client/docs/mutants_dash.py --dry     # check every pattern matches exactly once
    python3 broadcast-client/docs/mutants_dash.py           # run all
    python3 broadcast-client/docs/mutants_dash.py D03 D07   # run some

Commit first: a crash mid-run could leave a mutated file behind (it is restored in a finally block, but
`git diff` is the check). A mutant is KILLED when at least one test fails. "recompiled" shows the edited
crate was rebuilt for that run.
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

DASH_TESTS = ["cargo", "test", "-p", "broadcast-client", "--all-features", "--locked", "--no-fail-fast",
              "--test", "dash_static", "--test", "dash_live", "--test", "dash_select", "--test", "dash_errors",
              "--test", "dash_http", "--test", "dash_muxed", "--test", "dash_source_kind", "--test", "dash_matrix",
              "--test", "dash_clock_surface", "--test", "label_coverage"]
CORE_TESTS = ["cargo", "test", "-p", "dash-runtime", "--locked", "--no-fail-fast",
              "--test", "client_select_audio", "--test", "client_static"]

D = BC + "source/dash.rs"
S = BC + "session.rs"

M = [
 ("D01", "If-None-Match header name misspelt", D,
  'const HEADER_IF_NONE_MATCH: &str = "If-None-Match";', 'const HEADER_IF_NONE_MATCH: &str = "If-None-Matc";'),
 ("D02", "Last-Modified validator never sent", D,
  "conditional.push((HEADER_IF_MODIFIED_SINCE.to_string(), modified));", "let _ = modified;"),
 ("D03", "a 304 answer is no longer taken as 'unchanged'", D,
  "} else if status == STATUS_NOT_MODIFIED && req.purpose == Purpose::Mpd {",
  "} else if status == STATUS_NOT_MODIFIED && req.purpose == Purpose::Mpd && false {"),
 ("D04", "UTCTiming HEAD sent as GET", D,
  "UtcMethod::Head => METHOD_HEAD,", "UtcMethod::Head => METHOD_GET,"),
 ("D05", "HEAD Content-Length counted as a truncated body", D,
  "let truncated = !head_request && req.body.is_truncated();", "let truncated = req.body.is_truncated();"),
 ("D06", "range check skipped (any 2xx accepted for a ranged request)", D,
  "if !req.body.accept(status, headers, req.byte_range) {", "if false {"),
 ("D07", "redirect count never grows", D,
  "req.redirects = req.redirects.saturating_add(1);", "req.redirects = req.redirects;"),
 ("D08", "403 not reported as Forbidden", D,
  "if status == STATUS_FORBIDDEN {", "if false {"),
 ("D09", "410 no longer 'not found' for the first MPD", D,
  "&& (status == STATUS_NOT_FOUND || status == STATUS_GONE)", "&& (status == STATUS_NOT_FOUND)"),
 ("D10", "NotFound also after the MPD was accepted", D,
  "                && !self.mpd_accepted\n", ""),
 ("D11", "Retry-After ignored", D,
  "self.retry_after_us = retry_after_us(status, headers, now, wall);", "self.retry_after_us = 0;"),
 ("D12", "Retry-After kept for the next failure", D,
  "            self.fail(req, now, Failure::Status(status));\n            self.retry_after_us = 0;\n",
  "            self.fail(req, now, Failure::Status(status));\n"),
 ("D13", "data before the response head enters the body", D,
  "if !req.responded_ok || bytes.is_empty() {", "if bytes.is_empty() {"),
 ("D14", "body cap not enforced", D,
  "if req.body.push(bytes).is_err() {", "if false {"),
 ("D15", "oversize body reported every time", D,
  "&& self.oversize_reported.insert(field)", "&& true"),
 ("D16", "truncated transfer delivered as a complete body", D,
  "if error.is_some() || !req.responded_ok || truncated {", "if error.is_some() || !req.responded_ok {"),
 ("D17", "range slice failure delivered", D,
  "} else if req.body.finish() {", "} else if { req.body.finish(); true } {"),
 ("D18", "timed-out request keeps its connection", D,
  "                self.actions.push_back(Action::Close { conn });\n                self.fail(req, now, Failure::Timeout);",
  "                self.fail(req, now, Failure::Timeout);"),
 ("D19", "UTCTiming bounded by the manifest timeout", D,
  "Purpose::Utc(_) => self.utc_us,", "Purpose::Utc(_) => self.manifest_us,"),
 ("D20", "segment bounded by the manifest timeout", D,
  "Purpose::Resource(_) => self.segment_us,", "Purpose::Resource(_) => self.manifest_us,"),
 ("D21", "init and index bounded by the segment timeout", D,
  "            Purpose::Mpd\n            | Purpose::Resource(ResourceId::Init { .. } | ResourceId::Index { .. }) => {\n                self.manifest_us\n            }\n",
  "            Purpose::Mpd => self.manifest_us,\n"),
 ("D22", "segment body cap taken from the manifest cap", D,
  "Purpose::Resource(_) => self.body_limits.segment,", "Purpose::Resource(_) => self.body_limits.manifest,"),
 ("D23", "MPD body cap taken from the segment cap", D,
  "Purpose::Mpd => self.body_limits.manifest,", "Purpose::Mpd => self.body_limits.segment,"),
 ("D24", "unsupported MPD kind retried instead of fatal", D,
  "                !self.mpd_accepted,\n", "                false,\n"),
 ("D25", "MPD parse failures never fatal", D,
  "if definitive || verdict(attempt, self.max_key_failures) == Verdict::GiveUp {", "if definitive {"),
 ("D26", "a good MPD keeps the parse failure count", D,
  "            Ok(()) => {\n                self.mpd_failures.reset();\n                self.mpd_accepted = true;",
  "            Ok(()) => {\n                self.mpd_accepted = true;"),
 ("D27", "parse reason not_mpd misspelt", D,
  'const REASON_NOT_MPD: &str = "not_mpd";', 'const REASON_NOT_MPD: &str = "not-mpd";'),
 ("D28", "parse reason xml misspelt", D,
  'const REASON_XML: &str = "xml";', 'const REASON_XML: &str = "XML";'),
 ("D29", "feature name mpd_patch misspelt", D,
  'const FEATURE_MPD_PATCH: &str = "mpd_patch";', 'const FEATURE_MPD_PATCH: &str = "patch";'),
 ("D30", "feature name xlink misspelt", D,
  'const FEATURE_XLINK: &str = "xlink";', 'const FEATURE_XLINK: &str = "x_link";'),
 ("D31", "feature name utc_timing_scheme misspelt", D,
  'const FEATURE_UTC_SCHEME: &str = "utc_timing_scheme";', 'const FEATURE_UTC_SCHEME: &str = "utc_scheme";'),
 ("D32", "feature name utc_cross_origin misspelt", D,
  'const FEATURE_UTC_CROSS_ORIGIN: &str = "utc_cross_origin";', 'const FEATURE_UTC_CROSS_ORIGIN: &str = "cross_origin";'),
 ("D33", "features reported every time", D,
  "if self.features_reported.insert(feature) {", "if true {"),
 ("D34", "FellBehind not mapped to LiveEdgeLost", D,
  "if self.reported.insert(ErrorKind::LiveEdgeLost.name()) {", "if false {"),
 ("D35", "errors never re-armed by a success", D,
  "        self.reported.clear();\n        if !self.live {", "        if !self.live {"),
 ("D36", "Live announced with every sample batch", D,
  "        if !self.live {\n            self.live = true;", "        if true {\n            self.live = true;"),
 ("D37", "Establishing announced with every response", D,
  "            if !self.established {\n                self.established = true;", "            if true {\n                self.established = true;"),
 ("D38", "samples of a stopped audio track still delivered", D,
  "        if self.audio_stopped\n            && self", "        if false\n            && self"),
 ("D39", "stopped audio still listed in the track set", D,
  ".filter(|s| !(self.audio_stopped && s.config.audio_sample_rate().is_some()))", ".filter(|_| true)"),
 ("D40", "manual video choice not re-applied in a later Period", D,
  "            self.pending_variant = Some(index);\n        }\n        let desired", "        }\n        let desired"),
 ("D41", "manual audio choice not re-applied in a later Period", D,
  "                .filter(|index| Some(*index) != selection.chosen_audio)\n                .map(Some),", "                .filter(|_| false)\n                .map(Some),"),
 ("D42", "auto_select_audio=false still plays audio", D,
  "AudioMode::Manual(None) if !self.known.audio.is_empty() => Some(None),", "AudioMode::Manual(None) if false => Some(None),"),
 ("D43", "AudioRendition announced every time", D,
  "if self.announced_audio == self.known.chosen_audio {", "if false {"),
 ("D44", "unknown variant index accepted", D,
  "return Err(Error::NoSuchVariant(index));", "return Ok(());"),
 ("D45", "reconnect does not announce a discontinuity", D,
  "        self.events.push_back(Event::Discontinuity);\n        self.drain_outputs();\n        if !auth.is_waiting() {",
  "        self.drain_outputs();\n        if !auth.is_waiting() {"),
 ("D46", "reconnect drops the host clock", D,
  "            client.set_wall_clock(mono, unix);", "            let _ = (mono, unix);"),
 ("D47", "reconnect restarts an ended presentation", D,
  "    pub(crate) fn reconnect(&mut self, now: u64, auth: &mut AuthState) {\n        if self.fatal || self.ended {",
  "    pub(crate) fn reconnect(&mut self, now: u64, auth: &mut AuthState) {\n        if self.fatal {"),
 ("D48", "the host clock is not remembered", D,
  "        self.anchor = Some((now_us, unix_us));\n", ""),
 ("D49", "a static MPD gated as live", D,
  "if self.client.is_static() == Some(true) {", "if self.client.is_static() == Some(false) {"),
 ("D50", "live lead taken from a fixed one-second segment", D,
  ".map(|s| u32::try_from(s).unwrap_or(u32::MAX)),", ".map(|_| 1),"),
 ("D51", "retry due check off by one microsecond (resource)", D,
  ".filter(|(_, at)| **at <= now)", ".filter(|(_, at)| **at < now)"),
 ("D52", "retry due check off by one microsecond (MPD)", D,
  "            && now >= at\n        {\n            self.mpd_retry_at = None;", "            && now > at\n        {\n            self.mpd_retry_at = None;"),
 ("D53", "backoff ignores the attempt (resource)", D,
  "                .delay_us(u64::from(attempt))\n                .max(self.retry_after_us);\n            let at = now.saturating_add(wait);\n            self.retries.insert(id, at);",
  "                .delay_us(1)\n                .max(self.retry_after_us);\n            let at = now.saturating_add(wait);\n            self.retries.insert(id, at);"),
 ("D54", "MPD retry ignores Retry-After", D,
  "let wait = self.backoff.delay_us(attempt).max(self.retry_after_us);", "let wait = self.backoff.delay_us(attempt);"),
 ("D55", "redirect target not passed to the core as the MPD base", D,
  "let final_url = (req.url.as_str() != req.original_url).then(|| req.url.to_string());", "let final_url: Option<String> = None;"),
 ("D56", "response Date not passed on", D,
  "        if let Some(date) = req.head.date.as_deref() {\n            meta = meta.with_date(date);\n        }\n", ""),
 ("D57", "ETag not passed on", D,
  "        if let Some(etag) = req.head.etag.as_deref() {\n            meta = meta.with_etag(etag);\n        }\n", ""),
 ("D58", "UTC response Date not passed on", D,
  "        if let Some(date) = req.head.date.as_deref() {\n            response = response.with_date(date);\n        }\n", ""),
 ("D59", "request start not recorded", D,
  "        req.started_us = now;\n        req.responded_ok = false;", "        req.responded_ok = false;"),
 ("D60", "response arrival not recorded", D,
  "            req.received_us = now;\n            req.head = Head::of(headers);", "            req.head = Head::of(headers);"),
 ("D61", "a challenged request is not parked for credentials", D,
  "                    self.events.extend(events);\n                    self.parked.push(req);", "                    self.events.extend(events);"),
 ("D62", "parked requests not replayed on resume", D,
  "        for req in parked {\n            self.send(req, auth, now);\n        }\n        self.advance(now, auth);", "        let _ = parked;\n        self.advance(now, auth);"),
 ("D63", "a foreign origin's 401 prompts for credentials", D,
  "            && req.url.origin() == auth.origin().origin()\n", ""),
 ("D64", "protected-set drop not remembered", D,
  "            self.encryption_seen = true;\n", ""),
 ("D65", "undecodable init not remembered", D,
  "                self.init_undecodable.insert(id);\n", ""),
 ("D66", "live segment loss reported without its status", D,
  "            Output::SegmentLost { status, .. } => self.events.push_back(Event::Error {\n                kind: ErrorKind::SegmentFetchFailed { status },",
  "            Output::SegmentLost { status: _, .. } => self.events.push_back(Event::Error {\n                kind: ErrorKind::SegmentFetchFailed { status: None },"),
 ("D67", "every drop reported separately", D,
  "        if self.drop_errors.contains(&kind) {\n            return;\n        }\n", ""),
 ("D68", "all-skipped tracks not fatal", D,
  "                if visible.is_empty()\n                    && let Some(kind) = self.skipped_kind.clone()", "                if false\n                    && let Some(kind) = self.skipped_kind.clone()"),
 ("D69", "encrypted sample entry not recognised", D,
  'const ENCRYPTED_SAMPLE_ENTRIES: [&str; 2] = ["encv", "enca"];', 'const ENCRYPTED_SAMPLE_ENTRIES: [&str; 2] = ["encx", "ency"];'),
 ("D70", "non-fragmented MP4 reason misspelt", D,
  'const REASON_NON_FRAGMENTED_MP4: &str = "non_fragmented_mp4";', 'const REASON_NON_FRAGMENTED_MP4: &str = "progressive_mp4";'),
 ("S01", "Auto never picks DASH for .mpd", S,
  "SourceKind::Auto => has_mpd_extension(&parsed),", "SourceKind::Auto => false,"),
 ("S02", ".mpd extension compared case-sensitively", S,
  '.is_some_and(|e| e.eq_ignore_ascii_case("mpd"))', '.is_some_and(|e| e == "mpd")'),
 ("S03", "a DASH variant switch resets the playout gate", S,
  "if !ended && h.variant_switch_restarts_playout() {", "if !ended {"),
 ("S04", "the playout gate is not engaged for DASH", S,
  "        if use_hls || use_dash {\n            playout.engage", "        if use_hls {\n            playout.engage"),
 ("S05", "set_wall_clock reaches a closed session", S,
  "        self.wall = Some(WallClock::new(now_us, unix_us));\n        if self.closed {\n            return;\n        }",
  "        self.wall = Some(WallClock::new(now_us, unix_us));"),
 ("S06", "set_wall_clock never reaches the DASH source", S,
  "            a.set_wall_clock(now_us, unix_us);", "            let _ = (now_us, unix_us);"),
 ("E01", "core: stopping audio keeps requesting it", DR + "client/engine.rs",
  "            if let Some(rep) = lane.reps.last_mut() {\n                rep.dead = true;\n            }\n            self.periods[pi].selection.chosen_audio = None;",
  "            self.periods[pi].selection.chosen_audio = None;\n            let _ = lane;"),
 ("E02", "core: restarting audio on the same set is a no-op", DR + "client/engine.rs",
  "if current_at == at && !was_dead {", "if current_at == at {"),
 ("E03", "core: a set plays its lowest-bandwidth Representation", DR + "client/engine.rs",
  "            .max_by_key(|r| bandwidth(r))", "            .min_by_key(|r| bandwidth(r))"),
]


def run(cmd):
    return subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)


def main():
    dry = "--dry" in sys.argv
    only = [a for a in sys.argv[1:] if re.match(r"^[DSE]\d", a)]
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
            for cmd in ([CORE_TESTS, DASH_TESTS] if crate == "dash-runtime" else [DASH_TESTS]):
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
