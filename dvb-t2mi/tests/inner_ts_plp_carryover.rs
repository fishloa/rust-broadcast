//! Multi-PLP carry-over isolation for `InnerTsRecovery` (issue #1034).
//!
//! `dvb_bbframe::pump::BbframePump` keys its `CarryOverExtractor` per PLP
//! (`extractors: [Option<CarryOverExtractor>; MAX_PLPS]`) precisely because a
//! DVB-T2 T2-MI stream time-multiplexes independent PLPs, each with its own
//! SYNCD/carry-over chain (EN 302 755 §5.1.7: "ISI ... has the same meaning
//! as PLP_ID"). `InnerTsRecovery` (used by `dvb-tools t2mi --inner` without
//! `--plp`) instead ran every PLP through a single shared extractor, so a
//! user packet split across BBFrame boundaries (needed whenever a UP doesn't
//! align with a frame) gets corrupted the moment a *different* PLP's frame is
//! interleaved in between the split halves.
//!
//! No committed real capture exercises this: the only real T2-MI fixture
//! (`tests/fixtures/colombia-capital-t2mi.ts`) carries a single PLP (102), and
//! no local tool (TSDuck/ffmpeg/GPAC) generates a multi-PLP T2-MI stream. Per
//! the W4 fixture-first fallback, this test instead builds the T2-MI/BBFrame
//! framing directly from EN 302 755 §5.1.8 / TS 102 773's own byte layout,
//! using two distinguishable **real** TS packets as the two PLPs' payload
//! (`fixtures/ts/scte35-real.ts` and the first packet of
//! `fixtures/mpeg-ts/af-pcr-stuffing.ts`), split so each PLP's single UP
//! spans two BBFrames — the exact shape that needs carry-over state.

#![cfg(feature = "ts")]

use std::fs;

use broadcast_common::crc32_mpeg2;
use dvb_bbframe::crc::crc8;
use dvb_bbframe::header::{Bbheader, Matype, Mode, TsGs};
use dvb_t2mi::inner_ts::InnerTsRecovery;

const TS_LEN: usize = 188;
/// EN 302 755 §5.1.8 figure 5, ISSYI=0/NPD=0: CRC-8(1) + UP-minus-sync(187).
const STRIDE: usize = TS_LEN;
/// Split point inside the 188-byte transmitted UP: first frame carries this
/// many bytes, the second frame carries the remaining `STRIDE - SPLIT`.
const SPLIT: usize = 100;

fn real_packet(rel_path: &str, offset: usize) -> [u8; TS_LEN] {
    let path = format!(concat!(env!("CARGO_MANIFEST_DIR"), "/../{}"), rel_path);
    let bytes = fs::read(&path).unwrap_or_else(|e| panic!("{rel_path} must be present: {e}"));
    let pkt = &bytes[offset..offset + TS_LEN];
    assert_eq!(
        pkt[0], 0x47,
        "{rel_path}[{offset}] must be a real TS packet"
    );
    pkt.try_into().unwrap()
}

/// Build the 188-byte transmitted-UP wire chunk for one PLP's single UP:
/// `CRC-8(garbage — no predecessor in this 2-frame-only stream) + UP[1..188]`.
fn up_wire_chunk(inner: &[u8; TS_LEN]) -> [u8; STRIDE] {
    let mut chunk = [0u8; STRIDE];
    chunk[0] = crc8(&[0u8; TS_LEN - 1]); // arbitrary leading byte; unchecked (no chain yet)
    chunk[1..].copy_from_slice(&inner[1..]);
    chunk
}

/// One Normal-Mode BBHEADER (ISSYI=0, NPD=0) for a `dfl_bytes`-long data
/// field starting `syncd_bytes` into it.
fn nm_header(dfl_bytes: usize, syncd_bytes: usize) -> Bbheader {
    Bbheader {
        matype: Matype {
            ts_gs: TsGs::Ts,
            sis: true,
            ccm: true,
            issyi: false,
            npd: false,
            ext: 0,
            isi: 0,
        },
        upl: (STRIDE * 8) as u16,
        sync: 0x47,
        dfl: (dfl_bytes * 8) as u16,
        syncd: (syncd_bytes * 8) as u16,
        mode: Mode::Normal,
        issy_in_header: None,
    }
}

/// Wrap a BBFrame (header + data field) in a T2-MI BBFrame packet (type
/// 0x00) for the given PLP id, with a valid CRC-32 (TS 102 773).
fn t2mi_packet_for_plp(bbframe: &[u8], plp_id: u8) -> Vec<u8> {
    let mut payload = vec![0x00, plp_id, 0x80]; // frame_idx=0, plp_id, intl_frame_start
    payload.extend_from_slice(bbframe);
    let mut pkt = vec![0x00u8, 0x01, 0x00, 0x00];
    pkt.extend_from_slice(&((payload.len() * 8) as u16).to_be_bytes());
    pkt.extend_from_slice(&payload);
    let crc = crc32_mpeg2::compute(&pkt);
    pkt.extend_from_slice(&crc.to_be_bytes());
    pkt
}

/// Wrap T2-MI data in outer 188-byte TS packets on `pid`.
fn outer_ts(pid: u16, data: &[u8]) -> Vec<[u8; TS_LEN]> {
    let mut out = Vec::new();
    let first_cap = TS_LEN - 5;
    let cont_cap = TS_LEN - 4;
    let mut off = 0;
    let mut first = true;
    while off < data.len() {
        let mut pkt = [0xFFu8; TS_LEN];
        pkt[0] = 0x47;
        let cap = if first { first_cap } else { cont_cap };
        pkt[1] = (if first { 0x40 } else { 0x00 }) | (((pid >> 8) as u8) & 0x1F);
        pkt[2] = (pid & 0xFF) as u8;
        pkt[3] = 0x10;
        let hdr_len = if first {
            pkt[4] = 0x00;
            5
        } else {
            4
        };
        let n = (data.len() - off).min(cap);
        pkt[hdr_len..hdr_len + n].copy_from_slice(&data[off..off + n]);
        out.push(pkt);
        off += n;
        first = false;
    }
    out
}

#[test]
fn two_plps_interleaved_across_a_split_up_both_recover_intact() {
    let pid = 0x1000;

    let inner_a = real_packet("fixtures/ts/scte35-real.ts", 0);
    let inner_b = real_packet("fixtures/mpeg-ts/af-pcr-stuffing.ts", 0);
    assert_ne!(
        &inner_a[..],
        &inner_b[..],
        "the two PLPs carry distinct real packets"
    );

    let chunk_a = up_wire_chunk(&inner_a);
    let chunk_b = up_wire_chunk(&inner_b);

    // Each PLP's single UP is split: frame 1 carries [0..SPLIT), frame 2
    // carries [SPLIT..STRIDE) with SYNCD = SPLIT bytes (per EN 302 755
    // Table 2, SYNCD is only meaningful relative to a stream's OWN prior
    // partial — that is exactly what per-PLP state must preserve).
    let bb_a1 = {
        let mut f = nm_header(SPLIT, 0).serialize().to_vec();
        f.extend_from_slice(&chunk_a[..SPLIT]);
        f
    };
    let bb_a2 = {
        let mut f = nm_header(STRIDE - SPLIT, STRIDE - SPLIT)
            .serialize()
            .to_vec();
        f.extend_from_slice(&chunk_a[SPLIT..]);
        f
    };
    let bb_b1 = {
        let mut f = nm_header(SPLIT, 0).serialize().to_vec();
        f.extend_from_slice(&chunk_b[..SPLIT]);
        f
    };
    let bb_b2 = {
        let mut f = nm_header(STRIDE - SPLIT, STRIDE - SPLIT)
            .serialize()
            .to_vec();
        f.extend_from_slice(&chunk_b[SPLIT..]);
        f
    };

    // Interleave: PLP0's first half, PLP1's first half, PLP0's second half,
    // PLP1's second half — a different PLP's frame always falls *between*
    // a split UP's two halves.
    let mut t2mi_stream = Vec::new();
    t2mi_stream.extend_from_slice(&t2mi_packet_for_plp(&bb_a1, 0));
    t2mi_stream.extend_from_slice(&t2mi_packet_for_plp(&bb_b1, 1));
    t2mi_stream.extend_from_slice(&t2mi_packet_for_plp(&bb_a2, 0));
    t2mi_stream.extend_from_slice(&t2mi_packet_for_plp(&bb_b2, 1));

    let outer = outer_ts(pid, &t2mi_stream);

    let mut rec = InnerTsRecovery::new(pid); // no --plp filter: the buggy shared path
    let mut recovered: Vec<[u8; TS_LEN]> = Vec::new();
    for pkt in &outer {
        recovered.extend_from_slice(rec.feed(pkt));
    }

    assert_eq!(
        recovered.len(),
        2,
        "both PLPs' single UP must be recovered, not merged/lost/duplicated"
    );
    assert!(
        recovered.contains(&inner_a),
        "PLP 0's real packet must come back byte-identical"
    );
    assert!(
        recovered.contains(&inner_b),
        "PLP 1's real packet must come back byte-identical"
    );
}
