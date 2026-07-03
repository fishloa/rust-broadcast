//! `media-doctor` CLI binary.

use std::fs;
use std::process;

use clap::Parser;
use media_doctor::cli::{CheckArgs, Cli};
use media_doctor::{
    run_all, CcAnomalyCheck, Diagnostic, PatPmtVersionCheck, PcrCheck, PtsCheck, Report,
    Scte35Check, SyncByteCheck,
};

fn main() {
    let cli = Cli::parse();
    match cli {
        Cli::Check(args) => {
            if let Err(e) = run_check(&args) {
                eprintln!("error: {e}");
                process::exit(1);
            }
        }
    }
}

fn run_check(args: &CheckArgs) -> Result<(), Box<dyn std::error::Error>> {
    let ts = fs::read(&args.input)?;
    let mut report = Report::new();
    let diagnostics: &[&dyn Diagnostic] = &[
        &SyncByteCheck,
        &PatPmtVersionCheck,
        &CcAnomalyCheck,
        &PcrCheck,
        &PtsCheck,
        &Scte35Check,
    ];
    run_all(&ts, diagnostics, &mut report);

    if args.json {
        #[cfg(feature = "serde")]
        {
            let json = serde_json::to_string_pretty(&report)?;
            println!("{json}");
        }
        #[cfg(not(feature = "serde"))]
        {
            // Should not happen: cli feature implies serde, but be safe.
            eprintln!("JSON output requires the `serde` feature.");
            process::exit(1);
        }
    } else {
        println!("{report}");
    }
    Ok(())
}
