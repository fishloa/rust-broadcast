//! Integration tests using real TV capture fixtures.

// Fixtures live in the top-level zenith repo at tests/bbframe/. dvb_bbframe
// borrows them via relative include_bytes! rather than duplicating the files.
// This keeps a single source of truth for fixture data across the workspace.
const TNT_FIXTURE: &[u8] = include_bytes!("../../../tests/bbframe/tnt-5w-12732v-bbframe.ts");
const RAI_FIXTURE: &[u8] = include_bytes!("../../../tests/bbframe/rai-5w-12606v-bbframe.ts");

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
    let frames = extract_bbframes(TNT_FIXTURE, 0x010E);
    assert!(
        frames.len() > 50,
        "expected >50 BBFrames from TNT fixture, got {}",
        frames.len()
    );
}

#[test]
fn every_tnt_bbframe_has_crc_zero_xor() {
    use dvb_bbframe::crc::crc8;

    let frames = extract_bbframes(TNT_FIXTURE, 0x010E);
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
    use dvb_bbframe::header::{Bbheader, Mode};

    let frames = extract_bbframes(TNT_FIXTURE, 0x010E);
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
    use dvb_bbframe::header::{Bbheader, Mode, TsGs};

    let frames = extract_bbframes(TNT_FIXTURE, 0x010E);
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
    use dvb_bbframe::crc::crc8;
    use dvb_bbframe::header::Bbheader;

    let frames = extract_bbframes(TNT_FIXTURE, 0x010E);

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
// reaching them requires T2-MI packet parsing (dvb_t2mi). Those tests
// live at the zenith integration layer.

#[test]
fn rai_fixture_outer_bbframes_parse_as_nm() {
    use dvb_bbframe::header::{Bbheader, Mode};

    let frames = extract_bbframes(RAI_FIXTURE, 0x010E);
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
            panic!("Rai outer BBHeader #{i} failed: {e}\n  header: {:02X?}", &frame[..10])
        });
        assert_eq!(hdr.mode, Mode::Normal, "Rai outer BBHeader #{i} must be NM");
    }
}

#[test]
fn rai_fixture_outer_bbframes_pass_nm_crc8() {
    use dvb_bbframe::crc::crc8;

    let frames = extract_bbframes(RAI_FIXTURE, 0x010E);
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
    use dvb_bbframe::crc::crc8;
    use dvb_bbframe::header::Bbheader;

    let frames = extract_bbframes(RAI_FIXTURE, 0x010E);

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
