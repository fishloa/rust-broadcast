//! `t2mi_dump` — pump a T2-MI stream and print one line per packet.
//!
//! Usage:
//!
//! ```text
//! cargo run -p dvb-t2mi --example t2mi_dump -- <file.t2mi|file.ts> [--pid 0xNNN]
//! ```
//!
//! `--pid` selects TS-encapsulated mode (filter that PID, strip TS framing); it
//! defaults to 0x0006 (the usual T2-MI data-piping PID). A `.t2mi` file with no
//! TS framing can be decoded in raw mode by passing `--pid raw`. Each emitted,
//! CRC-valid packet prints its type and header fields; a stats summary follows.

use std::process::ExitCode;

use dvb_t2mi::pump::T2miPump;

/// Parse a PID argument: a number (`0x0006` / `6`) or the literal `raw`.
fn parse_pid(s: &str) -> Option<PidArg> {
    if s.eq_ignore_ascii_case("raw") {
        return Some(PidArg::Raw);
    }
    let value = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u16::from_str_radix(hex, 16).ok()?
    } else {
        s.parse::<u16>().ok()?
    };
    Some(PidArg::Ts(value))
}

enum PidArg {
    Ts(u16),
    Raw,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let mut path: Option<String> = None;
    let mut pid = PidArg::Ts(0x0006);

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--pid" => {
                i += 1;
                match args.get(i).and_then(|s| parse_pid(s)) {
                    Some(p) => pid = p,
                    None => {
                        eprintln!("t2mi_dump: --pid expects a number or 'raw'");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "-h" | "--help" => {
                eprintln!("usage: t2mi_dump <file.t2mi|file.ts> [--pid 0xNNN|raw]");
                return ExitCode::SUCCESS;
            }
            other if other.starts_with('-') => {
                eprintln!("t2mi_dump: unknown option {other}");
                return ExitCode::FAILURE;
            }
            other => path = Some(other.to_string()),
        }
        i += 1;
    }

    let Some(path) = path else {
        eprintln!("usage: t2mi_dump <file.t2mi|file.ts> [--pid 0xNNN|raw]");
        return ExitCode::FAILURE;
    };

    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("t2mi_dump: {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut pump = match pid {
        PidArg::Ts(p) => T2miPump::new(p),
        PidArg::Raw => T2miPump::raw(),
    };

    let print_event = |event: dvb_t2mi::pump::T2miEvent| match event.header() {
        Ok(hdr) => println!(
            "type={} (0x{:02X}) count={} superframe_idx={} stream_id={} payload_bits={}",
            event.payload().map(|p| p.name()).unwrap_or("PARSE_ERROR"),
            event.packet_type(),
            hdr.packet_count,
            hdr.superframe_idx,
            hdr.t2mi_stream_id,
            hdr.payload_len_bits,
        ),
        Err(e) => eprintln!(
            "packet_type=0x{:02X} header error: {e}",
            event.packet_type()
        ),
    };

    match pid {
        PidArg::Ts(_) => {
            for chunk in data.chunks(188) {
                if chunk.len() != 188 {
                    continue;
                }
                for event in pump.feed_ts(chunk) {
                    print_event(event);
                }
            }
        }
        PidArg::Raw => {
            // Feed the whole file; the pump buffers across packet boundaries.
            for event in pump.feed_raw(&data) {
                print_event(event);
            }
        }
    }

    let s = pump.stats();
    eprintln!(
        "-- ts_packets={} t2mi_packets={} crc_failures={} malformed={}",
        s.ts_packets, s.t2mi_packets, s.crc_failures, s.malformed_packets
    );
    ExitCode::SUCCESS
}
