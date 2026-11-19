//! `CarryOverExtractor` cross-frame carry-over — EN 302 755 §5.1.7/§5.1.8.
//!
//! A user packet split across BBFrames must be reassembled byte-exactly: the
//! NM transmitted UP is `CRC-8 + 187 bytes` (stride 188) and the HEM UP is
//! 187 bytes with the sync byte prepended. SYNCD is a bit offset to the first
//! UP of the DATA FIELD; 0xFFFF means the whole field continues the carried
//! UP (EN 302 755 Table 2). The real TS packet below comes from the off-air
//! TNT BBFrame capture, so the expected bytes are not produced by this crate.

use std::collections::HashMap;
use std::fs;

use dvb_bbframe::header::{BBHEADER_LEN, Bbheader, Matype, Mode, TsGs};
use dvb_bbframe::packet::{CarryOverExtractor, HEM_UP_SIZE, NM_UP_SIZE, TS_SYNC_BYTE};

use crate::integration_fixture::extract_bbframes;

const ALL_CONTINUATION: u16 = 0xFFFF;

/// A genuine 188-byte inner TS packet recovered from the TNT capture.
fn real_ts_packet() -> [u8; NM_UP_SIZE] {
    let path = test_fixtures::path("bbframe/tnt-5w-12732v-bbframe.ts");
    let data = fs::read(path).expect("bbframe fixture must be present");
    let frames = extract_bbframes(&data, 0x010E);

    let mut extractors: HashMap<u8, CarryOverExtractor> = HashMap::new();
    for frame in &frames {
        let Ok(hdr) = Bbheader::parse(frame) else {
            continue;
        };
        if hdr.mode != Mode::Normal {
            continue;
        }
        let header_bytes: [u8; BBHEADER_LEN] = frame[..BBHEADER_LEN].try_into().unwrap();
        let extractor = extractors.entry(hdr.matype.isi).or_default();
        for packet in extractor.feed_nm(&header_bytes, &frame[BBHEADER_LEN..]) {
            if packet[0] == TS_SYNC_BYTE && packet[1] & 0x80 == 0 && packet[1] != 0x00 {
                return packet;
            }
        }
    }
    panic!("the TNT fixture must yield at least one clean inner TS packet");
}

fn header(mode: Mode, dfl_bits: u16, syncd: u16) -> [u8; BBHEADER_LEN] {
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
        upl: if mode == Mode::Normal {
            (NM_UP_SIZE * 8) as u16
        } else {
            0
        },
        sync: if mode == Mode::Normal {
            TS_SYNC_BYTE
        } else {
            0
        },
        dfl: dfl_bits,
        syncd,
        mode,
        issy_in_header: None,
    }
    .serialize()
    .unwrap()
}

#[test]
fn hem_up_split_across_three_frames_is_reassembled() {
    let packet = real_ts_packet();
    let up = &packet[1..]; // HEM: 187 bytes, no sync byte
    let mut ex = CarryOverExtractor::new();

    // Frame A: first 100 bytes. The buffer handed in is longer than DFL;
    // only the declared DATA FIELD may be read.
    let mut over_long = vec![0u8; 200];
    over_long[..100].copy_from_slice(&up[..100]);
    assert!(
        ex.feed_hem(&header(Mode::HighEfficiency, 100 * 8, 0), &over_long, false)
            .is_empty(),
        "100 of 187 bytes must not emit"
    );

    // Frame B: next 50 bytes, still short of the 188-byte carry buffer.
    let out = ex.feed_hem(
        &header(Mode::HighEfficiency, 50 * 8, ALL_CONTINUATION),
        &up[100..150],
        false,
    );
    assert!(out.is_empty(), "151 of 188 carried bytes must not emit");

    // Frame C: the last 37 bytes complete the UP.
    let out = ex.feed_hem(
        &header(Mode::HighEfficiency, 37 * 8, ALL_CONTINUATION),
        &up[150..],
        false,
    );
    assert_eq!(out.len(), 1, "the split UP must complete exactly once");
    assert_eq!(out[0], packet, "188 bytes must be byte-identical");
    assert_eq!(ex.stats().partial_discards, 0);
}

#[test]
fn hem_carry_needs_the_final_byte_before_it_emits() {
    let packet = real_ts_packet();
    let up = &packet[1..];
    let mut ex = CarryOverExtractor::new();

    // 186 bytes: the carry buffer is one byte short of its 188-byte width.
    assert!(
        ex.feed_hem(
            &header(Mode::HighEfficiency, 186 * 8, 0),
            &up[..HEM_UP_SIZE - 1],
            false
        )
        .is_empty()
    );
    let out = ex.feed_hem(
        &header(Mode::HighEfficiency, 8, ALL_CONTINUATION),
        &up[HEM_UP_SIZE - 1..],
        false,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0], packet);
}

#[test]
fn hem_mismatched_syncd_discards_the_partial() {
    let mut ex = CarryOverExtractor::new();
    assert!(
        ex.feed_hem(
            &header(Mode::HighEfficiency, 100 * 8, 0),
            &[0xA5; 100],
            false
        )
        .is_empty()
    );

    // need = 188 - 101 = 87; SYNCD claims 50 bytes.
    let out = ex.feed_hem(
        &header(Mode::HighEfficiency, 50 * 8, 50 * 8),
        &[0x5A; 50],
        false,
    );
    assert!(out.is_empty());
    assert_eq!(ex.stats().partial_discards, 1);

    // Stranded SYNCD matching the need but without enough bytes is also a
    // mismatch, not an out-of-bounds copy.
    assert!(
        ex.feed_hem(
            &header(Mode::HighEfficiency, 100 * 8, 0),
            &[0xA5; 100],
            false
        )
        .is_empty()
    );
    let out = ex.feed_hem(
        &header(Mode::HighEfficiency, 50 * 8, 87 * 8),
        &[0x5A; 50],
        false,
    );
    assert!(out.is_empty());
    assert_eq!(ex.stats().partial_discards, 2);
}

#[test]
fn hem_whole_up_frame_leaves_no_carry() {
    let packet = real_ts_packet();
    let up = &packet[1..];
    let mut ex = CarryOverExtractor::new();

    let out = ex.feed_hem(&header(Mode::HighEfficiency, 187 * 8, 0), up, false);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0], packet);

    // A continuation frame with nothing carried has nothing to continue.
    let out = ex.feed_hem(
        &header(Mode::HighEfficiency, 187 * 8, ALL_CONTINUATION),
        up,
        false,
    );
    assert!(out.is_empty(), "no carry may survive a whole-UP frame");
}

#[test]
fn nm_up_split_across_three_frames_is_reassembled() {
    let packet = real_ts_packet();
    // NM transmitted UP: a leading CRC-8 byte (unchecked on a fresh chain)
    // followed by the original UP without its sync byte.
    let mut up = vec![0x00u8];
    up.extend_from_slice(&packet[1..]);
    assert_eq!(up.len(), NM_UP_SIZE);

    let mut ex = CarryOverExtractor::new();

    let mut over_long = vec![0u8; 200];
    over_long[..100].copy_from_slice(&up[..100]);
    assert!(
        ex.feed_nm(&header(Mode::Normal, 100 * 8, 0), &over_long)
            .is_empty()
    );

    let out = ex.feed_nm(
        &header(Mode::Normal, 50 * 8, ALL_CONTINUATION),
        &up[100..150],
    );
    assert!(out.is_empty(), "150 of 188 carried bytes must not emit");

    let out = ex.feed_nm(&header(Mode::Normal, 38 * 8, ALL_CONTINUATION), &up[150..]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0], packet);
    assert_eq!(ex.stats().partial_discards, 0);
}

#[test]
fn nm_carry_needs_the_final_byte_before_it_emits() {
    let packet = real_ts_packet();
    let mut up = vec![0x00u8];
    up.extend_from_slice(&packet[1..]);
    let mut ex = CarryOverExtractor::new();

    assert!(
        ex.feed_nm(&header(Mode::Normal, 187 * 8, 0), &up[..NM_UP_SIZE - 1])
            .is_empty()
    );
    let out = ex.feed_nm(
        &header(Mode::Normal, 8, ALL_CONTINUATION),
        &up[NM_UP_SIZE - 1..],
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0], packet);
}

#[test]
fn nm_mismatched_syncd_discards_the_partial() {
    let mut ex = CarryOverExtractor::new();
    assert!(
        ex.feed_nm(&header(Mode::Normal, 100 * 8, 0), &[0xA5; 100])
            .is_empty()
    );

    // need = 188 - 100 = 88; SYNCD claims 50 bytes.
    let out = ex.feed_nm(&header(Mode::Normal, 50 * 8, 50 * 8), &[0x5A; 50]);
    assert!(out.is_empty());
    assert_eq!(ex.stats().partial_discards, 1);

    assert!(
        ex.feed_nm(&header(Mode::Normal, 100 * 8, 0), &[0xA5; 100])
            .is_empty()
    );
    let out = ex.feed_nm(&header(Mode::Normal, 50 * 8, 88 * 8), &[0x5A; 50]);
    assert!(out.is_empty());
    assert_eq!(ex.stats().partial_discards, 2);
}

#[test]
fn nm_whole_up_frame_leaves_no_carry() {
    let packet = real_ts_packet();
    let mut up = vec![0x00u8];
    up.extend_from_slice(&packet[1..]);
    let mut ex = CarryOverExtractor::new();

    let out = ex.feed_nm(&header(Mode::Normal, 188 * 8, 0), &up);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0], packet);

    let out = ex.feed_nm(&header(Mode::Normal, 188 * 8, ALL_CONTINUATION), &up);
    assert!(out.is_empty(), "no carry may survive a whole-UP frame");
}
