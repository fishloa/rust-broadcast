//! Replay a per-chunk delivery trace through the controller and print every
//! decision.
//!
//! With NO arguments it replays a built-in deterministic synthetic trace (a
//! clean 60 s link, one 1.4 s delivery stall at 20 s followed by a line-rate
//! burst of the backlog) with a 500 ms initial target so the stall is visible;
//! the private real traces cannot be a default (they live in the private
//! fixtures repo). With a path it replays a real trace.
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
//! cargo run -p live-buffer --example replay_trace                # built-in synthetic
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

/// 60 s of 35 ms PCR chunks with 0-15 ms deterministic jitter and one 1.4 s
/// stall at 20 s whose backlog arrives as a 10x burst. `(arrival_us, bytes, pcr)`.
fn synthetic() -> Vec<(u64, u32, Option<u64>)> {
    const STEP_US: u64 = 35_000;
    const GAP_AT_US: u64 = 20_000_000;
    const GAP_US: u64 = 1_400_000;
    let (mut seed, mut last, mut burst) = (12345u64, 0u64, None::<u64>);
    let mut v = Vec::new();
    for k in 0..(60_000_000 / STEP_US) {
        let media = k * STEP_US;
        seed = (seed.wrapping_mul(1_103_515_245).wrapping_add(12_345)) & 0x7FFF_FFFF;
        let ideal = media + (seed >> 8) % 15_000;
        let mut arrival = ideal;
        if burst.is_none() && (GAP_AT_US..GAP_AT_US + STEP_US).contains(&ideal) {
            burst = Some(GAP_AT_US + GAP_US);
        }
        if let Some(b) = burst {
            if b > ideal {
                arrival = b;
                burst = Some(b + STEP_US / 10);
            } else {
                burst = None;
            }
        }
        arrival = arrival.max(last);
        last = arrival;
        v.push((
            arrival,
            1316,
            Some((media * TICKS_PER_US + 27_000_000) % WRAP_27MHZ),
        ));
    }
    v
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().filter(|a| !a.starts_with("--"));
    let tick_us = arg(&args, "--tick-ms", 50) * 1000;
    let offset = arg(&args, "--offset-ms", 500) * 1000 * TICKS_PER_US;
    let gap_us = arg(&args, "--reconnect-gap-ms", 2000) * 1000;
    let mut cfg = ControllerConfig::default();
    let default_initial_ms = if path.is_some() {
        cfg.policy.initial_us / 1000
    } else {
        500
    };
    cfg.policy.initial_us = arg(&args, "--initial-ms", default_initial_ms) * 1000;
    let mut c = match BufferController::new(cfg, None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("bad config: {e}");
            return ExitCode::from(1);
        }
    };

    let mut chunks: Vec<(u64, u32, Option<u64>)> = Vec::new();
    if let Some(path) = path {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                return ExitCode::from(1);
            }
        };
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
    } else {
        chunks = synthetic();
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
                (running || clock != 0).then_some(p.unwrap_or(clock)),
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
