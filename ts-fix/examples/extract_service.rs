//! Extract a single programme from a transport stream via `PidFilter::service`.
//!
//! Builds a tiny synthetic multi-program stream in memory, extracts
//! programme 1 using `filter_pids(PidFilter::service(1))`, and prints the
//! set of PIDs that survived the filter.
//!
//! # Usage
//!
//! ```bash
//! cargo run --example extract_service
//! ```

use std::collections::BTreeSet;

use ts_fix::{PidFilter, TsFix};

fn main() {
    // Build a minimal multi-program stream:
    //   PAT (PID 0x0000) → program 1 PMT PID 0x100, program 2 PMT PID 0x200
    //   PMT for program 1 (PID 0x100) → PCR PID 0x101, ES PID 0x102
    //   PMT for program 2 (PID 0x200) → PCR PID 0x201, ES PID 0x202
    let stream = build_multiprogram_stream();
    let total_packets = stream.len() / 188;
    println!("input: {total_packets} packets (synthetic multi-program stream)");

    // Extract program 1.
    let mut engine = TsFix::builder()
        .filter_pids(PidFilter::service(1))
        .build()
        .expect("build should not fail");

    let mut output = Vec::new();
    for chunk in stream.chunks(188) {
        engine
            .push(chunk, |pkt| output.extend_from_slice(pkt))
            .expect("valid 188-byte packet");
    }
    engine.finish(|pkt| output.extend_from_slice(pkt));

    // Collect surviving PIDs.
    let mut kept_pids: BTreeSet<u16> = BTreeSet::new();
    for chunk in output.chunks(188) {
        if chunk.len() >= 3 {
            let pid = (((chunk[1] & 0x1F) as u16) << 8) | chunk[2] as u16;
            kept_pids.insert(pid);
        }
    }

    println!("kept {} packets", output.len() / 188);
    println!("kept PIDs: {:#06x?}", kept_pids.iter().collect::<Vec<_>>());

    // Expected: PAT 0x0000, PMT 0x100, PCR 0x101, ES 0x102.
    let expected: BTreeSet<u16> = [0x0000, 0x0100, 0x0101, 0x0102].into();
    if kept_pids == expected {
        println!("result: PASS");
    } else {
        println!("result: FAIL (expected {expected:#06x?})");
    }
}

// ── Synthetic stream builder ──────────────────────────────────────────────────

/// Build a minimal multi-program TS with PAT + two PMTs + ES payload packets.
fn build_multiprogram_stream() -> Vec<u8> {
    let mut out = Vec::new();

    // PAT section (table_id 0x00, PID 0x0000) listing both programs.
    let pat = pat_section(&[(1, 0x0100), (2, 0x0200)]);
    out.extend_from_slice(&section_packet(0x0000, &pat));

    // PMT for program 1 (PID 0x0100): PCR PID 0x0101, ES 0x0102.
    let pmt1 = pmt_section(1, 0x0101, &[(0x1B, 0x0102)]);
    out.extend_from_slice(&section_packet(0x0100, &pmt1));

    // PMT for program 2 (PID 0x0200): PCR PID 0x0201, ES 0x0202.
    let pmt2 = pmt_section(2, 0x0201, &[(0x1B, 0x0202)]);
    out.extend_from_slice(&section_packet(0x0200, &pmt2));

    // ES payload packets for program 1 (kept after PMT resolves).
    for cc in 0..5 {
        out.extend_from_slice(&payload_packet(0x0102, cc));
    }
    // PCR packet for program 1.
    out.extend_from_slice(&payload_packet(0x0101, 0));
    // ES payload packets for program 2 (dropped).
    for cc in 0..3 {
        out.extend_from_slice(&payload_packet(0x0202, cc));
    }

    out
}

/// Build a complete PAT section (table_id 0x00).
fn pat_section(entries: &[(u16, u16)]) -> Vec<u8> {
    let mut body = Vec::new();

    // transport_stream_id
    body.extend_from_slice(&0x0001u16.to_be_bytes());
    // version(5) + current_next(1)
    body.push(0xC1); // version 0, current
    body.push(0x00); // section_number
    body.push(0x00); // last_section_number
                     // entries
    for &(prog, pid) in entries {
        body.extend_from_slice(&prog.to_be_bytes());
        body.extend_from_slice(&(pid | 0xE000).to_be_bytes());
    }

    // section_length = len(body) + CRC_LEN(4), as per PMT serializer at line 615-620.
    let section_length = body.len() as u16 + 4;

    let mut s = vec![0x00];
    s.push(0xB0 | ((section_length >> 8) & 0x0F) as u8);
    s.push((section_length & 0xFF) as u8);
    s.extend_from_slice(&body);

    let crc = calc_crc32_mpeg2(&s);
    s.extend_from_slice(&crc.to_be_bytes());

    s
}

/// Build a complete PMT section (table_id 0x02).
fn pmt_section(program_number: u16, pcr_pid: u16, es_list: &[(u8, u16)]) -> Vec<u8> {
    let mut body = Vec::new();

    // table_id_extension = program_number
    body.extend_from_slice(&program_number.to_be_bytes());
    // version(5) + current_next(1)
    body.push(0xC1); // version 0, current
    body.push(0x00); // section_number
    body.push(0x00); // last_section_number
                     // PCR PID
    body.push(0xE0 | ((pcr_pid >> 8) & 0x0F) as u8);
    body.push((pcr_pid & 0xFF) as u8);
    // program_info_length = 0
    body.push(0xF0);
    body.push(0x00);
    // ES entries (5 bytes each: stream_type + pid(2) + es_info_len(2))
    for &(stype, pid) in es_list {
        body.push(stype);
        body.push(0xE0 | ((pid >> 8) & 0x0F) as u8);
        body.push((pid & 0xFF) as u8);
        body.push(0xF0);
        body.push(0x00);
    }

    // section_length = len(body) + CRC_LEN(4)
    let section_length = body.len() as u16 + 4;

    let mut s = vec![0x02];
    s.push(0xB0 | ((section_length >> 8) & 0x0F) as u8);
    s.push((section_length & 0xFF) as u8);
    s.extend_from_slice(&body);

    let crc = calc_crc32_mpeg2(&s);
    s.extend_from_slice(&crc.to_be_bytes());

    s
}

/// Wrap a fully-formed PSI section into a 188-byte TS packet on `pid`.
fn section_packet(pid: u16, section: &[u8]) -> [u8; 188] {
    let mut pkt = [0xFFu8; 188];
    pkt[0] = 0x47;
    let pid_hi = ((pid >> 8) & 0x1F) as u8;
    pkt[1] = pid_hi | 0x40; // PUSI = 1
    pkt[2] = (pid & 0xFF) as u8;
    pkt[3] = 0x10; // payload only, CC = 0
    pkt[4] = 0x00; // pointer_field = 0
    let copy_len = section.len().min(183);
    pkt[5..5 + copy_len].copy_from_slice(&section[..copy_len]);
    pkt
}

/// Build a non-PSI payload-bearing packet.
fn payload_packet(pid: u16, cc: u8) -> [u8; 188] {
    let mut pkt = [0xFFu8; 188];
    pkt[0] = 0x47;
    let pid_hi = ((pid >> 8) & 0x1F) as u8;
    pkt[1] = pid_hi;
    pkt[2] = (pid & 0xFF) as u8;
    pkt[3] = 0x10 | (cc & 0x0F);
    for i in 0..180 {
        pkt[4 + i] = (i & 0xFF) as u8;
    }
    pkt
}

/// MPEG-2 CRC32 (polynomial 0x04C11DB7, init 0xFFFFFFFF, final XOR 0x00000000).
fn calc_crc32_mpeg2(data: &[u8]) -> u32 {
    let poly: u32 = 0x04C11DB7;
    let mut crc: u32 = 0xFFFFFFFF;
    for &byte in data {
        crc ^= (byte as u32) << 24;
        for _ in 0..8 {
            if crc & 0x80000000 != 0 {
                crc = (crc << 1) ^ poly;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}
