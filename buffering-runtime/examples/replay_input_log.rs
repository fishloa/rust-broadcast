//! Replay an input log through a fresh controller and print every decision.
//!
//! ```text
//! cargo run -p buffering-runtime --example replay_input_log -- <log.csv> [--initial-ms 1800]
//! ```
//!
//! A log with `config` rows replays with its recorded configuration. A log recorded before
//! `config` rows existed uses the defaults, with `--initial-ms` replacing the cold-start target.
//! The log is read with `std::fs` at run time.

use buffering_runtime::ControllerConfig;
use buffering_runtime::input_log::InputLog;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: replay_input_log <log.csv> [--initial-ms N]");
        return ExitCode::FAILURE;
    };
    let initial_ms: Option<u64> = args
        .iter()
        .position(|a| a == "--initial-ms")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok());
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut fallback = ControllerConfig::default();
    if let Some(ms) = initial_ms {
        fallback.policy.initial_us = ms * 1_000;
    }
    let mut log = match InputLog::parse_with_config(&text, fallback) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    log.rebase();
    match log.replay() {
        Ok(events) => {
            println!("{} inputs, {} events", log.inputs.len(), events.len());
            for (now_us, e) in events {
                println!("{:>10.3} s  {e:?}", now_us as f64 / 1e6);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{path}: {e}");
            ExitCode::FAILURE
        }
    }
}
