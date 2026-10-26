#!/usr/bin/env python3
"""Apply each mutant of docs/mutation-table.md to the production source, run the
crate's tests, and report whether any test failed. Every edit is reverted.

    python3 dash-runtime/docs/mutants.py --dry     # check every pattern matches once
    python3 dash-runtime/docs/mutants.py           # run all
    python3 dash-runtime/docs/mutants.py M05 M19   # run some

Commit or stash first: a crash mid-run could leave a mutated file behind (it is
restored in a finally block, but `git diff` is the check). A mutant that makes a
test loop forever has to be killed by hand (M53 does, in the crate doctest).
"""
import json
import os
import pathlib
import re
import subprocess
import sys
ROOT = str(pathlib.Path(__file__).resolve().parents[2])
C = "dash-runtime/src/"
M = [
 ("M01","template width bound `>` -> `>=`", C+"address/template.rs","if width > MAX_FORMAT_WIDTH {","if width >= MAX_FORMAT_WIDTH {"),
 ("M02","template zero flag constant '0' -> '1'", C+"address/template.rs","const TAG_ZERO_FLAG: char = '0';","const TAG_ZERO_FLAG: char = '1';"),
 ("M03","`$$` escape not recognised", C+"address/template.rs","if let Some(after) = tail.strip_prefix(DELIMITER) {","if let Some(after) = tail.strip_prefix('\\u{0}') {"),
 ("M04","media template allows Number and Time together", C+"address/template.rs","if used.number && used.time {","if false {"),
 ("M05","negative r count: ceil -> floor", C+"address/timeline.rs","Some(end) => end.saturating_sub(time).div_ceil(s.d),","Some(end) => end.saturating_sub(time) / s.d,"),
 ("M06","@n ignored", C+"address/timeline.rs","                number = n;\n","                let _ = n;\n"),
 ("M07","@t gap treated as contiguous", C+"address/timeline.rs","                time = t;\n            }\n            if let Some(n) = s.n","                let _ = t;\n            }\n            if let Some(n) = s.n"),
 ("M08","r+1 -> r (off by one repeat count)", C+"address/timeline.rs","s.r.unsigned_abs().saturating_add(1)","s.r.unsigned_abs()"),
 ("M09","index_at_or_before not clamped to the run", C+"address/timeline.rs","let k = ((time - run.time) / run.d).min(run.count - 1);","let k = (time - run.time) / run.d;"),
 ("M10","PTO not subtracted from S@t", C+"address/plan.rs","i128::from(s.time) - i128::from(self.pto),","i128::from(s.time),"),
 ("M11","$Number$ base: startNumber + k instead of + k - 1", C+"address/plan.rs","let number = start_number.saturating_add(index);","let number = start_number.saturating_add(index + 1);"),
 ("M12","segment count floor instead of ceil", C+"address/plan.rs","let c = num.div_euclid(den) + i128::from(num.rem_euclid(den) != 0);","let c = num.div_euclid(den);"),
 ("M13","last segment not shortened to the Period end", C+"address/plan.rs","pd.saturating_sub(full).clamp(1, d)","d"),
 ("M14","endNumber cap off by one", C+"address/plan.rs","en - m.start_number + 1\n","en - m.start_number\n"),
 ("M15","ATO sign flipped", C+"timing/availability.rs","        .saturating_sub(ato_us)\n}\n\n/// Availability end","        .saturating_add(ato_us)\n}\n\n/// Availability end"),
 ("M16","TSB omitted from availability end", C+"timing/availability.rs","            .saturating_add(mpd_duration_us.saturating_mul(2))\n            .saturating_add(tsb)","            .saturating_add(mpd_duration_us.saturating_mul(2))"),
 ("M17","availability end counts one duration, not two (segment given in ticks)", C+"timing/availability.rs","let end2 = start_ticks.saturating_add(duration_ticks.saturating_mul(2));","let end2 = start_ticks.saturating_add(duration_ticks);"),
 ("M18","segment duration dropped from availability start", C+"timing/availability.rs","ticks_to_us(start_ticks.saturating_add(duration_ticks), timescale),","ticks_to_us(start_ticks, timescale),"),
 ("M19","newest-complete boundary `<=` -> `<`", C+"timing/live_edge.rs","s.available_from_us.is_some_and(|a| a <= utc_now_us)","s.available_from_us.is_some_and(|a| a < utc_now_us)"),
 ("M20","presentation delay floor: max -> min", C+"timing/live_edge.rs","MIN_PRESENTATION_DELAY_US\n                .max(max_segment_duration_us.saturating_mul(DELAY_SEGMENT_MULTIPLE))","MIN_PRESENTATION_DELAY_US\n                .min(max_segment_duration_us.saturating_mul(DELAY_SEGMENT_MULTIPLE))"),
 ("M21","join time adds the delay", C+"timing/live_edge.rs","        .saturating_sub(delay_us)\n        .saturating_sub(ast_us.saturating_add(period_start_us))","        .saturating_add(delay_us)\n        .saturating_sub(ast_us.saturating_add(period_start_us))"),
 ("M22","join not clamped into the window", C+"timing/live_edge.rs","Some(at.clamp(w.oldest, w.newest))","Some(at)"),
 ("M23","exchange midpoint taken from the received side", C+"timing/utc_timing.rs","mono_us: exchange.sent_us + half,","mono_us: exchange.received_us - half,"),
 ("M24","uncertainty subtracts the resolution", C+"timing/utc_timing.rs","uncertainty_us: half.saturating_add(resolution_us),","uncertainty_us: half.saturating_sub(resolution_us),"),
 ("M25","Date half second dropped", C+"timing/utc_timing.rs","seconds_us.saturating_add(half_second),","seconds_us,"),
 ("M26","same-source treated as higher trust", C+"timing/clock.rs","let reason = if new.source > old.source {","let reason = if new.source >= old.source {"),
 ("M27","step threshold `>` -> `>=`", C+"timing/clock.rs","if diff > bars {","if diff >= bars {"),
 ("M28","resync period `>=` -> `>`", C+"timing/clock.rs","now_us.saturating_sub(old.mono_us) >= policy.resync_us","now_us.saturating_sub(old.mono_us) > policy.resync_us"),
 ("M29","refine precision `<=` -> `<`", C+"timing/clock.rs","} else if new.uncertainty_us <= old.uncertainty_us {","} else if new.uncertainty_us < old.uncertainty_us {"),
 ("M30","downgrade ignores the policy flag", C+"timing/clock.rs","if policy.allow_downgrade {\n                AdoptReason::Downgrade","if true {\n                AdoptReason::Downgrade"),
 ("M31","PeriodStart chain ignores previous duration", C+"timing/period.rs","(Some(d), Some(ps)) => Some(ps.saturating_add(duration_us(d))),","(Some(_), Some(ps)) => Some(ps),"),
 ("M32","early-terminated needs both conditions", C+"timing/period.rs","p.duration.is_some() && (next_has_start || mpd.minimum_update_period.is_some());","p.duration.is_some() && (next_has_start && mpd.minimum_update_period.is_some());"),
 ("M33","inheritance: higher level wins", C+"mpd/inherit.rs","start_number: child.start_number.or(parent.start_number),","start_number: parent.start_number.or(child.start_number),"),
 ("M34","timeline level still inherits @duration", C+"mpd/inherit.rs","    let (duration, timeline) = if child.timeline.is_some() {\n        (None, child.timeline.clone())","    let (duration, timeline) = if child.timeline.is_some() {\n        (parent.duration, child.timeline.clone())"),
 ("M35","BaseURL alternates not ordered by priority", C+"mpd/urls.rs","ordered.sort_by_key(|b| b.priority.unwrap_or(DEFAULT_BASE_PRIORITY));","ordered.sort_by_key(|_| 0u32);"),
 ("M36","useMPDUrlQuery ignored", C+"mpd/urls.rs","if info.use_mpd_url_query\n        && let Some(q)","if let Some(q)"),
 ("M37","sidx anchor off by one", C+"address/base_index.rs",".and_then(|l| l.checked_add(1))",".and_then(|l| l.checked_add(0))"),
 ("M38","video picks the lowest bandwidth", C+"selection.rs",".max_by_key(|ri| set.representations[**ri].bandwidth)",".min_by_key(|ri| set.representations[**ri].bandwidth)"),
 ("M39","language prefix without subtag boundary", C+"selection.rs",".is_some_and(|rest| rest.starts_with(LANG_SUBTAG_SEPARATOR))",".is_some()"),
 ("M40","hour 24 accepted with a fraction", C+"timing/datetime.rs","if minute != 0 || second != 0 || !fraction_zero {","if minute != 0 || second != 0 {"),
 ("M41","zone +14:mm accepted", C+"timing/datetime.rs","|| (h == MAX_ZONE_HOUR && m != 0)",""),
 ("M42","depth bound off by one", C+"mpd/xml.rs","if stack.len() + 1 > limits.max_depth {","if stack.len() + 1 >= limits.max_depth {"),
 ("M43","DOCTYPE accepted", C+"mpd/xml.rs","Event::DocType(_) => return Err(reader.fail(\"DOCTYPE declarations are not allowed\")),","Event::DocType(_) => {}"),
 ("M44","timeline @t going backwards accepted", C+"address/timeline.rs","if t < time {\n                    return Err(AddressError::TimelineNotIncreasing { t, floor: time });\n                }","if false {\n                    return Err(AddressError::TimelineNotIncreasing { t, floor: time });\n                }"),
 ("M45","availability end counts one duration, not two (microsecond form)", C+"timing/availability.rs","            .saturating_add(mpd_duration_us.saturating_mul(2))","            .saturating_add(mpd_duration_us)"),
 ("M46","join picks the segment after T_join", C+"timing/live_edge.rs","let at = plan.index_at_or_before(t).unwrap_or(w.oldest);","let at = plan.index_at_or_before(t).map_or(w.oldest, |i| i + 1);"),
 ("M47","first static Period has no PeriodStart", C+"timing/period.rs","(mpd.kind == PresentationType::Static).then_some(0)","None"),
 ("M48","endNumber does not truncate a timeline run", C+"address/timeline.rs","count += fit.min(run.count);","count += run.count;"),
 ("M49","http-xsdate classified as http-iso", C+"timing/utc_timing.rs","SCHEME_HTTP_XSDATE => Self::HttpXsdate,","SCHEME_HTTP_XSDATE => Self::HttpIso,"),
 ("M50","$Time$ takes S@t - PTO instead of the raw S@t (D3)", C+"address/plan.rs","self.reference(s.number, s.time, None),","self.reference(s.number, s.time.saturating_sub(self.pto), None),"),
 ("M51","hierarchical sidx accepted", C+"address/base_index.rs","if r.reference_type == REFERENCE_TYPE_SIDX {","if false {"),
 ("M52","DASH-IF role Main not recognised", C+"mpd/descriptors.rs","\"main\" => Self::Main,","\"main\" => Self::Other,"),
 ("M53","Period end of the last Period ignores mediaPresentationDuration", C+"timing/period.rs",".then(|| mpd.media_presentation_duration.map(duration_us))",".then(|| None::<i128>)"),
 ("M54","xlink resolve-to-zero Period kept", C+"mpd/parse.rs","fn is_resolve_to_zero(x: &Option<Xlink>) -> bool {\n    x.as_ref().is_some_and(|x| x.href == XLINK_RESOLVE_TO_ZERO)","fn is_resolve_to_zero(x: &Option<Xlink>) -> bool {\n    x.as_ref().is_some_and(|x| x.href != XLINK_RESOLVE_TO_ZERO)"),
 ("M55","timescale default 1 -> 1000", C+"mpd/inherit.rs","pub const DEFAULT_TIMESCALE: u64 = 1;","pub const DEFAULT_TIMESCALE: u64 = 1000;"),
]
def run(cmd):
    return subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
def main():
    dry = "--dry" in sys.argv
    only = [a for a in sys.argv[1:] if a.startswith("M")]
    results = []
    for mid, desc, f, old, new in M:
        if only and mid not in only: continue
        path = os.path.join(ROOT, f)
        src = open(path).read()
        n = src.count(old)
        if n != 1:
            print(f"{mid}: PATTERN COUNT {n} in {f}"); results.append((mid,desc,"BAD-PATTERN",[])); continue
        if dry: print(mid, "ok"); continue
        open(path,"w").write(src.replace(old,new))
        try:
            r = run(["cargo","test","-p","dash-runtime","--locked","--no-fail-fast"])
            out = r.stdout + r.stderr
            compiled = "Compiling dash-runtime" in out
            if "error[" in out or "error: could not compile" in out and "FAILED" not in out:
                status = "COMPILE-ERROR"; killers=[]
            else:
                killers = sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED", out, re.M)))
                status = "KILLED" if killers else "SURVIVED"
            results.append((mid,desc,status,killers,compiled))
            print(f"{mid}: {status} recompiled={compiled} {killers[:4]}", flush=True)
        finally:
            open(path,"w").write(src)
    print(json.dumps(results))
main()
