//! Parse a dynamic MPD, simulate a clock and print the live edge: the
//! availability window, the segment to join and when the next one appears.
//!
//! The clock is an argument: `--utc` takes an `xs:dateTime`, default is the
//! MPD's own `publishTime`. A real client feeds a `UtcClock` anchored by
//! `UTCTiming` here; this example shows the arithmetic only.
//!
//! # Usage
//!
//! ```bash
//! cargo run -p dash-runtime --example live_window
//! cargo run -p dash-runtime --example live_window -- --mpd fixtures/dash/live-timeline/live.mpd --utc 2026-10-26T09:18:40Z
//! ```

use std::path::PathBuf;
use std::time::Duration;

use dash_runtime::address::{PlanOptions, plan_representation};
use dash_runtime::mpd::{Mpd, PresentationType};
use dash_runtime::selection::{Policy, select};
use dash_runtime::timing::datetime::parse_xs_datetime;
use dash_runtime::timing::live_edge::{join_index, newest_complete, presentation_delay_us, window};
use dash_runtime::timing::period::duration_us;

const DEFAULT_MPD: &str = "../fixtures/dash/live-number/live.mpd";
const FALLBACK_SEGMENT_US: i128 = 2_000_000;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_MPD);
    let mut utc_arg: Option<String> = None;
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        match flag.as_str() {
            "--mpd" => path = it.next().ok_or("--mpd needs a path")?.into(),
            "--utc" => utc_arg = Some(it.next().ok_or("--utc needs an xs:dateTime")?),
            other => {
                return Err(format!("unknown argument {other} (use --mpd PATH, --utc TIME)").into());
            }
        }
    }
    let mpd = Mpd::parse(&std::fs::read(&path)?)?;
    if mpd.kind != PresentationType::Dynamic {
        return Err("this example needs a dynamic MPD".into());
    }
    let now_us = match utc_arg {
        Some(t) => i128::from(parse_xs_datetime(&t)?.unix_us),
        None => i128::from(mpd.publish_time_us.ok_or("no publishTime; pass --utc")?),
    };
    let sel = select(&mpd, 0, &Policy::default());
    let max_segment = mpd
        .max_segment_duration
        .map_or(FALLBACK_SEGMENT_US, duration_us);
    let tsb = mpd.time_shift_buffer_depth.map(duration_us);
    let delay = presentation_delay_us(mpd.suggested_presentation_delay, max_segment, tsb, None);
    println!(
        "clock {now_us} us, presentation delay {:.3}s (suggested {:?})",
        delay as f64 / 1e6,
        mpd.suggested_presentation_delay
            .map(|d: Duration| d.as_secs_f64())
    );
    for (label, rep) in [
        ("video", sel.chosen_variant.map(|i| sel.variants[i])),
        ("audio", sel.chosen_audio),
    ] {
        let Some(at) = rep else { continue };
        let planned = plan_representation(&mpd, at, &PlanOptions::default())?;
        let plan = &planned.plan;
        match window(plan, now_us) {
            None => println!("{label}: no segment obtainable yet"),
            Some(w) => {
                let show = |i: u64| {
                    plan.get(i)
                        .map(|s| format!("#{} {}", s.number, s.reference))
                        .unwrap_or_default()
                };
                println!(
                    "{label}: window oldest {} .. newest {}",
                    show(w.oldest),
                    show(w.newest)
                );
                if let Some(j) = join_index(plan, now_us, delay) {
                    println!("{label}: join at {}", show(j));
                }
            }
        }
        let next = newest_complete(plan, now_us).map_or(0, |n| n + 1);
        if let Some(seg) = plan.get(next) {
            let wait = seg.available_from_us.unwrap_or(now_us) - now_us;
            println!(
                "{label}: next segment #{} in {:.3}s",
                seg.number,
                wait as f64 / 1e6
            );
        }
    }
    Ok(())
}
