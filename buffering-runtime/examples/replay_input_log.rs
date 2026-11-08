//! Replay an input log through a fresh controller and print every decision.
//!
//! ```text
//! cargo run -p buffering-runtime --example replay_input_log                          # built-in synthetic log
//! cargo run -p buffering-runtime --example replay_input_log -- <log.csv> [--initial-ms 1800]
//! ```
//!
//! With NO arguments it writes a deterministic synthetic log (two tracks, a steady 20 ms chunk
//! cadence, 40 ms video frames and 24 ms audio frames) in the input-log text format, parses it
//! back and replays it, so the example always runs. With a path it replays that log. A log with
//! `config` rows replays with its recorded configuration; a log recorded before `config` rows
//! existed uses the defaults, with `--initial-ms` replacing the cold-start target. The log is
//! read with `std::fs` at run time.

use buffering_runtime::ControllerConfig;
use buffering_runtime::input_log::{Input, InputLog};
use std::fmt::Write as _;
use std::process::ExitCode;

const VIDEO: u32 = 1;
const AUDIO: u32 = 2;
const TICKS_PER_US: u64 = 27;
const CHUNK_PERIOD_US: u64 = 20_000;
const CHUNK_BYTES: u64 = 7 * 188;
const VIDEO_FRAME_US: u64 = 40_000;
const AUDIO_FRAME_US: u64 = 24_000;
const SYNTHETIC_RUN_US: u64 = 4_000_000;
const MEDIA_START_27MHZ: u64 = 900_000 * 300;

fn synthetic_log() -> String {
    let mut text = String::new();
    let _ = writeln!(text, "{}", Input::CSV_HEADER);
    let _ = writeln!(text, "register,0,{VIDEO},");
    let _ = writeln!(text, "register,0,{AUDIO},");
    let mut now = 0;
    while now <= SYNTHETIC_RUN_US {
        let media_27mhz = MEDIA_START_27MHZ + now * TICKS_PER_US;
        let _ = writeln!(text, "chunk,{now},{CHUNK_BYTES},{media_27mhz}");
        let video_end =
            MEDIA_START_27MHZ + (now / VIDEO_FRAME_US + 1) * VIDEO_FRAME_US * TICKS_PER_US;
        let audio_end =
            MEDIA_START_27MHZ + (now / AUDIO_FRAME_US + 1) * AUDIO_FRAME_US * TICKS_PER_US;
        let _ = writeln!(text, "media,{now},{VIDEO},{video_end}");
        let _ = writeln!(text, "media,{now},{AUDIO},{audio_end}");
        let _ = writeln!(text, "tick,{now},0,");
        now += CHUNK_PERIOD_US;
    }
    text
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (name, text) = match args.first() {
        None => ("synthetic".to_string(), synthetic_log()),
        Some(path) => match std::fs::read_to_string(path) {
            Ok(t) => (path.clone(), t),
            Err(e) => {
                eprintln!("{path}: {e}");
                return ExitCode::FAILURE;
            }
        },
    };
    let initial_ms: Option<u64> = args
        .iter()
        .position(|a| a == "--initial-ms")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok());
    let mut fallback = ControllerConfig::default();
    if let Some(ms) = initial_ms {
        fallback.policy.initial_us = ms * 1_000;
    }
    let mut log = match InputLog::parse_with_config(&text, fallback) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{name}: {e}");
            return ExitCode::FAILURE;
        }
    };
    log.rebase();
    match log.replay() {
        Ok(events) => {
            println!(
                "{name}: {} inputs, {} events",
                log.inputs.len(),
                events.len()
            );
            for (now_us, e) in events {
                println!("{:>10.3} s  {e:?}", now_us as f64 / 1e6);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{name}: {e}");
            ExitCode::FAILURE
        }
    }
}
