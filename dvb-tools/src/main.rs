//! `dvb-tools` — command-line DVB stream analyzer over the `rust-broadcast` crates.
//!
//! ```text
//! dvb-tools dump     <FILE> [--json]
//! dvb-tools services <FILE>
//! dvb-tools epg      <FILE> [--json]
//! dvb-tools pids     <FILE>
//! dvb-tools t2mi     <FILE> [--pid 0xNNN|raw] [--inner] [--plp N]
//! ```
//!
//! CLI follows the workspace standard (`docs/CLI-STANDARD.md`): `clap` derive,
//! named flags, auto `--help`/`--version`.

mod dump;
mod epg;
mod pids;
mod services;
mod t2mi;
mod util;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// Command-line DVB stream analyzer over the rust-broadcast crates.
#[derive(Parser)]
#[command(name = "dvb-tools", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// SI section dump — decodes PMT/SDT/NIT descriptor loops (incl. NorDig/EACEM LCN).
    Dump {
        /// Input transport-stream file (188- or 204-byte packets).
        file: String,
        /// Emit each decoded table as pretty JSON instead of a summary line.
        #[arg(long)]
        json: bool,
    },
    /// SDT + NIT/LCN service tree.
    Services {
        /// Input transport-stream file.
        file: String,
    },
    /// EIT schedule.
    Epg {
        /// Input transport-stream file.
        file: String,
        /// Emit events as JSON.
        #[arg(long)]
        json: bool,
    },
    /// PID table + bitrate.
    Pids {
        /// Input transport-stream file.
        file: String,
    },
    /// T2-MI dump, or inner-TS extraction with `--inner`.
    T2mi {
        /// Input file (`.t2mi` raw, or a `.ts` carrying T2-MI).
        file: String,
        /// T2-MI source PID (`0x`-hex or decimal), or `raw` for a bare T2-MI
        /// byte stream. Defaults to 0x0006.
        #[arg(long)]
        pid: Option<String>,
        /// Recover and write the inner MPEG-TS to stdout (pipe to `dump`).
        #[arg(long)]
        inner: bool,
        /// With `--inner`, keep only this PLP.
        #[arg(long)]
        plp: Option<u8>,
    },
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Dump { file, json } => dump::run(&file, json),
        Command::Services { file } => services::run(&file),
        Command::Epg { file, json } => epg::run(&file, json),
        Command::Pids { file } => pids::run(&file),
        Command::T2mi {
            file,
            pid,
            inner,
            plp,
        } => t2mi::run(&file, pid.as_deref(), inner, plp),
    }
}
