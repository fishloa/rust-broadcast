//! Replay a per-chunk delivery trace through the controller and print every
//! decision.
//!
//! The trace is the zenith-tv CSV: `arrival_ns,bytes,pcr_27mhz` (the PCR cell
//! is empty for a chunk without one). It is read with `std::fs` at run time.
//! The renderer is the obedient model of the decision contract (it freezes at
//! `Pause`, restarts at `Resume`), ticked every `--tick-ms` (default 50).
//!
//! The playback (PTS) domain is derived from the PCR minus a constant
//! `--offset-ms` (default 500: PCR runs ahead of PTS); decisions do not
//! depend on it, only the printed positions do.
//!
//! ```text
//! cargo run -p live-buffer --example replay_trace -- <trace.csv> \
//!     [--initial-ms 1800] [--tick-ms 50] [--offset-ms 500] [--reconnect-gap-ms 2000]
//! ```
//!
//! A silence longer than `--reconnect-gap-ms` before a chunk is treated as a
//! reconnect: `SourceEnded` at the last chunk, `Reconnected` before the new one.

use live_buffer::{BufferController, ControllerConfig, Event, Hint, WRAP_27MHZ};
use std::process::ExitCode;

const TICKS_PER_US: u64 = 27;

fn arg(args: &[String], name: &str, default: u64) -> u64 {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first().filter(|a| !a.starts_with("--")) else {
        eprintln!(
            "usage: replay_trace <trace.csv> [--initial-ms N] [--tick-ms N] [--offset-ms N] [--reconnect-gap-ms N]"
        );
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            return ExitCode::from(1);
        }
    };
    let tick_us = arg(&args, "--tick-ms", 50) * 1000;
    let offset = arg(&args, "--offset-ms", 500) * 1000 * TICKS_PER_US;
    let gap_us = arg(&args, "--reconnect-gap-ms", 2000) * 1000;
    let mut cfg = ControllerConfig::default();
    cfg.policy.initial_us = arg(&args, "--initial-ms", cfg.policy.initial_us / 1000) * 1000;
    let mut c = match BufferController::new(cfg, None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("bad config: {e}");
            return ExitCode::from(1);
        }
    };

    let mut chunks = Vec::new();
    for line in text.lines().skip(1) {
        let mut f = line.split(',');
        let (Some(a), Some(b), pcr) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        let (Ok(a), Ok(b)) = (a.parse::<u64>(), b.parse::<u32>()) else {
            continue;
        };
        chunks.push((a / 1000, b, pcr.and_then(|p| p.parse::<u64>().ok())));
    }
    let Some(t0) = chunks.first().map(|c| c.0) else {
        eprintln!("empty trace");
        return ExitCode::from(1);
    };

    // Renderer model: (running, clock position at last set, time of last set).
    let mut running = false;
    let (mut clock, mut t_set) = (0u64, 0u64);
    let pos =
        |clock: u64, t_set: u64, now: u64| (clock + (now - t_set) * TICKS_PER_US) % WRAP_27MHZ;
    let mut next_tick = 0u64;
    let mut last_arrival = 0u64;
    let out = |c: &mut BufferController,
               now: u64,
               running: &mut bool,
               clock: &mut u64,
               t_set: &mut u64| {
        while let Some(ev) = c.poll_event() {
            let t = now as f64 / 1e6;
            match ev {
                Event::StartPlaying {
                    media_27mhz,
                    lead_us,
                }
                | Event::Resume {
                    media_27mhz,
                    lead_us,
                } => {
                    *running = true;
                    *clock = media_27mhz;
                    *t_set = now;
                    println!(
                        "{t:9.3}s {} media={media_27mhz} lead={lead_us}us",
                        ev.name()
                    );
                }
                Event::Pause { hold_at_27mhz, .. } => {
                    *clock = hold_at_27mhz;
                    *t_set = now;
                    *running = false;
                    println!("{t:9.3}s {ev:?}");
                }
                Event::TargetChanged { .. } => {}
                other => println!("{t:9.3}s {other:?}"),
            }
        }
    };

    for (i, (arrival, bytes, pcr)) in chunks.iter().enumerate() {
        let arrival = arrival - t0;
        if i > 0 && arrival - last_arrival >= gap_us {
            c.on_hint(last_arrival, Hint::SourceEnded);
            c.on_hint(arrival, Hint::Reconnected);
            out(&mut c, arrival, &mut running, &mut clock, &mut t_set);
        }
        while next_tick <= arrival {
            let p = running.then(|| pos(clock, t_set, next_tick));
            c.on_tick(
                next_tick,
                Some(p.unwrap_or(clock)).filter(|_| running || clock != 0),
            );
            out(&mut c, next_tick, &mut running, &mut clock, &mut t_set);
            next_tick += tick_us;
        }
        c.on_chunk(arrival, *bytes, *pcr);
        if let Some(pcr) = pcr {
            let end = (*pcr + WRAP_27MHZ - offset % WRAP_27MHZ) % WRAP_27MHZ;
            c.on_media(arrival, end);
        }
        last_arrival = arrival;
    }
    let s = c.stats();
    println!(
        "state={} target={}us stalls={} longest_freeze={}us pdv_baseline={}us stall_peak={}us rfc3550_j={}us discontinuities={} hints={}",
        s.state,
        s.target_us,
        s.stalls,
        s.longest_freeze_us,
        s.est.baseline_us,
        s.est.stall_peak_us,
        s.est.rfc3550_jitter_us,
        s.discontinuities,
        s.hints_seen
    );
    ExitCode::SUCCESS
}
