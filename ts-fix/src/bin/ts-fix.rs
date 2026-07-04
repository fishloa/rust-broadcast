//! CLI for the `ts-fix` MPEG-2 TS repair / remux engine.
//!
//! Reads an input `.ts` file, applies the selected repair operations, and writes
//! the repaired stream to an output file.
//!
//! # Example
//!
//! ```bash
//! ts-fix --input corrupt.ts --output repaired.ts --repair-continuity --drop-nulls
//! ```

use std::fs;
use std::path::PathBuf;

use clap::Parser;
use ts_fix::{PcrRestamp, PidFilter, Stuffing, TsFix};

#[derive(Parser)]
#[command(
    name = "ts-fix",
    version,
    about = "MPEG-2 TS repair / remux engine (ISO/IEC 13818-1 §2.4)",
    long_about = "Reads an input .ts file, applies repair operations, writes the repaired stream.\n\
                   Operations are applied in the engine's canonical order (filter → psi-regen → \n\
                   continuity → restamp-pcr/honor-pcr-discontinuity → stuffing) regardless of \n\
                   flag order.  All operations are opt-in."
)]
struct Cli {
    /// Input TS file path.
    #[arg(
        long = "input",
        short = 'i',
        value_name = "PATH",
        help = "Input TS file"
    )]
    input: PathBuf,

    /// Output TS file path.
    #[arg(
        long = "output",
        short = 'o',
        value_name = "PATH",
        help = "Output TS file"
    )]
    output: PathBuf,

    /// Repair continuity counters.
    #[arg(
        long = "repair-continuity",
        help = "Renumber per-PID continuity counters to monotonic (mod 16) sequences"
    )]
    repair_continuity: bool,

    /// Keep only the specified PIDs.
    #[arg(
        long = "keep-pids",
        value_name = "PID,PID,...",
        conflicts_with = "service",
        value_delimiter = ',',
        help = "Comma-separated PIDs to keep (PAT PID 0x0000 always included)"
    )]
    keep_pids: Option<Vec<u16>>,

    /// Extract a single programme by number.
    #[arg(
        long = "service",
        value_name = "PROGRAM",
        conflicts_with = "keep_pids",
        help = "Extract a single programme by program_number (observes PAT/PMT)"
    )]
    service: Option<u16>,

    /// PCR restamp — interpolate between observed PCRs.
    #[arg(
        long = "restamp-pcr-interpolate",
        conflicts_with_all = ["restamp_pcr_bitrate", "honor_pcr_discontinuity"],
        help = "Interpolate PCRs between observed anchors (preserves first PCR)"
    )]
    restamp_pcr_interpolate: bool,

    /// PCR restamp — recompute from a fixed bitrate.
    #[arg(
        long = "restamp-pcr-bitrate",
        value_name = "BPS",
        conflicts_with_all = ["restamp_pcr_interpolate", "honor_pcr_discontinuity"],
        help = "Recompute PCRs from a fixed bitrate in bits/sec (e.g. 27000000)"
    )]
    restamp_pcr_bitrate: Option<u64>,

    /// PCR-discontinuity honor mode — flag genuine unflagged breaks, don't rewrite values.
    #[arg(
        long = "honor-pcr-discontinuity",
        conflicts_with_all = ["restamp_pcr_interpolate", "restamp_pcr_bitrate"],
        help = "Set discontinuity_indicator on genuine, unflagged PCR breaks (TR 101 290 §5.2.2 2.3b) \
                without rewriting any timestamp"
    )]
    honor_pcr_discontinuity: bool,

    /// Regenerate PAT/PMT from observed stream state.
    #[arg(
        long = "regen-psi",
        help = "Rebuild PAT from observed PMT PIDs on flush"
    )]
    regen_psi: bool,

    /// Drop all null packets (PID 0x1FFF).
    #[arg(
        long = "drop-nulls",
        conflicts_with = "pad_to",
        help = "Remove all null packets from the output"
    )]
    drop_nulls: bool,

    /// Pad to a target packet rate (e.g. 2.0 doubles packet count).
    #[arg(
        long = "pad-to",
        value_name = "RATE",
        conflicts_with = "drop_nulls",
        help = "Insert null packets to reach target rate (packets_per_input)"
    )]
    pad_to: Option<f64>,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Read input.
    let input =
        fs::read(&cli.input).map_err(|e| format!("cannot read {}: {e}", cli.input.display()))?;

    // Build the engine.
    let mut builder = TsFix::builder();

    if cli.repair_continuity {
        builder = builder.repair_continuity();
    }

    if let Some(pids) = &cli.keep_pids {
        builder = builder.filter_pids(PidFilter::keep(pids.iter().copied()));
    } else if let Some(program) = cli.service {
        builder = builder.filter_pids(PidFilter::service(program));
    }

    if cli.regen_psi {
        builder = builder.regen_psi();
    }

    if cli.restamp_pcr_interpolate {
        builder = builder.restamp_pcr(PcrRestamp::interpolate());
    } else if let Some(bps) = cli.restamp_pcr_bitrate {
        builder = builder.restamp_pcr(PcrRestamp::from_bitrate(bps));
    } else if cli.honor_pcr_discontinuity {
        builder = builder.honor_pcr_discontinuity();
    }

    if cli.drop_nulls {
        builder = builder.stuffing(Stuffing::drop_nulls());
    } else if let Some(rate) = cli.pad_to {
        builder = builder.stuffing(Stuffing::pad_to(rate));
    }

    let mut engine = builder.build()?;
    let mut output = Vec::with_capacity(input.len());

    for chunk in input.chunks(188) {
        engine.push(chunk, |pkt| output.extend_from_slice(pkt))?;
    }
    engine.finish(|pkt| output.extend_from_slice(pkt));

    fs::write(&cli.output, &output)
        .map_err(|e| format!("cannot write {}: {e}", cli.output.display()))?;

    eprintln!(
        "wrote {} packets ({} bytes) to {}",
        output.len() / 188,
        output.len(),
        cli.output.display(),
    );

    Ok(())
}
