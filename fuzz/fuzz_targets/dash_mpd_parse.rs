#![no_main]

use dash_runtime::address::{PlanOptions, plan_representation};
use dash_runtime::mpd::Mpd;
use dash_runtime::selection::{Policy, RepRef, select};
use dash_runtime::timing::live_edge::{join_index, newest_complete, window};
use dash_runtime::timing::period::period_timings;
use libfuzzer_sys::fuzz_target;

const MAX_INPUT: usize = 1 << 20;
const SEGMENTS_WALKED: usize = 64;
const SAMPLE_CLOCK_US: i128 = 1_800_000_000_000_000;

// Fuzz the MPD parser and everything built on a parsed MPD: selection, Period
// timing and the segment plan of every Representation. Must not panic and must
// do bounded work whatever the bytes claim (billion-laughs entities, huge
// `@r`, deep nesting, huge attribute values).
fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }
    let Ok(mpd) = Mpd::parse(data) else { return };
    let _ = period_timings(&mpd);
    for (pi, period) in mpd.periods.iter().enumerate() {
        let _ = select(&mpd, pi, &Policy::default());
        for (si, set) in period.adaptation_sets.iter().enumerate() {
            for ri in 0..set.representations.len() {
                let Ok(planned) =
                    plan_representation(&mpd, RepRef::at(pi, si, ri), &PlanOptions::default())
                else {
                    continue;
                };
                let plan = &planned.plan;
                let _ = plan.init();
                let _ = plan.index();
                for seg in plan.iter().take(SEGMENTS_WALKED) {
                    let _ = planned.candidates.first().and_then(|c| c.resolve(None, &seg.reference));
                }
                if plan.is_dynamic() {
                    let _ = newest_complete(plan, SAMPLE_CLOCK_US);
                    let _ = window(plan, SAMPLE_CLOCK_US);
                    let _ = join_index(plan, SAMPLE_CLOCK_US, 4_000_000);
                }
            }
        }
    }
});
