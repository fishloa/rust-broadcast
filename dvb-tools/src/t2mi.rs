//! `dvb-tools t2mi <file> [--pid 0xNNN|raw] [--inner] [--plp N]` — T2-MI
//! pump + optional inner-TS extraction.
//!
//! Without `--inner` this is byte-identical to the former
//! `dvb-t2mi/examples/t2mi_dump.rs`. With `--inner` it chains
//! [`dvb_t2mi::inner_ts::InnerTsRecovery`] to recover the inner MPEG-TS
//! carried inside the BBFrames of the T2-MI stream and writes the recovered
//! 188-byte packets to stdout, so a caller can `> inner.ts` and pipe them
//! back into `dvb-tools dump`.
use mpeg_ts::pid::Pid;
use std::io::{BufWriter, Write};
use std::process::ExitCode;

use dvb_t2mi::inner_ts::InnerTsRecovery;
use dvb_t2mi::pump::T2miPump;

use crate::util::{PacketReader, READ_CHUNK, open_file, read_chunk};

/// Default T2-MI PID when none is specified (EN 302 755 V1.4.1 §5.1 — the usual
/// data-piping PID for T2-MI in TS).
const DEFAULT_T2MI_PID: Pid = Pid::new(0x0006);

/// `--pid` value: a TS-encapsulated target PID, or raw bytes.
enum PidArg {
    Ts(Pid),
    Raw,
}

/// Parse `--pid` (decimal or `0x`-prefixed hex) or a literal `raw`.
fn parse_pid(s: &str) -> Option<PidArg> {
    if s.eq_ignore_ascii_case("raw") {
        return Some(PidArg::Raw);
    }
    let value = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u16::from_str_radix(hex, 16).ok()?
    } else {
        s.parse::<u16>().ok()?
    };
    // A value outside the 13-bit PID range can never match a packet: reject it.
    Pid::try_new(value).ok().map(PidArg::Ts)
}

/// Scan `path` for the PLPs carried on `t2mi_pid`. `Ok(Some(id))` for exactly
/// one PLP, `Ok(None)` for none (nothing to recover; the caller proceeds and
/// writes nothing), `Err` (after printing the PLP list) for more than one.
fn discover_single_plp(path: &str, t2mi_pid: Pid) -> Result<Option<u8>, ExitCode> {
    let mut packets = PacketReader::open(path, "dvb-tools t2mi")?;
    let mut rec = InnerTsRecovery::new(t2mi_pid);
    for packet in packets.by_ref() {
        let _ = rec.feed(&packet);
    }
    packets.finish()?;
    single_plp(&rec)
}

/// The sole PLP `rec` has seen (`None` if none); prints the list and fails
/// when there is more than one.
fn single_plp(rec: &InnerTsRecovery) -> Result<Option<u8>, ExitCode> {
    let plps: Vec<u8> = rec.plps_seen().collect();
    match plps.as_slice() {
        [] => Ok(None),
        [only] => Ok(Some(*only)),
        many => {
            let list: Vec<String> = many.iter().map(u8::to_string).collect();
            eprintln!(
                "dvb-tools t2mi: --inner: {} PLPs found ({}); each carries an independent \
                 transport stream, so pick one with --plp N",
                many.len(),
                list.join(", "),
            );
            Err(ExitCode::FAILURE)
        }
    }
}

/// `--inner` without `--plp` on a non-seekable input (pipe, FIFO, process
/// substitution): the input can be read only once, so recover every PLP in a
/// single pass into memory, then emit it only if exactly one PLP was seen. A
/// single-PLP unfiltered recovery is byte-identical to the filtered one.
fn inner_single_pass(path: &str, t2mi_pid: Pid) -> Result<Vec<u8>, ExitCode> {
    let mut packets = PacketReader::open(path, "dvb-tools t2mi")?;
    let mut rec = InnerTsRecovery::new(t2mi_pid);
    let mut buf = Vec::new();
    for packet in packets.by_ref() {
        for inner_pkt in rec.feed(&packet) {
            buf.extend_from_slice(inner_pkt);
        }
    }
    packets.finish()?;
    single_plp(&rec)?;
    let s = rec.stats();
    eprintln!(
        "-- inner packets written: ts_packets={} t2mi_packets={} \
         crc_failures={} malformed={} filtered_bbframes={}",
        s.ts_packets,
        s.t2mi_packets,
        s.crc_failures,
        s.malformed_packets,
        rec.filtered_bbframes(),
    );
    Ok(buf)
}

pub fn run(path: &str, pid_arg: Option<&str>, inner: bool, plp: Option<u8>) -> ExitCode {
    let pid = match pid_arg {
        None => PidArg::Ts(DEFAULT_T2MI_PID),
        Some(s) => match parse_pid(s) {
            Some(p) => p,
            None => {
                eprintln!("dvb-tools t2mi: --pid expects a number (0xNNN or decimal) or 'raw'");
                return ExitCode::FAILURE;
            }
        },
    };

    if plp.is_some() && !inner {
        eprintln!("dvb-tools t2mi: --plp requires --inner");
        return ExitCode::FAILURE;
    }

    if inner && matches!(pid, PidArg::Raw) {
        eprintln!("dvb-tools t2mi: --inner is not valid with --pid raw");
        return ExitCode::FAILURE;
    }

    if inner {
        let PidArg::Ts(t2mi_pid) = pid else {
            // Unreachable — guarded above.
            return ExitCode::FAILURE;
        };
        // Each PLP carries an independent transport stream (own PAT/PIDs):
        // writing several into one output would interleave them into one
        // garbled mux (#1034). Without `--plp`, scan the capture first and
        // refuse a multi-PLP feed, listing the PLPs to pick from.
        let seekable = std::fs::metadata(path).is_ok_and(|m| m.is_file());
        if plp.is_none() && !seekable {
            let buf = match inner_single_pass(path, t2mi_pid) {
                Ok(b) => b,
                Err(code) => return code,
            };
            let mut out = std::io::stdout().lock();
            if let Err(e) = out.write_all(&buf).and_then(|()| out.flush()) {
                return stdout_failure("write stdout", &e);
            }
            return ExitCode::SUCCESS;
        }
        let plp = match plp {
            Some(p) => Some(p),
            None => match discover_single_plp(path, t2mi_pid) {
                Ok(only) => only,
                Err(code) => return code,
            },
        };
        let mut packets = match PacketReader::open(path, "dvb-tools t2mi") {
            Ok(p) => p,
            Err(code) => return code,
        };
        let mut rec = match plp {
            Some(p) => InnerTsRecovery::new_for_plp(t2mi_pid, p),
            None => InnerTsRecovery::new(t2mi_pid),
        };
        let stdout = std::io::stdout();
        let mut out = BufWriter::new(stdout.lock());
        for packet in packets.by_ref() {
            for inner_pkt in rec.feed(&packet) {
                if let Err(e) = out.write_all(inner_pkt) {
                    return stdout_failure("write stdout", &e);
                }
            }
        }
        if let Err(code) = packets.finish() {
            return code;
        }
        if let Err(e) = out.flush() {
            return stdout_failure("flush stdout", &e);
        }
        let s = rec.stats();
        eprintln!(
            "-- inner packets written: ts_packets={} t2mi_packets={} \
             crc_failures={} malformed={} filtered_bbframes={}",
            s.ts_packets,
            s.t2mi_packets,
            s.crc_failures,
            s.malformed_packets,
            rec.filtered_bbframes(),
        );
        return ExitCode::SUCCESS;
    }

    // Default behaviour (no --inner): byte-identical to the old t2mi_dump.
    let mut pump = match pid {
        PidArg::Ts(p) => T2miPump::new(p),
        PidArg::Raw => T2miPump::raw(),
    };

    let stdout = std::io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    match pid {
        PidArg::Ts(_) => {
            let mut packets = match PacketReader::open(path, "dvb-tools t2mi") {
                Ok(p) => p,
                Err(code) => return code,
            };
            for packet in packets.by_ref() {
                for event in pump.feed_ts(&packet) {
                    if let Err(e) = write_event(&mut out, event) {
                        return stdout_failure("write stdout", &e);
                    }
                }
            }
            if let Err(code) = packets.finish() {
                return code;
            }
        }
        PidArg::Raw => {
            let mut file = match open_file(path, "dvb-tools t2mi") {
                Ok(f) => f,
                Err(code) => return code,
            };
            let mut chunk = vec![0u8; READ_CHUNK];
            loop {
                let n = match read_chunk(&mut file, &mut chunk) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("dvb-tools t2mi: {path}: {e}");
                        return ExitCode::FAILURE;
                    }
                };
                for event in pump.feed_raw(&chunk[..n]) {
                    if let Err(e) = write_event(&mut out, event) {
                        return stdout_failure("write stdout", &e);
                    }
                }
            }
        }
    }
    if let Err(e) = out.flush() {
        return stdout_failure("flush stdout", &e);
    }

    let s = pump.stats();
    eprintln!(
        "-- ts_packets={} t2mi_packets={} crc_failures={} malformed={}",
        s.ts_packets, s.t2mi_packets, s.crc_failures, s.malformed_packets
    );
    ExitCode::SUCCESS
}

/// A stdout write failed. A closed pipe (`BrokenPipe`: the consumer — `head`,
/// a closed pager — went away) is the normal way to stop reading and is a
/// clean, quiet success; anything else is a real failure. Either way the
/// caller stops immediately instead of grinding through the rest of the
/// capture and then failing on the final flush (r11-W-DT-3).
fn stdout_failure(what: &str, e: &std::io::Error) -> ExitCode {
    if e.kind() == std::io::ErrorKind::BrokenPipe {
        return ExitCode::SUCCESS;
    }
    eprintln!("dvb-tools t2mi: {what}: {e}");
    ExitCode::FAILURE
}

/// One summary line per T2-MI event to `out`; a header error goes to stderr.
fn write_event(out: &mut impl Write, event: dvb_t2mi::pump::T2miEvent) -> std::io::Result<()> {
    match event.header() {
        Ok(hdr) => writeln!(
            out,
            "type={} (0x{:02X}) count={} superframe_idx={} stream_id={} payload_bits={}",
            event.payload().map(|p| p.name()).unwrap_or("PARSE_ERROR"),
            event.packet_type(),
            hdr.packet_count,
            hdr.superframe_idx,
            hdr.t2mi_stream_id,
            hdr.payload_len_bits,
        ),
        Err(e) => {
            eprintln!(
                "packet_type=0x{:02X} header error: {e}",
                event.packet_type()
            );
            Ok(())
        }
    }
}
