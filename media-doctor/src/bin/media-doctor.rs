//! `media-doctor` CLI binary.

use std::fs;
use std::process;

use clap::Parser;
use media_doctor::cli::{CheckArgs, Cli};
use media_doctor::{
    CcAnomalyCheck, CodecSignallingCheck, Diagnostic, FpsCadenceCheck, InterlaceCheck,
    ParamSetsCheck, PatPmtVersionCheck, PcrCheck, PtsCheck, Report, Scte35Check, SyncByteCheck,
    check_container_codec, run_all,
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

/// A minimal MPEG-2 TS sniff: sync byte `0x47` at both packet 0 and packet 1
/// (ISO/IEC 13818-1 §2.4.3.2). Used only to pick which diagnostic set the
/// CLI runs — an ISOBMFF/CMAF file's first bytes essentially never match
/// this by chance, and running the TS-only diagnostics against MP4 bytes
/// (or vice versa) produces meaningless noise (e.g. a `sync-byte` error per
/// "packet") rather than a crash, so this is a UX choice, not a correctness
/// requirement.
fn looks_like_ts(bytes: &[u8]) -> bool {
    const TS_PACKET_SIZE: usize = 188;
    bytes.len() >= 2 * TS_PACKET_SIZE && bytes[0] == 0x47 && bytes[TS_PACKET_SIZE] == 0x47
}

fn run_check(args: &CheckArgs) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = fs::read(&args.input)?;
    let mut report = Report::new();

    if looks_like_ts(&bytes) {
        let diagnostics: &[&dyn Diagnostic] = &[
            &SyncByteCheck,
            &PatPmtVersionCheck,
            &CcAnomalyCheck,
            &PcrCheck,
            &PtsCheck,
            &Scte35Check,
            &CodecSignallingCheck,
            &FpsCadenceCheck,
            &ParamSetsCheck,
            &InterlaceCheck,
        ];
        run_all(&bytes, diagnostics, &mut report);
    } else {
        check_container_codec(&bytes, &mut report);
    }

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
