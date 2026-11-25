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

use std::fs::File;
use std::io::{BufReader, BufWriter, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use clap::Parser;
use mpeg_ts::pid::Pid;
use mpeg_ts::resync::TsResync;
use mpeg_ts::ts::TS_PACKET_SIZE;
use ts_fix::{PcrRestamp, PidFilter, Stuffing, TsFix};

#[derive(Parser)]
#[command(
    name = "ts-fix",
    version,
    about = "MPEG-2 TS repair / remux engine (ISO/IEC 13818-1 §2.4)",
    long_about = "Reads an input .ts file, applies repair operations, writes the repaired stream.\n\
                   Operations are applied in the engine's canonical order (filter → psi-regen → \n\
                   continuity → stuffing → restamp-pcr/honor-pcr-discontinuity) regardless of \n\
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
        value_parser = parse_pid,
        help = "Comma-separated PIDs to keep (PAT PID 0x0000 always included)"
    )]
    keep_pids: Option<Vec<Pid>>,

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
        help = "Rebuild PAT from observed PMT PIDs, replacing the PAT in-position as it \
                passes (or emitting one at end-of-stream if the input has no PAT)"
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

/// clap value parser for a decimal PID, rejecting values outside the 13-bit
/// range (ISO/IEC 13818-1 §2.4.3.2) instead of silently never matching them.
fn parse_pid(s: &str) -> Result<Pid, String> {
    let value: u16 = s.parse().map_err(|e| format!("{e}"))?;
    Pid::try_new(value).map_err(|e| e.to_string())
}

/// Read granularity for streaming the input; memory stays flat on a
/// multi-gigabyte capture.
const READ_CHUNK: usize = 1 << 20;

/// One `read`, retried on `Interrupted`.
fn read_chunk(reader: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    loop {
        match reader.read(buf) {
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            other => return other,
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

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

    let input =
        File::open(&cli.input).map_err(|e| format!("cannot read {}: {e}", cli.input.display()))?;
    let mut reader = BufReader::with_capacity(READ_CHUNK, input);
    let output = File::create(&cli.output)
        .map_err(|e| format!("cannot write {}: {e}", cli.output.display()))?;
    let mut out = BufWriter::with_capacity(READ_CHUNK, output);

    // Resynchronise the input byte stream rather than assuming it starts
    // packet-aligned and is a clean multiple of 188: a capture that begins
    // mid-packet, carries a corrupted sync byte, or is 204-byte RS-coded
    // must still be repaired, not aborted (#1101 W-TF-5).
    let mut resync = TsResync::new();
    let mut chunk = vec![0u8; READ_CHUNK];
    let mut aligned: Vec<[u8; TS_PACKET_SIZE]> = Vec::new();
    let mut written_packets: u64 = 0;
    let mut written_bytes: u64 = 0;

    loop {
        let read = read_chunk(&mut reader, &mut chunk)
            .map_err(|e| format!("cannot read {}: {e}", cli.input.display()))?;
        if read == 0 {
            break;
        }
        resync.feed_into(&chunk[..read], &mut aligned);
        for packet in aligned.drain(..) {
            push_and_write(
                &mut engine,
                &packet,
                &mut out,
                &cli.output,
                &mut written_packets,
                &mut written_bytes,
            )?;
        }
    }
    resync.finish();
    for packet in resync.take_tail_packets() {
        push_and_write(
            &mut engine,
            &packet,
            &mut out,
            &cli.output,
            &mut written_packets,
            &mut written_bytes,
        )?;
    }
    flush_engine(
        &mut engine,
        &mut out,
        &cli.output,
        &mut written_packets,
        &mut written_bytes,
    )?;
    out.flush()
        .map_err(|e| format!("cannot write {}: {e}", cli.output.display()))?;

    let stats = resync.stats();
    if stats.resyncs != 0 || stats.dropped_bytes != 0 {
        eprintln!(
            "input resynchronised: {} lost-sync events, {} bytes dropped",
            stats.resyncs, stats.dropped_bytes
        );
    }
    eprintln!(
        "wrote {written_packets} packets ({written_bytes} bytes) to {}",
        cli.output.display(),
    );

    Ok(())
}

/// Feed one aligned packet through the engine, writing every emitted packet.
fn push_and_write(
    engine: &mut TsFix,
    packet: &[u8],
    out: &mut BufWriter<File>,
    output: &Path,
    written_packets: &mut u64,
    written_bytes: &mut u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut write_error: Option<std::io::Error> = None;
    engine.push(packet, |emitted| {
        write_emitted(
            out,
            emitted,
            &mut write_error,
            written_packets,
            written_bytes,
        );
    })?;
    write_error.map_or(Ok(()), |e| Err(write_failure(output, &e)))
}

/// Flush the engine's buffered state, writing every emitted packet.
fn flush_engine(
    engine: &mut TsFix,
    out: &mut BufWriter<File>,
    output: &Path,
    written_packets: &mut u64,
    written_bytes: &mut u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut write_error: Option<std::io::Error> = None;
    engine.finish(|emitted| {
        write_emitted(
            out,
            emitted,
            &mut write_error,
            written_packets,
            written_bytes,
        );
    });
    write_error.map_or(Ok(()), |e| Err(write_failure(output, &e)))
}

/// Write one emitted packet unless an earlier write already failed.
fn write_emitted(
    out: &mut BufWriter<File>,
    emitted: &[u8],
    write_error: &mut Option<std::io::Error>,
    written_packets: &mut u64,
    written_bytes: &mut u64,
) {
    if write_error.is_some() {
        return;
    }
    match out.write_all(emitted) {
        Ok(()) => {
            *written_packets += 1;
            *written_bytes = written_bytes.saturating_add(emitted.len() as u64);
        }
        Err(e) => *write_error = Some(e),
    }
}

/// The message a failed output write reports.
fn write_failure(output: &Path, e: &std::io::Error) -> Box<dyn std::error::Error> {
    format!("cannot write {}: {e}", output.display()).into()
}
