#!/usr/bin/env python3
"""Apply each mutant of docs/mutation-table-live.md and run the dash-runtime tests.

    python3 dash-runtime/docs/mutants_live.py --dry   # every pattern matches once
    python3 dash-runtime/docs/mutants_live.py M05 M19

Commit first; each edit is reverted with git checkout."""
import pathlib, subprocess, sys
ROOT = str(pathlib.Path(__file__).resolve().parents[2])
FLT = "all()"
M = [
 ('M01', 'dash-runtime/src/timing/availability.rs', 'ticks_to_us(start_ticks.saturating_add(duration_ticks), timescale),', 'ticks_to_us(start_ticks, timescale),', 'all()'),
 ('M02', 'dash-runtime/src/timing/availability.rs', '        .saturating_sub(ato_us)\n}', '        .saturating_add(ato_us)\n}', 'all()'),
 ('M03', 'dash-runtime/src/timing/availability.rs', '            .saturating_add(tsb)\n    })', '            .saturating_add(0)\n    })', 'all()'),
 ('M04', 'dash-runtime/src/timing/live_edge.rs', 's.available_from_us.is_some_and(|a| a <= utc_now_us)', 's.available_from_us.is_some_and(|a| a < utc_now_us)', 'all()'),
 ('M05', 'dash-runtime/src/timing/live_edge.rs', 's.available_until_us.is_some_and(|u| u < utc_now_us)', 's.available_until_us.is_some_and(|u| u <= utc_now_us)', 'all()'),
 ('M06', 'dash-runtime/src/timing/live_edge.rs', '        .saturating_sub(delay_us)\n        .saturating_sub(ast_us', '        .saturating_add(delay_us)\n        .saturating_sub(ast_us', 'all()'),
 ('M07', 'dash-runtime/src/timing/live_edge.rs', 'pub const MIN_PRESENTATION_DELAY_US: i128 = 4 * MICROS_PER_SECOND;', 'pub const MIN_PRESENTATION_DELAY_US: i128 = 0;', 'all()'),
 ('M08', 'dash-runtime/src/timing/live_edge.rs', 'Some(at.clamp(w.oldest, w.newest))', 'Some(at)', 'all()'),
 ('M09', 'dash-runtime/src/address/plan.rs', 'Some(s) if s.start_us <= period_time_us => lo = mid + 1,', 'Some(s) if s.start_us < period_time_us => lo = mid + 1,', 'all()'),
 ('M10', 'dash-runtime/src/timing/utc_timing.rs', '        seconds_us.saturating_add(half_second),', '        seconds_us,', 'all()'),
 ('M11', 'dash-runtime/src/timing/utc_timing.rs', 'mono_us: exchange.sent_us + half,', 'mono_us: exchange.sent_us.saturating_sub(half),', 'all()'),
 ('M12', 'dash-runtime/src/timing/clock.rs', 'if policy.allow_downgrade {', 'if true {', 'all()'),
 ('M13', 'dash-runtime/src/timing/clock.rs', 'if diff > bars {', 'if diff >= bars {', 'all()'),
 ('M14', 'dash-runtime/src/timing/clock.rs', 'now_us.saturating_sub(old.mono_us) >= policy.resync_us', 'now_us.saturating_sub(old.mono_us) > policy.resync_us', 'all()'),
 ('M15', 'dash-runtime/src/client/engine/live.rs', 'mup.map(|m| mid.saturating_add(m.max(floor)))', 'mup.map(|m| mid.saturating_add(m))', 'all()'),
 ('M16', 'dash-runtime/src/client/engine/live.rs', '(Some(n), Some(o)) if n < o', '(Some(n), Some(o)) if n > o', 'all()'),
 ('M17', 'dash-runtime/src/client/engine/live.rs', '        self.note_exchange(now_us, meta);\n        self.schedule_refresh(now_us);\n        Ok(())', '        self.note_exchange(now_us, meta);\n        self.mpd_failed(now_us);\n        Ok(())', 'all()'),
 ('M18', 'dash-runtime/src/client/engine/live.rs', 'validators = live.refresh.validators.clone();', 'validators = MpdValidators::default();', 'all()'),
 ('M19', 'dash-runtime/src/client/engine/live.rs', '(stale, live.refresh.body == bytes)', '(stale, false)', 'all()'),
 ('M20', 'dash-runtime/src/client/engine/live.rs', '        live.refresh.redirect_url.clone()\n    }', '        None\n    }', 'all()'),
 ('M21', 'dash-runtime/src/client/engine/live.rs', '|| from.is_some_and(|f| utc < f.saturating_add(grace));', '|| from.is_some_and(|f| utc < f);', 'all()'),
 ('M22', 'dash-runtime/src/client/engine/live.rs', 'status == Some(STATUS_TOO_EARLY)\n                || from', 'false\n                || from', 'all()'),
 ('M23', 'dash-runtime/src/client/engine/live.rs', 'if until.is_some_and(|u| utc > u) {', 'if false && until.is_some_and(|u| utc > u) {', 'all()'),
 ('M24', 'dash-runtime/src/client/engine.rs', 'if count >= self.max_failures(id) {', 'if count > self.max_failures(id) {', 'all()'),
 ('M25', 'dash-runtime/src/client/engine/live.rs', 'let delay = base.saturating_mul(1u64 << shift).min(cap);', 'let delay = base.min(cap);', 'all()'),
 ('M26', 'dash-runtime/src/client/engine/live.rs', 'let delay = base.saturating_mul(1u64 << shift).min(cap);', 'let delay = base.saturating_mul(1u64 << shift);', 'all()'),
 ('M27', 'dash-runtime/src/client/engine/live.rs', 'const EXPIRED_REJOIN_RUN: u32 = 3;', 'const EXPIRED_REJOIN_RUN: u32 = 1_000_000;', 'all()'),
 ('M28', 'dash-runtime/src/client/engine.rs', '        if !first && !continuous {\n            self.discontinuity();\n        }', '        if !first && !continuous {}', 'all()'),
 ('M29', 'dash-runtime/src/client/engine.rs', 'SlotState::Marker => self.discontinuity(),', 'SlotState::Marker => {}', 'all()'),
 ('M30', 'dash-runtime/src/client/engine/live.rs', '            if utc < from {\n                return Candidate::Wait(from);', '            if utc + 1_000_000 < from {\n                return Candidate::Wait(from);', 'all()'),
 ('M31', 'dash-runtime/src/client/engine/live.rs', '                .unwrap_or(i128::MIN)\n                .saturating_add(margin);', '                .unwrap_or(i128::MIN)\n                .saturating_add(0);', 'all()'),
 ('M32', 'dash-runtime/src/client/engine/live.rs', '.with_availability_time_offset(self.dash.use_availability_time_offset);', '.with_availability_time_offset(false);', 'all()'),
 ('M33', 'dash-runtime/src/client/engine/live.rs', 'utc < ast.saturating_add(end).saturating_add(grace)', 'false', 'all()'),
 ('M34', 'dash-runtime/src/client/engine.rs', 'self.outstanding < self.cfg.window()', 'self.outstanding <= self.cfg.window()', 'all()'),
 ('M35', 'dash-runtime/src/client/engine.rs', '.periods.first().is_some_and(|p| p.finished)', '.periods.first().is_some_and(|_| false)', 'test(/twenty_four/)'),
 ('M36', 'dash-runtime/src/client/engine.rs', 'while self.remembered_done.len() > MAX_REMEMBERED_DONE {', 'while self.remembered_done.len() > usize::MAX {', 'test(/twenty_four/)'),
 ('M37', 'dash-runtime/src/client/engine/live.rs', 'old.availability_start_time_us != new.availability_start_time_us', 'false', 'all()'),
 ('M38', 'dash-runtime/src/client/engine/live.rs', '        let stepped = matches!(outcome, OfferOutcome::Adopted(AdoptReason::Step));\n        let ready = {', '        let stepped = false && matches!(outcome, OfferOutcome::Adopted(AdoptReason::Step));\n        let ready = {', 'all()'),
 ('M39', 'dash-runtime/src/client/utc_sampler.rs', 'let head_refused = inflight.method == UtcMethod::Head', 'let head_refused = false && inflight.method == UtcMethod::Head', 'all()'),
 ('M40', 'dash-runtime/src/client/utc_sampler.rs', 'if !inflight.rtt_retried => {', 'if true => {', 'all()'),
 ('M41', 'dash-runtime/src/client/utc_sampler.rs', 'deadline_us: now_us.saturating_add(cfg.utc_timeout_us),', 'deadline_us: now_us,', 'all()'),
 ('M42', 'dash-runtime/src/client/engine/live.rs', 'sampler.wait_deadline_us = Some(now_us.saturating_add(self.dash.utc_wait_us));', 'sampler.wait_deadline_us = Some(now_us);', 'all()'),
 ('M43', 'dash-runtime/src/client/engine/live.rs', 'const FORCED_RESYNC_MIN_GAP_US: u64 = 30_000_000;', 'const FORCED_RESYNC_MIN_GAP_US: u64 = 0;', 'all()'),
 ('M44', 'dash-runtime/src/client/engine/live.rs', 'if self.clock.anchor().is_none() {\n            self.go_fatal(FatalKind::ClockUnavailable);', 'if false {\n            self.go_fatal(FatalKind::ClockUnavailable);', 'all()'),
 ('M45', 'dash-runtime/src/client/engine/live.rs', 'let host_first = self.dash.clock_policy == DashClockPolicy::PreferHost', 'let host_first = false && self.dash.clock_policy == DashClockPolicy::PreferHost', 'all()'),
 ('M46', 'dash-runtime/src/client/utc_sampler.rs', '                    if let Some(url) = accept(v) {\n                        last.push(Cand::Http {', '                    if let Some(url) = accept(v) {\n                        first.push(Cand::Http {', 'all()'),
 ('M47', 'dash-runtime/src/client/engine.rs', '.ok_or(Error::NotSwitchable)?,\n            };', '.unwrap_or(0),\n            };', 'all()'),
 ('M48', 'dash-runtime/src/client/engine/live.rs', '        if let Some((pi, li, _)) = self.rep_loc.get(&rep).copied()\n            && let Some(lane) = self.periods.get_mut(pi).and_then(|p| p.lanes.get_mut(li))\n        {\n            lane.expired_run = 0;', '        if let Some((pi, li, _)) = self.rep_loc.get(&rep).copied()\n            && let Some(lane) = self.periods.get_mut(pi).and_then(|p| p.lanes.get_mut(li))\n        {\n            lane.expired_run = lane.expired_run;', 'all()'),
 ('M49', 'dash-runtime/src/client/engine/live.rs', '                && m.media_presentation_duration.is_none()', '                && true', 'all()'),
 ('M50', 'dash-runtime/src/client/engine/live.rs', '        self.force_refresh_after(now_us, wait);', '        let _ = wait;', 'all()'),
 ('M51', 'dash-runtime/src/client/engine/live.rs', '            ClockSource::UtcTiming,\n            now_us,\n            OfferPolicy::new(self.dash.utc_resync_us, false),\n        );\n        let stepped', '            ClockSource::Host,\n            now_us,\n            OfferPolicy::new(self.dash.utc_resync_us, false),\n        );\n        let stepped', 'all()'),
 ('M52', 'dash-runtime/src/client/engine/live.rs', 'l.refresh.last_request_us = Some(meta.sent_us.min(meta.received_us));', '', 'all()'),
 ('M53', 'dash-runtime/src/client/engine.rs', '(exhausted && !self.period_open(pi, now_us))', 'exhausted', 'all()'),
 ('M54', 'dash-runtime/src/client/engine/live.rs', 'if !self.mpd_inflight || self.live.is_none() {\n            return Err(Error::MpdNotAwaited);', 'if self.live.is_none() {\n            return Err(Error::MpdNotAwaited);', 'all()'),
 ('M55', 'dash-runtime/src/client/engine/live.rs', 'Some(a) if a > utc => Candidate::Wait(a),', 'Some(a) if a >= utc => Candidate::Wait(a),', 'all()'),
 ('M56', 'dash-runtime/src/client/engine/live.rs', '        if from.is_some_and(|f| utc >= f) {\n            self.request_resync(now_us);', '        if false {\n            self.request_resync(now_us);', 'all()'),
 ('M57', 'dash-runtime/src/address/plan.rs', 'let number = start_number.saturating_add(index);', 'let number = start_number.saturating_add(index + 1);', 'all()'),
 ('M58', 'dash-runtime/src/address/plan.rs', 'i128::from(s.time) - i128::from(self.pto),\n                    s.duration,\n                    self.reference(s.number, s.time, None),', 'i128::from(s.time),\n                    s.duration,\n                    self.reference(s.number, s.time, None),', 'all()'),
 ('M59', 'dash-runtime/src/timing/live_edge.rs', '    count.checked_sub(1)\n}', '    Some(count)\n}', 'all()'),
 ('M60', 'dash-runtime/src/client/engine/live.rs', 'Some(fetch.saturating_add(mup))', 'None', 'all()'),
]

def main():
    args = sys.argv[1:]
    dry = "--dry" in args
    only = [a for a in args if a != "--dry"]
    for mid, f, old, new, flt in M:
        if only and mid not in only:
            continue
        path = pathlib.Path(ROOT) / f
        src = path.read_text()
        if src.count(old) != 1:
            print(mid, "pattern matches", src.count(old)); continue
        if dry:
            print(mid, "ok"); continue
        path.write_text(src.replace(old, new, 1))
        try:
            p = subprocess.run(["cargo", "nextest", "run", "-p", "dash-runtime", "--all-features", "--locked", "-E", flt], cwd=ROOT, capture_output=True, text=True)
        finally:
            subprocess.run(["git", "checkout", "--", f], cwd=ROOT, check=True)
        out = p.stdout + p.stderr
        status = "invalid" if "could not compile" in out else ("killed" if p.returncode else "SURVIVED")
        print(mid, status)

main()
