//! End-to-end chain test: T2-MI → BBFrame → inner TS → SiDemux → typed table.
//!
//! This test proves the composed pipeline works by building a fully synthetic
//! fixture layer by layer:
//!
//! 1. A valid PAT section (built via `dvb_si::tables::Pat::serialize_into`).
//! 2. Wrapped in a 188-byte MPEG-TS packet (sync 0x47, PID 0x0000, PUSI,
//!    pointer_field 0, 0xFF padding).
//! 3. Wrapped in a Normal-Mode BBFrame: 10-byte BBHEADER (`NmTsIter`-compatible
//!    layout: UPL=1504 bits, SYNC=0x47, DFL = 188*8 bits, SYNCD=0) + 188-byte
//!    data field (CRC-8 of previous UP in byte 0; the prior UP is nominally
//!    all-zeros so CRC-8(all zeros over 188 bytes) replaces byte 0 of the UP).
//! 4. Wrapped in a T2-MI BBFrame payload (3-byte sub-header) with a correct
//!    T2-MI header and CRC-32 trailer.
//! 5. Wrapped in an outer 188-byte TS packet on PID 0x0006; fed through
//!    `T2miPump::new(0x0006)`.
//! 6. The resulting `AnyPayload::Bbframe` is unwrapped; its `bb.bbframe` field
//!    holds the raw BBHEADER + data field.
//! 7. `Bbheader::parse` + `up_iter` extracts the inner TS packets.
//! 8. Each inner TS packet is fed to `SiDemux`.
//! 9. We assert `AnyTable::Pat` arrives with the expected program entries.
//!
//! ## Hostility (T2-MI side)
//!
//! An additional `hostility_t2mi_garbage` test feeds 10 000 packets of seeded
//! LCG garbage and every truncation of a valid T2-MI packet through
//! `T2miPump::feed_ts` and `T2miPump::feed_raw` — no panics, counters move.

#![cfg(feature = "ts")]

use dvb_bbframe::header::{Bbheader, Matype, Mode, TsGs, BBHEADER_LEN};
use dvb_bbframe::packet::NM_UP_SIZE;
use dvb_common::crc32_mpeg2;
use dvb_si::demux::SiDemux;
use dvb_si::tables::AnyTable;
use dvb_t2mi::payload::AnyPayload;
use dvb_t2mi::pump::T2miPump;

// ── Helper constants ──────────────────────────────────────────────────────────

const TS_SYNC: u8 = 0x47;
const TS_PACKET_SIZE: usize = 188;
const PUSI_MASK: u8 = 0x40;
const PID_HI_MASK: u8 = 0x1F;
const PAYLOAD_FLAG: u8 = 0x10;

// ── Layer 1: Build a serialized PAT section ───────────────────────────────────

/// Build a PAT section for TSID 1, version 0, one program (program 1 → PID 0x0100).
/// Returns the full byte sequence including the CRC-32 trailer.
fn build_pat_section() -> Vec<u8> {
    use dvb_common::Serialize;
    use dvb_si::tables::pat::{Pat, PatEntry};

    let pat = Pat {
        transport_stream_id: 0x0001,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        entries: vec![PatEntry {
            program_number: 1,
            pid: 0x0100,
        }],
    };
    let mut buf = vec![0u8; pat.serialized_len()];
    pat.serialize_into(&mut buf).expect("PAT serialize");
    buf
}

// ── Layer 2: Wrap PAT section in an inner MPEG-TS packet ──────────────────────

/// Wrap `section` in a 188-byte MPEG-TS packet on `pid` with PUSI set,
/// pointer_field=0, 0xFF padding.
fn inner_ts_packet(pid: u16, section: &[u8]) -> [u8; TS_PACKET_SIZE] {
    assert!(
        section.len() <= TS_PACKET_SIZE - 5,
        "section too large to fit in one TS packet"
    );
    let mut pkt = [0xFFu8; TS_PACKET_SIZE];
    pkt[0] = TS_SYNC;
    pkt[1] = PUSI_MASK | (((pid >> 8) as u8) & PID_HI_MASK);
    pkt[2] = (pid & 0xFF) as u8;
    pkt[3] = PAYLOAD_FLAG;
    pkt[4] = 0x00; // pointer_field
    pkt[5..5 + section.len()].copy_from_slice(section);
    pkt
}

// ── Layer 3: Wrap inner TS packet in an NM BBFrame ────────────────────────────

/// Build a Normal-Mode BBFrame containing exactly one TS user packet (188 bytes).
///
/// NM layout:
/// - 10-byte BBHEADER: MATYPE-1=0xF0 (TS/SIS/CCM), MATYPE-2=0x00, UPL=1504 bits,
///   DFL=1504 bits (188 bytes), SYNC=0x47, SYNCD=0, CRC-8 correct.
/// - 188-byte data field: byte 0 is the CRC-8 of the *previous* user packet (we
///   treat the "previous" UP as all-zeros so we can compute its CRC-8 directly),
///   bytes 1–187 are the inner TS packet's bytes 1–187.
///
/// `NmTsIter::next` restores byte 0 to 0x47 (it simply overwrites it).
fn build_nm_bbframe(inner_ts: &[u8; TS_PACKET_SIZE]) -> Vec<u8> {
    use dvb_bbframe::crc::crc8;

    let upl_bits: u16 = 1504; // 188 * 8
    let dfl_bits: u16 = 1504; // one full UP
    let syncd_bits: u16 = 0;

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
        upl: upl_bits,
        sync: TS_SYNC,
        dfl: dfl_bits,
        syncd: syncd_bits,
        mode: Mode::Normal,
        issy_in_header: None,
    };
    let header_bytes = hdr.serialize();

    // Data field: NM places CRC-8 of the *preceding* UP at byte 0.
    // We designate the preceding UP as all-zero bytes, so:
    //   crc8_of_prev_up = crc8([0u8; 188])
    let prev_up_all_zeros = [0u8; TS_PACKET_SIZE];
    let crc_of_prev = crc8(&prev_up_all_zeros);

    let mut data_field = [0u8; TS_PACKET_SIZE];
    data_field[0] = crc_of_prev; // replaces the sync byte from the UP
    data_field[1..].copy_from_slice(&inner_ts[1..]);

    let mut frame = Vec::with_capacity(BBHEADER_LEN + TS_PACKET_SIZE);
    frame.extend_from_slice(&header_bytes);
    frame.extend_from_slice(&data_field);
    frame
}

// ── Layer 4: Wrap BBFrame in a T2-MI BBFrame payload and packet ───────────────

/// Build a T2-MI packet (type 0x00, BBFrame) carrying `bbframe_data`.
///
/// T2-MI BBFrame payload layout (ETSI TS 102 773 §5.2.1):
///   byte 0: frame_idx
///   byte 1: plp_id
///   byte 2 bit 7: intl_frame_start; bits 6..0: rfu=0
///   bytes 3..: raw BBFrame (BBHEADER + data field)
///
/// T2-MI header layout (6 bytes, §5.1):
///   byte 0: packet_type
///   byte 1: packet_count
///   byte 2: superframe_idx (4b) | rfu (1b) | t2mi_stream_id (3b)
///   byte 3: rfu
///   bytes 4-5: payload_len_bits (big-endian)
/// Followed by: payload bytes, then 4-byte CRC-32 (Annex A).
fn build_t2mi_packet(bbframe_data: &[u8]) -> Vec<u8> {
    // Sub-header: frame_idx=0, plp_id=5, intl_frame_start=1, rfu=0
    let mut payload = Vec::with_capacity(3 + bbframe_data.len());
    payload.push(0x00); // frame_idx
    payload.push(0x05); // plp_id
    payload.push(0x80); // intl_frame_start=1, rfu=0

    payload.extend_from_slice(bbframe_data);

    let payload_len_bits = (payload.len() * 8) as u16;
    let mut pkt = Vec::with_capacity(6 + payload.len() + 4);
    pkt.push(0x00); // packet_type = BBFrame
    pkt.push(0x01); // packet_count
    pkt.push(0x00); // superframe_idx=0, rfu=0, t2mi_stream_id=0
    pkt.push(0x00); // rfu
    pkt.extend_from_slice(&payload_len_bits.to_be_bytes());
    pkt.extend_from_slice(&payload);
    let crc = crc32_mpeg2::compute(&pkt);
    pkt.extend_from_slice(&crc.to_be_bytes());
    pkt
}

// ── Layer 5: Wrap T2-MI packet in outer TS packets (one or two) ──────────────

/// Maximum bytes of T2-MI payload per PUSI TS packet:
/// 188 - 4 (TS header) - 1 (pointer_field) = 183 bytes.
const MAX_T2MI_IN_PUSI: usize = TS_PACKET_SIZE - 4 - 1;

/// Maximum bytes of T2-MI payload per continuation TS packet:
/// 188 - 4 (TS header) = 184 bytes.
const MAX_T2MI_IN_CONT: usize = TS_PACKET_SIZE - 4;

/// Wrap `t2mi_data` in one or two 188-byte outer TS packets on `pid`.
///
/// The first packet is PUSI with pointer_field=0; if the data overflows one
/// packet a second continuation packet (PUSI=0) carries the remainder.  The
/// combined payload capacity (183 + 184 = 367 bytes) is sufficient for the
/// synthetic chain fixture (≈211 bytes).
fn outer_ts_packets(pid: u16, t2mi_data: &[u8]) -> Vec<[u8; TS_PACKET_SIZE]> {
    assert!(
        t2mi_data.len() <= MAX_T2MI_IN_PUSI + MAX_T2MI_IN_CONT,
        "T2-MI data too large ({} bytes) to fit in two outer TS packets",
        t2mi_data.len()
    );

    let mut result = Vec::new();

    // First packet: PUSI, pointer_field=0
    {
        let chunk = &t2mi_data[..t2mi_data.len().min(MAX_T2MI_IN_PUSI)];
        let mut pkt = [0xFFu8; TS_PACKET_SIZE];
        pkt[0] = TS_SYNC;
        pkt[1] = PUSI_MASK | (((pid >> 8) as u8) & PID_HI_MASK);
        pkt[2] = (pid & 0xFF) as u8;
        pkt[3] = PAYLOAD_FLAG;
        pkt[4] = 0x00; // pointer_field = 0: T2-MI packet starts immediately
        pkt[5..5 + chunk.len()].copy_from_slice(chunk);
        result.push(pkt);
    }

    // Second packet: continuation (if needed)
    if t2mi_data.len() > MAX_T2MI_IN_PUSI {
        let remainder = &t2mi_data[MAX_T2MI_IN_PUSI..];
        let mut pkt = [0xFFu8; TS_PACKET_SIZE];
        pkt[0] = TS_SYNC;
        pkt[1] = ((pid >> 8) as u8) & PID_HI_MASK; // PUSI=0
        pkt[2] = (pid & 0xFF) as u8;
        pkt[3] = PAYLOAD_FLAG;
        // No pointer_field for continuation packets.
        pkt[4..4 + remainder.len()].copy_from_slice(remainder);
        result.push(pkt);
    }

    result
}

// ── Main chain test ───────────────────────────────────────────────────────────

/// Full pipeline: outer TS → T2miPump → AnyPayload::Bbframe → Bbheader + up_iter
/// → inner TS → SiDemux → AnyTable::Pat with expected entries.
#[test]
fn chain_t2mi_bbframe_si_pat() {
    // Layer 1: PAT section
    let pat_section = build_pat_section();

    // Layer 2: inner TS packet (PID 0x0000, PUSI)
    let inner_ts = inner_ts_packet(0x0000, &pat_section);

    // Layer 3: NM BBFrame
    let bbframe = build_nm_bbframe(&inner_ts);

    // Layer 4: T2-MI BBFrame packet
    let t2mi_pkt = build_t2mi_packet(&bbframe);

    // Layer 5: outer TS packet(s) on PID 0x0006.
    // The fixture's T2-MI packet is ~211 bytes (6 header + 3 sub-hdr + 10 BBHEADER
    // + 188 data + 4 CRC) — larger than the 183-byte PUSI payload capacity of one
    // TS packet, so we emit two packets (PUSI + continuation).
    let outer_ts_pkts = outer_ts_packets(0x0006, &t2mi_pkt);

    // ── Step A: T2miPump → T2miEvent ─────────────────────────────────────────
    let mut pump = T2miPump::new(0x0006);
    let mut all_events: Vec<_> = Vec::new();
    for pkt in &outer_ts_pkts {
        all_events.extend(pump.feed_ts(pkt));
    }
    let events = all_events;
    assert_eq!(events.len(), 1, "expected exactly one T2-MI event");

    // ── Step B: T2miEvent → AnyPayload::Bbframe ──────────────────────────────
    let payload = events[0].payload().expect("payload parse");
    let bb = match payload {
        AnyPayload::Bbframe(ref bb) => bb,
        other => panic!("expected Bbframe, got {other:?}"),
    };
    assert_eq!(bb.plp_id, 5, "plp_id");
    assert!(bb.intl_frame_start, "intl_frame_start");

    // ── Step C: Parse BBHEADER from bb.bbframe ────────────────────────────────
    let bbheader = Bbheader::parse(bb.bbframe).expect("Bbheader::parse on inner BBFrame bytes");
    assert_eq!(bbheader.mode, Mode::Normal, "mode");
    assert_eq!(bbheader.dfl, 1504, "DFL bits");
    assert_eq!(bbheader.syncd, 0, "SYNCD bits");

    // ── Step D: up_iter → inner TS packets ───────────────────────────────────
    let dfl_bytes = (bbheader.dfl / 8) as usize;
    let data_field_start = BBHEADER_LEN;
    let data_field = &bb.bbframe[data_field_start..data_field_start + dfl_bytes];
    let inner_pkts: Vec<[u8; NM_UP_SIZE]> =
        dvb_bbframe::packet::up_iter(data_field, &bbheader).collect();
    assert_eq!(inner_pkts.len(), 1, "expected one inner TS packet");
    assert_eq!(inner_pkts[0][0], TS_SYNC, "inner TS sync byte restored");

    // ── Step E: SiDemux → AnyTable::Pat ─────────────────────────────────────
    let mut demux = SiDemux::builder().build();
    let events: Vec<_> = demux.feed(&inner_pkts[0]).collect();
    assert_eq!(events.len(), 1, "SiDemux must emit one section event");

    let table = events[0].table().expect("table dispatch");
    match table {
        AnyTable::Pat(pat) => {
            assert_eq!(pat.transport_stream_id, 0x0001, "TSID");
            assert_eq!(pat.entries.len(), 1, "one program entry");
            assert_eq!(pat.entries[0].program_number, 1, "program_number");
            assert_eq!(pat.entries[0].pid, 0x0100, "PMT PID");
        }
        other => panic!("expected AnyTable::Pat, got {other:?}"),
    }
}

// ── Hostility: T2-MI garbage and truncation ───────────────────────────────────

/// Simple deterministic LCG (no external deps).
/// Deliberately duplicated (chain.rs <-> hostility.rs): no shared test-helper crate; keep constants in sync.
struct Lcg(u64);
impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u8(&mut self) -> u8 {
        // Numerical Recipes LCG (m=2^64)
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u8
    }
}

/// Feed 10 000 × 188-byte seeded-LCG garbage into T2miPump::feed_ts — no panic.
#[test]
fn hostility_t2mi_garbage_feed_ts() {
    let mut pump = T2miPump::new(0x0006);
    let mut lcg = Lcg::new(0xDEAD_BEEF_CAFE_BABEu64);

    for _ in 0..10_000u32 {
        let mut pkt = [0u8; TS_PACKET_SIZE];
        for b in &mut pkt {
            *b = lcg.next_u8();
        }
        let _: Vec<_> = pump.feed_ts(&pkt).collect();
    }

    let s = pump.stats();
    assert_eq!(s.ts_packets, 10_000);
    assert!(
        s.malformed_packets > 9_000,
        "garbage must be counted malformed (got {})",
        s.malformed_packets
    );
}

/// Feed 10 000 × random-length seeded-LCG garbage into T2miPump::feed_raw — no panic.
#[test]
fn hostility_t2mi_garbage_feed_raw() {
    let mut pump = T2miPump::raw();
    let mut lcg = Lcg::new(0xCAFE_F00D_1234_5678u64);

    for _ in 0..10_000u32 {
        let len = 1 + (lcg.next_u8() as usize % 256);
        let data: Vec<u8> = (0..len).map(|_| lcg.next_u8()).collect();
        let _: Vec<_> = pump.feed_raw(&data).collect();
    }
    let s = pump.stats();
    // garbage feed_raw is buffered; occasional decoding as plausible packet headers
    // generates CRC failures but no outright malformed packets (no sync errors when
    // raw bytes happen to decode as valid header + payload). These events still move
    // counters.
    assert!(
        s.t2mi_packets + s.crc_failures + s.malformed_packets > 0,
        "garbage feed_raw must move some counter; got stats: {s:?}"
    );
}

/// Every truncation of a valid T2-MI packet through feed_raw — no panic.
#[test]
fn hostility_t2mi_truncation_feed_raw() {
    let pat_section = build_pat_section();
    let inner_ts = inner_ts_packet(0x0000, &pat_section);
    let bbframe = build_nm_bbframe(&inner_ts);
    let t2mi_pkt = build_t2mi_packet(&bbframe);

    // Every prefix 0..len must not panic (feed_raw has no TS framing).
    for len in 0..=t2mi_pkt.len() {
        let mut pump = T2miPump::raw();
        let _: Vec<_> = pump.feed_raw(&t2mi_pkt[..len]).collect();
    }
}
