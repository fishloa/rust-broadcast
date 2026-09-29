//! Synthetic NM BBFrame with ISSYI=1 *and* NPD=1 simultaneously active
//! (issue #1033).
//!
//! Neither committed real capture (`fixtures/dvb-bbframe/{tnt,rai}-*.ts`,
//! exercised in `tests/integration_fixture.rs`) has NPD active — both are
//! confirmed NPD=0 there. No local independent tool (TSDuck/ffmpeg/GPAC)
//! generates a raw DVB-S2/T2 BBFrame, let alone one with ISSY+DNP, so per the
//! W4 fixture-first rule this test instead builds the frame directly from EN
//! 302 755 §5.1.8's own byte-layout table (figure 5, "Normal Mode, GFPS and
//! TS"): each transmitted UP is `CRC-8 (1) + original UP minus sync (187) +
//! ISSY (2, short form) + DNP (1)` = 191-byte stride. The UP *payload* itself
//! is not fabricated: it is the real, committed SCTE-35 canonical
//! `splice_insert` TS packet (`fixtures/ts/scte35-real.ts`, added in #421 from
//! the public SCTE-35 corpus), so the assertions are checking real packet
//! bytes survive extraction, not a pattern our own code invented.

use std::fs;

use dvb_bbframe::crc::crc8;
use dvb_bbframe::header::{Bbheader, Matype, Mode, TsGs};
use dvb_bbframe::packet::{CarryOverExtractor, NM_UP_SIZE, TS_SYNC_BYTE, nm_stride_bytes};

/// EN 302 755 §5.1.8 figure 5 stride for ISSYI=1 (short form, 2 bytes) and
/// NPD=1 (1 byte): CRC-8(1) + UP-minus-sync(187) + ISSY(2) + DNP(1).
const STRIDE: usize = NM_UP_SIZE + 2 + 1;

fn real_up_payload() -> [u8; NM_UP_SIZE] {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/scte35-real.ts");
    let bytes = fs::read(path).expect("fixtures/ts/scte35-real.ts must be present");
    assert_eq!(bytes.len(), NM_UP_SIZE, "fixture is exactly one TS packet");
    assert_eq!(bytes[0], TS_SYNC_BYTE);
    bytes.try_into().unwrap()
}

/// Build one transmitted UP chunk's *content* (everything after the leading
/// CRC-8 byte): the 187-byte UP with its sync byte stripped, a short-form
/// ISSY (bit `[7]`=0 selects short form; EN 302 755 Annex C), then a DNP byte.
fn up_chunk_content(up: &[u8; NM_UP_SIZE], issy: [u8; 2], dnp: u8) -> Vec<u8> {
    let mut content = Vec::with_capacity(STRIDE - 1);
    content.extend_from_slice(&up[1..]); // 187 bytes, sync byte dropped
    content.extend_from_slice(&issy);
    content.push(dnp);
    content
}

fn nm_issy_npd_header(dfl_bytes: usize) -> Bbheader {
    Bbheader {
        matype: Matype {
            ts_gs: TsGs::Ts,
            sis: true,
            ccm: true,
            issyi: true,
            npd: true,
            ext: 0,
            isi: 0,
        },
        upl: (STRIDE * 8) as u16,
        sync: TS_SYNC_BYTE,
        dfl: (dfl_bytes * 8) as u16,
        syncd: 0,
        mode: Mode::Normal,
        issy_in_header: None,
    }
}

#[test]
fn nm_stride_with_issy_and_npd_is_191() {
    let hdr = nm_issy_npd_header(STRIDE * 3);
    let stride = nm_stride_bytes(&hdr).expect("ISSYI=1 + NPD=1 upl=191*8 must be valid");
    assert_eq!(stride, STRIDE);
}

#[test]
fn nm_issy_npd_recovers_real_up_and_checks_crc8_chain() {
    let up = real_up_payload();

    let chunk0 = up_chunk_content(&up, [0x00, 0x01], 0x00); // DNP=0
    let chunk1 = up_chunk_content(&up, [0x00, 0x02], 0x01); // DNP=1
    let chunk2 = up_chunk_content(&up, [0x00, 0x03], 0x02); // DNP=2

    let chunk1_leading_crc = crc8(&chunk0); // correct: chunk0's own CRC-8
    let chunk2_leading_crc = crc8(&chunk1); // correct one — then corrupted below

    let mut data = Vec::with_capacity(STRIDE * 3);
    data.push(0xEE); // chunk0's leading byte: no predecessor, never checked
    data.extend_from_slice(&chunk0);
    data.push(chunk1_leading_crc); // correct — must NOT be flagged
    data.extend_from_slice(&chunk1);
    data.push(chunk2_leading_crc ^ 0xFF); // deliberately wrong — must be flagged
    data.extend_from_slice(&chunk2);
    assert_eq!(data.len(), STRIDE * 3);

    let hdr = nm_issy_npd_header(data.len());
    let header_bytes = hdr.serialize();

    let mut extractor = CarryOverExtractor::new();
    let pkts = extractor.feed_nm(&header_bytes, &data);

    assert_eq!(pkts.len(), 3, "three transmitted UPs, one per stride");
    for (i, pkt) in pkts.iter().enumerate() {
        assert_eq!(pkt[0], TS_SYNC_BYTE, "UP {i}: sync byte restored");
        if i == 2 {
            // UP 2's CRC-8 was deliberately corrupted, so TEI must be set (W-BB-1)
            assert_eq!(
                pkt[1] & 0x80,
                0x80,
                "UP 2: TEI must be set for corrupted CRC-8"
            );
            assert_eq!(
                pkt[1] & 0x7F,
                up[1] & 0x7F,
                "UP 2: payload byte 0 matches (TEI masked)"
            );
            assert_eq!(&pkt[2..], &up[2..], "UP 2: remaining payload matches");
        } else {
            // UPs 0 and 1 have valid CRC-8 (or no predecessor), so no TEI
            assert_eq!(
                &pkt[1..],
                &up[1..],
                "UP {i}: recovered payload matches the real SCTE-35 TS packet \
                 (ISSY/DNP trailer correctly excluded from the 188-byte packet)"
            );
        }
    }

    let stats = extractor.stats();
    assert_eq!(stats.nm_upl_invalid, 0);
    assert_eq!(stats.partial_discards, 0);
    assert_eq!(
        stats.crc8_mismatches, 1,
        "only chunk2's deliberately-corrupted leading CRC-8 byte must be flagged"
    );
}
