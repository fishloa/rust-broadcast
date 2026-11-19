//! `BbframePump` diagnostic counters, PLP carry discard and header boundaries.
//!
//! The pump stays infallible on wire-derived input; these counters are the
//! only observable signal that a frame was skipped, so their exact values
//! (EN 302 755 §5.1.7 BBHEADER, §5.1.8 user-packet framing) are pinned here.

use dvb_bbframe::header::{BBHEADER_LEN, Bbheader, Matype, Mode, TsGs};
use dvb_bbframe::packet::{CarryOverExtractor, HEM_UP_SIZE, NM_UP_SIZE};
use dvb_bbframe::pump::BbframePump;

fn header(
    mode: Mode,
    ts_gs: TsGs,
    npd: bool,
    upl: u16,
    dfl_bits: u16,
    syncd: u16,
) -> [u8; BBHEADER_LEN] {
    Bbheader {
        matype: Matype {
            ts_gs,
            sis: true,
            ccm: true,
            issyi: false,
            npd,
            ext: 0,
            isi: 0,
        },
        upl: if mode == Mode::Normal { upl } else { 0 },
        sync: if mode == Mode::Normal { 0x47 } else { 0 },
        dfl: dfl_bits,
        syncd,
        mode,
        issy_in_header: None,
    }
    .serialize()
    .unwrap()
}

fn frame(hdr: &[u8; BBHEADER_LEN], data: &[u8]) -> Vec<u8> {
    let mut bytes = hdr.to_vec();
    bytes.extend_from_slice(data);
    bytes
}

const ALL_CONTINUATION: u16 = 0xFFFF;

#[test]
fn carry_over_counters_sum_across_plps() {
    let mut pump = BbframePump::new();

    // PLP 1: two NPD-HEM frames the extractor cannot recover (2).
    let npd_hem = header(Mode::HighEfficiency, TsGs::Ts, true, 0, 0, 0);
    for _ in 0..2 {
        assert!(pump.feed(1, &frame(&npd_hem, &[])).is_empty());
    }

    // PLP 2: three partial UPs discarded by a mismatched SYNCD (3).
    let partial = header(Mode::HighEfficiency, TsGs::Ts, false, 0, 800, 0);
    let mismatched = header(Mode::HighEfficiency, TsGs::Ts, false, 0, 400, 50 * 8);
    for _ in 0..3 {
        assert!(pump.feed(2, &frame(&partial, &[0xA5; 100])).is_empty());
        assert!(pump.feed(2, &frame(&mismatched, &[0x5A; 50])).is_empty());
    }

    // PLP 3: four NM frames whose UPL cannot describe a stride (4).
    let bad_upl = header(Mode::Normal, TsGs::Ts, false, 1505, 0, 0);
    for _ in 0..4 {
        assert!(pump.feed(3, &frame(&bad_upl, &[0x33; 10])).is_empty());
    }

    // PLP 4: five NM UPs whose leading CRC-8 byte is wrong (5).
    let crc_hdr = header(Mode::Normal, TsGs::Ts, false, 1504, 1504, 0);
    let wrong_leading = dvb_bbframe::crc::crc8(&[0x11u8; NM_UP_SIZE - 1]) ^ 0xFF;
    let mut nm_frame = vec![wrong_leading];
    nm_frame.extend_from_slice(&[0x11; NM_UP_SIZE - 1]);
    for _ in 0..6 {
        assert_eq!(
            pump.feed(4, &frame(&crc_hdr, &nm_frame)).len(),
            1,
            "a CRC-8 mismatch emits the packet flagged with TEI"
        );
    }

    // Pump-level counters, at distinct values (3 short, 2 non-TS).
    for _ in 0..3 {
        assert!(pump.feed(6, &[0u8; BBHEADER_LEN - 1]).is_empty());
    }
    let gse = header(Mode::Normal, TsGs::Gse, false, 0, 0, 0);
    for _ in 0..2 {
        assert!(pump.feed(6, &frame(&gse, &[])).is_empty());
    }

    let stats = pump.stats();
    assert_eq!(stats.header_parse_failures, 3);
    assert_eq!(stats.non_ts_payloads, 2);
    assert_eq!(stats.carry_over.npd_unsupported, 2);
    assert_eq!(stats.carry_over.partial_discards, 3);
    assert_eq!(stats.carry_over.nm_upl_invalid, 4);
    assert_eq!(stats.carry_over.crc8_mismatches, 5);
    assert_eq!(stats.carry_over.mode_mismatches, 0);
    assert_eq!(stats.carry_over.header_parse_failures, 0);
}

#[test]
fn non_ts_frame_discards_the_plp_carry() {
    let mut pump = BbframePump::new();

    // Frame A: HEM, 100 bytes of a 187-byte UP → carried over.
    let a = header(Mode::HighEfficiency, TsGs::Ts, false, 0, 100 * 8, 0);
    assert!(pump.feed(5, &frame(&a, &[0xA5; 100])).is_empty());

    // A non-TS frame for the same PLP must drop that carry, not glue the next
    // frame's bytes onto it.
    let gse = header(Mode::HighEfficiency, TsGs::Gse, false, 0, 0, 0);
    assert!(pump.feed(5, &frame(&gse, &[])).is_empty());
    assert_eq!(pump.stats().non_ts_payloads, 1);

    // Frame B would have completed the stale partial UP.
    let b = header(
        Mode::HighEfficiency,
        TsGs::Ts,
        false,
        0,
        (HEM_UP_SIZE - 100) as u16 * 8,
        ALL_CONTINUATION,
    );
    assert!(
        pump.feed(5, &frame(&b, &[0x5A; HEM_UP_SIZE - 100]))
            .is_empty(),
        "a discarded carry must not complete from the next frame"
    );
    assert_eq!(pump.stats().carry_over.partial_discards, 0);
}

#[test]
fn header_only_frame_is_valid_not_a_parse_failure() {
    let mut pump = BbframePump::new();
    let hdr = header(Mode::Normal, TsGs::Ts, false, 1504, 0, 0);
    assert_eq!(hdr.len(), BBHEADER_LEN);
    assert!(pump.feed(0, &hdr).is_empty());
    assert_eq!(pump.stats().header_parse_failures, 0);

    let mut short = BbframePump::new();
    assert!(short.feed(0, &hdr[..BBHEADER_LEN - 1]).is_empty());
    assert_eq!(short.stats().header_parse_failures, 1);
}

#[test]
fn one_byte_less_than_a_header_is_a_parse_failure() {
    let mut pump = BbframePump::new();
    assert!(pump.feed(0, &[0u8; BBHEADER_LEN - 1]).is_empty());
    assert_eq!(pump.stats().header_parse_failures, 1);
    assert_eq!(pump.stats().non_ts_payloads, 0);
}

#[test]
fn hem_path_counts_a_corrupt_header_as_a_parse_failure() {
    let mut ex = CarryOverExtractor::new();
    let mut bad = header(Mode::HighEfficiency, TsGs::Ts, false, 0, 0, 0);
    bad[9] ^= 0x55;
    assert!(ex.feed_hem(&bad, &[0u8; HEM_UP_SIZE], false).is_empty());
    assert_eq!(ex.stats().header_parse_failures, 1);
}

#[test]
fn nm_path_counts_a_hem_header_as_a_mode_mismatch() {
    let mut ex = CarryOverExtractor::new();
    let hem = header(Mode::HighEfficiency, TsGs::Ts, false, 0, 0, 0);
    assert!(ex.feed_nm(&hem, &[0u8; NM_UP_SIZE]).is_empty());
    assert_eq!(ex.stats().mode_mismatches, 1);
}
