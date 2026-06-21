//! `dvb-tools` — small CLI over the `rust-dvb` analyzer crates.
//!
//! Subcommands:
//!
//! ```text
//! dvb-tools dump     <file.ts> [--json]
//! dvb-tools services <file.ts>
//! dvb-tools epg      <file.ts> [--json]
//! dvb-tools pids     <file.ts>
//! dvb-tools t2mi     <file> [--pid 0xNNN|raw] [--inner] [--plp N]
//! ```
//!
//! `-h` / `--help` or no subcommand prints the usage block to stderr and
//! exits successfully. Unknown subcommands also print the usage block to
//! stderr but exit with a failure status.

mod dump;
mod epg;
mod pids;
mod services;
mod t2mi;
mod util;

use std::process::ExitCode;

const USAGE: &str = "\
usage: dvb-tools <subcommand> [args...]

  dump     <file.ts> [--json]                         SI section dump (decodes PMT/SDT/NIT descriptor loops, incl. NorDig/EACEM LCN)
  services <file.ts>                                 SDT + NIT/LCN service tree
  epg      <file.ts> [--json]                        EIT schedule
  pids     <file.ts>                                 PID table + bitrate
  t2mi     <file> [--pid 0xNNN|raw] [--inner] [--plp N]
                                                  T2-MI dump / inner-TS extraction";

pub(crate) fn print_usage() {
    eprintln!("{USAGE}");
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let sub = match args.get(1).map(String::as_str) {
        Some(sub) => sub,
        None => {
            print_usage();
            return ExitCode::SUCCESS;
        }
    };

    let rest: &[String] = if args.len() > 2 { &args[2..] } else { &[] };

    match sub {
        "-h" | "--help" | "help" => {
            print_usage();
            ExitCode::SUCCESS
        }
        "dump" => dump::run(rest),
        "services" => services::run(rest),
        "epg" => epg::run(rest),
        "pids" => pids::run(rest),
        "t2mi" => t2mi::run(rest),
        other => {
            eprintln!("dvb-tools: unknown subcommand '{other}'");
            print_usage();
            ExitCode::FAILURE
        }
    }
}
