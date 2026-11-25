//! Scramble a TS capture's payloads packet by packet.
//!
//! Reads 188-byte TS packets from the input, scrambles each packet's payload
//! with the given control word, and writes them to the output as it goes, so a
//! multi-gigabyte capture is never held whole.
//!
//! ```sh
//! cargo run -p dvb-csa --example scramble_file -- capture.ts scrambled.ts 0102030405060708
//! ```

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::{env, process};

use dvb_csa::ControlWord;
use dvb_csa::ts::{KeyParity, scramble_ts_packet};

/// Length of one MPEG-2 TS packet (ISO/IEC 13818-1 §2.4.3.2).
const TS_PACKET_LEN: usize = 188;

/// Read/write granularity for the stream.
const CHUNK_LEN: usize = 1 << 20;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 4 {
        eprintln!("Usage: scramble_file <input.bin> <output.bin> <cw-hex>");
        eprintln!("  cw-hex: 16 hex chars, e.g. 0102030405060708");
        process::exit(1);
    }

    let cw = ControlWord::from_bytes(parse_cw(&args[3]));
    let input = File::open(&args[1]).unwrap_or_else(|e| {
        eprintln!("read {}: {e}", args[1]);
        process::exit(1);
    });
    let output = File::create(&args[2]).unwrap_or_else(|e| {
        eprintln!("write {}: {e}", args[2]);
        process::exit(1);
    });
    let mut reader = BufReader::with_capacity(CHUNK_LEN, input);
    let mut writer = BufWriter::with_capacity(CHUNK_LEN, output);

    let written = transform(&mut reader, &mut writer, |packet| {
        let mut out = [0u8; TS_PACKET_LEN];
        out.copy_from_slice(packet);
        // A packet with no payload (`adaptation_field_control` 00/10) has
        // nothing to scramble and is copied through unchanged.
        let _ = scramble_ts_packet(&cw, KeyParity::Even, &mut out);
        out
    })
    .unwrap_or_else(|e| {
        eprintln!("scramble: {e}");
        process::exit(1);
    });
    writer.flush().unwrap_or_else(|e| {
        eprintln!("write {}: {e}", args[2]);
        process::exit(1);
    });
    eprintln!("Scrambled {written} bytes → {}", args[2]);
}

/// Read the input in 188-byte packets, applying `on_packet` to each whole one
/// and copying a trailing partial packet through unchanged.
fn transform(
    reader: &mut impl Read,
    writer: &mut impl Write,
    mut on_packet: impl FnMut(&[u8]) -> [u8; TS_PACKET_LEN],
) -> io::Result<usize> {
    let mut written = 0usize;
    let mut packet = [0u8; TS_PACKET_LEN];
    loop {
        let filled = read_up_to(reader, &mut packet)?;
        if filled == 0 {
            return Ok(written);
        }
        if filled == TS_PACKET_LEN {
            writer.write_all(&on_packet(&packet))?;
        } else {
            writer.write_all(&packet[..filled])?;
        }
        written += filled;
        if filled < TS_PACKET_LEN {
            return Ok(written);
        }
    }
}

/// Fill `buf` as far as the stream allows; returns how many bytes arrived.
fn read_up_to(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0usize;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

fn parse_cw(hex: &str) -> [u8; 8] {
    assert!(hex.len() == 16, "CW must be 16 hex chars (8 bytes)");
    let mut cw = [0u8; 8];
    for i in 0..8 {
        cw[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("invalid hex in CW");
    }
    cw
}
