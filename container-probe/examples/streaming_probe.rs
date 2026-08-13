//! `streaming_probe` — demonstrate the `Insufficient` contract.
//!
//! The subtlest part of the API is the distinction between `Insufficient`
//! ("read more bytes") and `Unknown` ("stop"). This example reads a real TS
//! fixture, then feeds `probe_with_budget` a growing prefix — 64 bytes, then
//! doubling — printing the verdict at each step and stopping at the first
//! `Identified`. A caller with a streaming source can use `need_at_least` to
//! decide exactly how much more to buffer before asking again.
//!
//! A TS file is the right choice: its verdict genuinely progresses from
//! `Insufficient` (too few syncs in a short prefix) to `Identified` once enough
//! whole packets are seen — there is no point at which extra bytes stop being
//! useful, which is what `Unknown` would mean.

use container_probe::Probe;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "fixtures/ts/h264_aac.ts".to_string());
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("failed to read {path}: {e}");
            std::process::exit(2);
        }
    };

    println!("streaming probe of {path} ({} bytes total)", data.len());
    println!("feeding a growing prefix, starting at 64 bytes and doubling:");
    println!("  budget     verdict");

    let mut budget = 64usize;
    loop {
        let p = container_probe::probe_with_budget(&data, budget);
        let full = budget >= data.len();
        match &p {
            Probe::Identified {
                format, confidence, ..
            } => {
                println!(
                    "  {budget:<8} IDENTIFIED: {} ({}) — stopping",
                    format.name(),
                    confidence.name()
                );
                break;
            }
            Probe::Insufficient { need_at_least, .. } => {
                println!("  {budget:<8} Insufficient: supply >= {need_at_least} bytes (read more)");
            }
            Probe::Unknown => {
                println!(
                    "  {budget:<8} Unknown — a TS at this prefix would be Insufficient; unexpected"
                );
            }
            Probe::Ambiguous { .. } => {
                println!("  {budget:<8} Ambiguous; stopping");
                break;
            }
            _ => {} // `#[non_exhaustive]` requires a wildcard arm.
        }
        if full {
            println!("  exhausted the buffer at {budget} bytes");
            break;
        }
        budget = budget.saturating_mul(2).min(data.len());
    }
}
