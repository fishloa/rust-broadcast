//! Integration tests using real TV capture fixtures.

use std::fs;

fn tnt_fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/dvb-bbframe/tnt-5w-12732v-bbframe.ts"
    );
    fs::read(path).expect("fixture tnt-5w-12732v-bbframe.ts must be present")
}

fn rai_fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/dvb-bbframe/rai-5w-12606v-bbframe.ts"
    );
    fs::read(path).expect("fixture rai-5w-12606v-bbframe.ts must be present")
}

/// Walk a capture file and reassemble complete BBFrames from TS sections
/// on a given PID. The section count byte `0xB8` marks the start of a new
/// BBFrame; the final incomplete frame (if any) is discarded.
fn extract_bbframes(data: &[u8], pid: u16) -> Vec<Vec<u8>> {
    let mut frames: Vec<Vec<u8>> = Vec::new();
    let mut current_frame = Vec::with_capacity(8192);
    let mut frame_started = false;

    let mut pos = 0;
    while pos + 188 <= data.len() {
        let pkt = &data[pos..pos + 188];
        pos += 188;

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
        // Section: 00 80 00 [slen] [count]
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
        if data_end > 188 {
            continue;
        }
        let section_data = &pkt[9..data_end];

        if count == 0xB8 {
            if frame_started && !current_frame.is_empty() {
                frames.push(std::mem::take(&mut current_frame));
            }
            frame_started = true;
            current_frame.clear();
        }

        if frame_started {
            current_frame.extend_from_slice(section_data);
        }
    }

    frames
}

// ─── TNT tests ────────────────────────────────────────────────────────

#[test]
fn extract_at_least_50_bbframes_from_tnt() {
    let data = tnt_fixture();
    let frames = extract_bbframes(&data, 0x010E);
    assert!(
        frames.len() > 50,
        "expected >50 BBFrames from TNT fixture, got {}",
        frames.len()
    );
}

#[test]
fn every_tnt_bbframe_has_crc_zero_xor() {
    let data = tnt_fixture();
    use dvb_bbframe::crc::crc8;

    let frames = extract_bbframes(&data, 0x010E);
    for (i, frame) in frames.iter().enumerate() {
        if frame.len() < 10 {
            continue;
        }
        let computed = crc8(&frame[..9]);
        let stored = frame[9];
        assert_eq!(
            computed ^ stored,
            0,
            "TNT BBFrame #{i}: CRC-8 mismatch (computed=0x{computed:02X}, stored=0x{stored:02X})"
        );
    }
}

#[test]
fn every_tnt_bbframe_parses_as_normal_mode() {
    let data = tnt_fixture();
    use dvb_bbframe::header::{Bbheader, Mode};

    let frames = extract_bbframes(&data, 0x010E);
    assert!(!frames.is_empty(), "no BBFrames from TNT fixture");

    for (i, frame) in frames.iter().enumerate() {
        if frame.len() < 10 {
            continue;
        }
        let hdr = Bbheader::parse(frame)
            .unwrap_or_else(|e| panic!("TNT BBFrame #{i} failed: {e}\n  header: {frame:02X?}"));
        assert_eq!(hdr.mode, Mode::Normal, "TNT BBFrame #{i} mode");
    }
}

#[test]
fn tnt_bbframe_fields_match_known_values() {
    let data = tnt_fixture();
    use dvb_bbframe::header::{Bbheader, Mode, TsGs};

    let frames = extract_bbframes(&data, 0x010E);
    let hdr = Bbheader::parse(&frames[0]).unwrap();

    assert_eq!(hdr.mode, Mode::Normal);
    assert_eq!(hdr.matype.ts_gs, TsGs::Ts);
    assert!(!hdr.matype.sis); // MIS (multi-input stream) as seen in MATYPE-1 0xD8
    assert!(hdr.matype.ccm);
    assert!(hdr.matype.issyi);
    assert_eq!(hdr.upl, 1520);
    assert_eq!(hdr.sync, 0x47);
    assert_eq!(hdr.dfl, 57392);
}

#[test]
fn serialize_round_trip_all_tnt_bbframes() {
    let data = tnt_fixture();
    use dvb_bbframe::crc::crc8;
    use dvb_bbframe::header::Bbheader;

    let frames = extract_bbframes(&data, 0x010E);

    for (i, frame) in frames.iter().enumerate() {
        if frame.len() < 10 {
            continue;
        }
        let hdr = Bbheader::parse(frame).unwrap();
        let serialized = hdr.serialize();
        let round = Bbheader::parse(&serialized).unwrap();

        assert_eq!(hdr.matype.ts_gs, round.matype.ts_gs, "#{i} ts_gs");
        assert_eq!(hdr.matype.sis, round.matype.sis, "#{i} sis");
        assert_eq!(hdr.matype.ccm, round.matype.ccm, "#{i} ccm");
        assert_eq!(hdr.matype.issyi, round.matype.issyi, "#{i} issyi");
        assert_eq!(hdr.matype.npd, round.matype.npd, "#{i} npd");
        assert_eq!(hdr.matype.ext, round.matype.ext, "#{i} ext");
        assert_eq!(hdr.matype.isi, round.matype.isi, "#{i} isi");
        assert_eq!(hdr.upl, round.upl, "#{i} upl");
        assert_eq!(hdr.dfl, round.dfl, "#{i} dfl");
        assert_eq!(hdr.sync, round.sync, "#{i} sync");
        assert_eq!(hdr.syncd, round.syncd, "#{i} syncd");
        assert_eq!(hdr.mode, round.mode, "#{i} mode");

        let computed = crc8(&serialized[..9]);
        assert_eq!(computed ^ serialized[9], hdr.mode as u8, "#{i} crc^mode");
    }
}

// ─── Rai outer-BBFrame (DVB-S2 NM) tests ──────────────────────────────
//
// The Rai fixture (rai-5w-12606v-bbframe.ts) is a capture of the outer
// DVB-S2 layer on Eutelsat 5°W transponder 12606V — raw BBFrames wrapped
// in TS private sections on PID 0x010E. These are the DVB-S2 BBFrames a
// demod emits in BBFrame-raw mode; they carry either plain TS or inner
// T2-MI-wrapped TS in their data field.
//
// Inner HEM BBFrame validation is out of scope for dvb_bbframe tests —
// reaching them requires T2-MI packet parsing (dvb_t2mi).

#[test]
fn rai_fixture_outer_bbframes_parse_as_nm() {
    let data = rai_fixture();
    use dvb_bbframe::header::{Bbheader, Mode};

    let frames = extract_bbframes(&data, 0x010E);
    assert!(
        frames.len() > 50,
        "expected >50 outer BBFrames from Rai fixture, got {}",
        frames.len()
    );

    for (i, frame) in frames.iter().enumerate() {
        if frame.len() < 10 {
            continue;
        }
        let hdr = Bbheader::parse(frame).unwrap_or_else(|e| {
            panic!(
                "Rai outer BBHeader #{i} failed: {e}\n  header: {:02X?}",
                &frame[..10]
            )
        });
        assert_eq!(hdr.mode, Mode::Normal, "Rai outer BBHeader #{i} must be NM");
    }
}

#[test]
fn rai_fixture_outer_bbframes_pass_nm_crc8() {
    let data = rai_fixture();
    use dvb_bbframe::crc::crc8;

    let frames = extract_bbframes(&data, 0x010E);
    for (i, frame) in frames.iter().enumerate() {
        if frame.len() < 10 {
            continue;
        }
        // NM: stored CRC-8 byte equals computed CRC-8 with init=0 (MODE XOR == 0).
        let computed = crc8(&frame[..9]);
        let stored = frame[9];
        assert_eq!(
            computed, stored,
            "Rai outer BBFrame #{i}: NM CRC-8 mismatch: computed=0x{computed:02X}, stored=0x{stored:02X}"
        );
    }
}

#[test]
fn rai_outer_bbheader_round_trip() {
    let data = rai_fixture();
    use dvb_bbframe::crc::crc8;
    use dvb_bbframe::header::Bbheader;

    let frames = extract_bbframes(&data, 0x010E);

    for (i, frame) in frames.iter().enumerate() {
        let hdr = Bbheader::parse(frame).unwrap();
        let serialized = hdr.serialize();
        let round = Bbheader::parse(&serialized).unwrap();

        assert_eq!(hdr.mode, round.mode, "Rai #{i} mode");
        assert_eq!(hdr.matype.ts_gs, round.matype.ts_gs, "Rai #{i} ts_gs");
        assert_eq!(hdr.matype.sis, round.matype.sis, "Rai #{i} sis");
        assert_eq!(hdr.matype.ccm, round.matype.ccm, "Rai #{i} ccm");
        assert_eq!(hdr.matype.issyi, round.matype.issyi, "Rai #{i} issyi");
        assert_eq!(hdr.matype.npd, round.matype.npd, "Rai #{i} npd");
        assert_eq!(hdr.matype.ext, round.matype.ext, "Rai #{i} ext");
        assert_eq!(hdr.matype.isi, round.matype.isi, "Rai #{i} isi");
        assert_eq!(hdr.upl, round.upl, "Rai #{i} upl");
        assert_eq!(hdr.sync, round.sync, "Rai #{i} sync");
        assert_eq!(hdr.dfl, round.dfl, "Rai #{i} dfl");
        assert_eq!(hdr.syncd, round.syncd, "Rai #{i} syncd");

        let computed = crc8(&serialized[..9]);
        assert_eq!(
            computed ^ serialized[9],
            hdr.mode as u8,
            "Rai #{i} crc^mode"
        );
    }
}

// ─── NM stride from UPL/ISSYI/NPD (issue #1033) ───────────────────────
//
// The TNT fixture's real off-air BBFrames carry ISSYI=1 with UPL=1520 bits
// = 190 bytes (188 + a 2-byte short-form ISSY, no NPD; see
// `tnt_bbframe_fields_match_known_values` above for the raw header values).
// The Rai fixture carries a genuine *mix*: some frames ISSYI=0 (UPL=1504 =
// 188 bytes, the old code's hardcoded assumption) and others ISSYI=1
// (UPL=1520 = 190 bytes) within the same capture.
//
// A fixed 188-byte NM stride (the pre-fix behaviour: `NmTsIter`/
// `CarryOverExtractor::feed_nm_into` both hardcoded `NM_UP_SIZE`) reads every
// UP after the first in a 190-byte-stride frame two bytes short, which
// corrupts the per-UP CRC-8 chain (EN 302 755 §5.1.6/§5.1.8 figure 5): the
// CRC-8 carried in one UP's leading byte covers the *previous* UP's content,
// so any stride error desyncs `crc8(prev content) == next.leading_byte`
// almost immediately. These tests use that real, independent oracle (the
// off-air CRC-8 bytes actually transmitted by the modulator) rather than our
// own encoder, to prove the fix.
//
// TNT is MIS (MATYPE SIS=0): three Input Streams (ISI 1, 4, 6) are time-
// multiplexed on this PID, each with its own independent SYNCD/carry-over
// state (EN 302 755 §5.1.7 "ISI ... has the same meaning as PLP_ID") — so
// carry-over is tracked per ISI here, exactly as `BbframePump` tracks it per
// `plp_id` for the T2-MI-wrapped case.
#[test]
fn tnt_nm_stride_is_190_not_188_and_crc8_chain_holds() {
    use dvb_bbframe::header::Bbheader;
    use dvb_bbframe::packet::{CarryOverExtractor, nm_stride_bytes};
    use std::collections::HashMap;

    let data = tnt_fixture();
    let frames = extract_bbframes(&data, 0x010E);
    assert!(frames.len() > 50);

    let mut extractors: HashMap<u8, CarryOverExtractor> = HashMap::new();
    let mut total_packets = 0usize;
    let mut checked_a_frame = false;

    for frame in &frames {
        if frame.len() < 10 {
            continue;
        }
        let hdr = Bbheader::parse(frame).expect("TNT BBHEADER parses");
        assert!(hdr.matype.issyi, "TNT fixture is known ISSYI=1");
        assert!(!hdr.matype.npd, "TNT fixture is known NPD=0");

        let stride = nm_stride_bytes(&hdr).expect("valid NM stride from a real capture");
        assert_eq!(
            stride, 190,
            "188 (CRC-8 + 187-byte UP) + 2 (short-form ISSY), no DNP — §5.1.8 figure 5"
        );

        let header_bytes: [u8; 10] = frame[..10].try_into().unwrap();
        let extractor = extractors.entry(hdr.matype.isi).or_default();
        let pkts = extractor.feed_nm(&header_bytes, &frame[10..]);
        total_packets += pkts.len();
        checked_a_frame = true;
    }

    assert!(checked_a_frame);
    assert_eq!(extractors.len(), 3, "TNT multiplexes 3 ISIs (1, 4, 6)");
    assert!(total_packets > 100, "expected many recovered TS packets");
    let mut stats = dvb_bbframe::packet::CarryOverStats::default();
    for e in extractors.values() {
        let s = e.stats();
        stats.nm_upl_invalid += s.nm_upl_invalid;
        stats.partial_discards += s.partial_discards;
        stats.crc8_mismatches += s.crc8_mismatches;
    }
    assert_eq!(
        stats.nm_upl_invalid, 0,
        "every real TNT header has a valid UPL"
    );
    assert_eq!(
        stats.partial_discards, 0,
        "carry-over must track the real 190-byte stride per ISI, not resync every frame"
    );
    assert_eq!(
        stats.crc8_mismatches, 0,
        "the real off-air CRC-8 chain must verify once the stride is read from UPL"
    );

    // Pre-fix reference: what a hardcoded 188-byte stride actually produced.
    // Not a call into removed code (the old `NmTsIter`/`feed_nm_into` took no
    // stride parameter at all) — this reimplements exactly that fixed-stride
    // walk over the same real bytes (still demuxed per ISI, so the only
    // variable versus the fixed loop above is the stride), to show *why* it
    // broke the CRC-8 chain.
    let mut naive_prev: HashMap<u8, Option<u8>> = HashMap::new();
    let mut naive_mismatches = 0usize;
    let mut naive_checked = 0usize;
    for frame in frames.iter().filter(|f| f.len() >= 10) {
        let hdr = Bbheader::parse(frame).unwrap();
        let dfl_bytes = (hdr.dfl / 8) as usize;
        let data = &frame[10..10 + dfl_bytes.min(frame.len() - 10)];
        let prev = naive_prev.entry(hdr.matype.isi).or_insert(None);
        let mut i = 0usize;
        const OLD_FIXED_STRIDE: usize = 188;
        while i + OLD_FIXED_STRIDE <= data.len() {
            let chunk = &data[i..i + OLD_FIXED_STRIDE];
            if let Some(expected) = *prev {
                naive_checked += 1;
                if chunk[0] != expected {
                    naive_mismatches += 1;
                }
            }
            *prev = Some(dvb_bbframe::crc::crc8(&chunk[1..]));
            i += OLD_FIXED_STRIDE;
        }
    }
    assert!(naive_checked > 100);
    assert!(
        naive_mismatches * 2 > naive_checked,
        "a fixed 188-byte stride against a real 190-byte-stride capture must \
         desync the CRC-8 chain almost immediately (got {naive_mismatches}/{naive_checked})"
    );
}

#[test]
fn rai_nm_mixed_issy_stride_from_upl() {
    use dvb_bbframe::header::Bbheader;
    use dvb_bbframe::packet::{CarryOverExtractor, nm_stride_bytes};
    use std::collections::HashMap;

    let data = rai_fixture();
    let frames = extract_bbframes(&data, 0x010E);
    assert!(frames.len() > 50);

    let mut saw_188 = false;
    let mut saw_190 = false;
    let mut extractors: HashMap<u8, CarryOverExtractor> = HashMap::new();
    let mut total_packets = 0usize;

    for frame in &frames {
        if frame.len() < 10 {
            continue;
        }
        let hdr = Bbheader::parse(frame).expect("Rai BBHEADER parses");
        assert!(!hdr.matype.npd, "Rai fixture is known NPD=0");
        let stride = nm_stride_bytes(&hdr).expect("valid NM stride from a real capture");
        match stride {
            188 => saw_188 = true,
            190 => saw_190 = true,
            other => panic!("unexpected Rai stride {other}"),
        }
        let header_bytes: [u8; 10] = frame[..10].try_into().unwrap();
        // Rai may also be MIS; key carry-over per ISI as for TNT above.
        let key = if hdr.matype.sis { 0 } else { hdr.matype.isi };
        let extractor = extractors.entry(key).or_default();
        total_packets += extractor.feed_nm(&header_bytes, &frame[10..]).len();
    }

    assert!(saw_188, "expected some ISSYI=0 (188-byte) Rai frames");
    assert!(saw_190, "expected some ISSYI=1 (190-byte) Rai frames");
    assert!(total_packets > 100);
    let mut stats = dvb_bbframe::packet::CarryOverStats::default();
    for e in extractors.values() {
        let s = e.stats();
        stats.nm_upl_invalid += s.nm_upl_invalid;
        stats.crc8_mismatches += s.crc8_mismatches;
    }
    assert_eq!(stats.nm_upl_invalid, 0);
    assert_eq!(stats.crc8_mismatches, 0);
}
