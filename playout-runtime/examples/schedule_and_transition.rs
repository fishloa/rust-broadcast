//! Build a small linear-channel schedule (programme -> ad -> programme) and
//! walk it forward, planning each transition as the playhead reaches it.
//!
//! ```sh
//! cargo run -p playout-runtime --example schedule_and_transition
//! ```

use playout_runtime::schedule::{CodecConfigId, EntryKind, Schedule, ScheduleEntry};
use playout_runtime::transition::next_transition;

/// One second of a 90 kHz channel clock — the unit SCTE-35 cues use, and the
/// natural choice for a schedule that ultimately emits them.
const TICKS_PER_SECOND: u64 = 90_000;

fn main() {
    let mut schedule = Schedule::new();

    // A programme, starting at the top of the hour.
    schedule
        .push(ScheduleEntry {
            id: "programme-morning-news".into(),
            kind: EntryKind::Programme,
            planned_start: 0,
            source_start_pts: 0,
            codec_config: CodecConfigId(1), // the channel's steady-state encode.
        })
        .unwrap();

    // A 30-second ad break, 10 minutes in, cut from an ad asset whose own
    // timeline starts at PTS 5_000 (e.g. a pre-roll trimmed from a longer
    // source file) and encoded with a different profile.
    schedule
        .push(ScheduleEntry {
            id: "ad-insurance-30s".into(),
            kind: EntryKind::Ad,
            planned_start: 10 * 60 * TICKS_PER_SECOND,
            source_start_pts: 5_000,
            codec_config: CodecConfigId(2),
        })
        .unwrap();

    // Return to programme after the break.
    schedule
        .push(ScheduleEntry {
            id: "programme-morning-news".into(),
            kind: EntryKind::Programme,
            planned_start: 10 * 60 * TICKS_PER_SECOND + 30 * TICKS_PER_SECOND,
            source_start_pts: 10 * 60 * TICKS_PER_SECOND, // resuming mid-asset.
            codec_config: CodecConfigId(1),
        })
        .unwrap();

    // Walk the playhead across every transition the schedule contains.
    let mut now_pts = 0u64;
    loop {
        let Some(t) = next_transition(&schedule, now_pts) else {
            println!("no more transitions after pts={now_pts}");
            break;
        };
        println!(
            "transition: {} ({}) -> {} ({}) at pts={} rebase_offset={} discontinuity={}",
            t.from.id,
            t.from.kind,
            t.to.id,
            t.to.kind,
            t.plan.at_pts,
            t.plan.pts_rebase_offset,
            t.plan.discontinuity,
        );
        now_pts = t.plan.at_pts;
    }
}
