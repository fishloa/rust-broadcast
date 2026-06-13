//! Criterion benchmarks for dvb-bbframe hot paths.
//!
//! Covers:
//! - `BbframePump::feed` — throughput on the TNT and RAI real-capture fixtures
//! - `up_iter` / `NmTsIter` — iterator throughput on a synthetic NM data field
//!
//! The TNT fixture carries NM BBFrames (PID 0x010E); the RAI fixture also has
//! NM BBFrames (PID TBD — extracted by the fixture helper below).

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use dvb_bbframe::crc::crc8;
use dvb_bbframe::header::{Bbheader, Matype, Mode, TsGs, BBHEADER_LEN};
use dvb_bbframe::packet::{up_iter, NmTsIter, NM_UP_SIZE};
use dvb_bbframe::pump::BbframePump;

// ── Fixtures ─────────────────────────────────────────────────────────────────

const TNT_FIXTURE: &[u8] = include_bytes!("../tests/fixtures/tnt-5w-12732v-bbframe.ts");

/// PID carrying BBFrames in the TNT fixture.
const TNT_PID: u16 = 0x010E;

const TS_PACKET_SIZE: usize = 188;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Extract raw BBFrame byte blobs from a TS capture on a known PID.
///
/// The encoding used in these fixtures stores BBFrame data as private sections
/// (table_id 0x80 or similar); we reuse the same extraction logic from the
/// integration tests: a count byte 0xB8 marks the start of a new BBFrame.
fn extract_bbframe_data(data: &[u8], pid: u16) -> Vec<Vec<u8>> {
    let mut frames: Vec<Vec<u8>> = Vec::new();
    let mut current: Vec<u8> = Vec::with_capacity(8192);
    let mut started = false;
    let mut pos = 0;

    while pos + TS_PACKET_SIZE <= data.len() {
        let pkt = &data[pos..pos + TS_PACKET_SIZE];
        pos += TS_PACKET_SIZE;

        if pkt[0] != 0x47 {
            continue;
        }
        let ts_pid = ((pkt[1] as u16 & 0x1F) << 8) | pkt[2] as u16;
        if ts_pid != pid {
            continue;
        }
        if pkt[3] & 0x30 != 0x10 {
            continue;
        }
        if pkt[4] != 0x00 || pkt[5] != 0x80 || pkt[6] != 0x00 {
            continue;
        }
        let slen = pkt[7] as usize;
        if slen == 0 || slen > 0xB4 {
            continue;
        }
        let count = pkt[8];
        let data_len = slen - 1;
        let data_end = 9 + data_len;
        if data_end > TS_PACKET_SIZE {
            continue;
        }
        let section_data = &pkt[9..data_end];

        if count == 0xB8 {
            if started && !current.is_empty() {
                frames.push(std::mem::take(&mut current));
            }
            started = true;
            current.clear();
        }
        if started {
            current.extend_from_slice(section_data);
        }
    }
    frames
}

/// Build a syntactically valid Normal-Mode BBHEADER (10 bytes) with the given DFL
/// and SYNCD=0 (first UP starts at byte 0 of the data field).
fn nm_bbheader(dfl_bits: u16) -> [u8; BBHEADER_LEN] {
    let hdr = Bbheader {
        matype: Matype {
            ts_gs: TsGs::Ts,
            sis: true,
            ccm: true,
            issyi: false,
            npd: false,
            ext: 0,
            isi: 0,
        },
        upl: NM_UP_SIZE as u16 * 8,
        sync: 0x47,
        dfl: dfl_bits,
        syncd: 0,
        mode: Mode::Normal,
        issy_in_header: None,
    };
    hdr.serialize()
}

// ── Benchmarks ────────────────────────────────────────────────────────────────

/// `BbframePump::feed` throughput over the TNT real-capture fixture.
fn bench_bbframe_pump_feed(c: &mut Criterion) {
    let frames = extract_bbframe_data(TNT_FIXTURE, TNT_PID);
    if frames.is_empty() {
        // No frames extracted — benches still link, just produce no timing data.
        return;
    }

    // Total bytes across all frames fed to the pump.
    let total_bytes: u64 = frames.iter().map(|f| f.len() as u64).sum();

    let mut group = c.benchmark_group("bbframe_pump_feed");
    group.throughput(Throughput::Bytes(total_bytes));

    group.bench_function("tnt-nm-frames", |b| {
        b.iter(|| {
            let mut pump = BbframePump::new();
            let mut pkts = 0u64;
            for frame in black_box(&frames) {
                for _ in pump.feed(0, frame) {
                    pkts += 1;
                }
            }
            black_box(pkts)
        });
    });

    group.finish();
}

/// `NmTsIter` (the inner `up_iter` NM path) over a synthetic data field.
///
/// Builds a 10-frame synthetic corpus: each frame holds 8 NM user packets
/// (8 × 188 = 1504 bytes of data field) so the iterator does real work.
fn bench_up_iter_nm(c: &mut Criterion) {
    // 8 NM UPs per frame: each UP is 188 bytes with a dummy CRC-8 at byte 0.
    const UPS_PER_FRAME: usize = 8;
    const DATA_FIELD_BYTES: usize = UPS_PER_FRAME * NM_UP_SIZE;
    const DFL_BITS: u16 = (DATA_FIELD_BYTES * 8) as u16;

    let bbheader = nm_bbheader(DFL_BITS);

    // Build the data field: each UP slot's byte 0 is crc8([0u8; NM_UP_SIZE]),
    // bytes 1..188 are a count pattern.
    let mut data_field = vec![0u8; DATA_FIELD_BYTES];
    for i in 0..UPS_PER_FRAME {
        let base = i * NM_UP_SIZE;
        for j in 1..NM_UP_SIZE {
            data_field[base + j] = j as u8;
        }
        // byte 0 = CRC-8 (placeholder; up_iter replaces it with 0x47 anyway)
        data_field[base] = crc8(&data_field[base..base + NM_UP_SIZE]);
    }

    let hdr = Bbheader::parse(&bbheader).expect("synthetic header parses");
    let total_bytes = (DATA_FIELD_BYTES * 10) as u64;

    let mut group = c.benchmark_group("up_iter_nm");
    group.throughput(Throughput::Bytes(total_bytes));

    group.bench_function("8_ups_x10_frames", |b| {
        b.iter(|| {
            let mut count = 0u64;
            for _ in 0..10 {
                for pkt in up_iter(black_box(&data_field), black_box(&hdr)) {
                    let _ = black_box(pkt);
                    count += 1;
                }
            }
            black_box(count)
        });
    });

    // Also bench the raw NmTsIter directly (no BBHEADER overhead).
    group.bench_function("NmTsIter_direct", |b| {
        b.iter(|| {
            let mut count = 0u64;
            for _ in 0..10 {
                for pkt in NmTsIter::new(black_box(&data_field)) {
                    let _ = black_box(pkt);
                    count += 1;
                }
            }
            black_box(count)
        });
    });

    group.finish();
}

criterion_group!(benches, bench_bbframe_pump_feed, bench_up_iter_nm);
criterion_main!(benches);
