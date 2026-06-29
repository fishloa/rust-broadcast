//! Validate `dvb-subtitle` against real broadcast captures.
//!
//! Extracts the DVB subtitle PES (data_identifier 0x20) from the committed
//! `dvb-si` capture fixtures, reassembles each PES with `mpeg-pes`, and parses
//! the PES data field into typed segments — then proves the serializer is
//! byte-exact on real-world bytes (the round-trip invariant, on data we did
//! not author).

use broadcast_common::{Parse, Serialize};
use dvb_subtitle::{AnySegment, PesDataField};
use mpeg_pes::{PesAssembler, PesPacket, StreamId};

const M6: &[u8] = include_bytes!("../../dvb-si/tests/fixtures/m6-single.ts");
const PKT: usize = 188;
const SUBTITLE_PID: u16 = 0x008C; // component in m6-single.ts carrying DVB subtitle
                                  // DVB subtitle is carried in private_stream_1 with data_identifier 0x20; the same
                                  // PID also multiplexes padding_stream (0xBE) PES, which are not subtitle.
const PRIVATE_STREAM_1: StreamId = StreamId(0xBD);
const DATA_IDENTIFIER_DVB_SUBTITLE: u8 = 0x20;

/// (pid, payload_unit_start, payload) for a TS packet carrying payload.
fn ts_payload(p: &[u8]) -> Option<(u16, bool, &[u8])> {
    if p.len() != PKT || p[0] != 0x47 {
        return None;
    }
    let pid = (u16::from(p[1] & 0x1F) << 8) | u16::from(p[2]);
    let pusi = p[1] & 0x40 != 0;
    let start = match (p[3] >> 4) & 0x03 {
        1 => 4,
        3 => 5 + usize::from(p[4]),
        _ => return None,
    };
    (start < PKT).then(|| (pid, pusi, &p[start..]))
}

/// Reassemble the subtitle PES packets on `pid` from a TS capture.
fn subtitle_pes(ts: &[u8], pid: u16) -> Vec<Vec<u8>> {
    let mut asm = PesAssembler::new();
    let mut out = Vec::new();
    for pkt in ts.chunks(PKT) {
        if let Some((p, pusi, payload)) = ts_payload(pkt) {
            if p == pid {
                if let Some(v) = asm.feed(pusi, payload) {
                    out.push(v);
                }
            }
        }
    }
    if let Some(v) = asm.flush() {
        out.push(v);
    }
    out
}

#[test]
fn parses_real_subtitle_pes_from_m6() {
    let pes = subtitle_pes(M6, SUBTITLE_PID);
    assert!(!pes.is_empty(), "no subtitle PES reassembled");

    let mut fields = 0;
    let mut total_segments = 0;
    let mut known_segments = 0;
    let mut clut_segments = 0;

    for raw in &pes {
        let pkt = PesPacket::parse(raw).expect("subtitle PES parses");
        // Only private_stream_1 PES carrying data_identifier 0x20 are DVB
        // subtitle; skip the padding_stream PES multiplexed on the same PID.
        if pkt.stream_id != PRIVATE_STREAM_1
            || pkt.payload.first() != Some(&DATA_IDENTIFIER_DVB_SUBTITLE)
        {
            continue;
        }
        // The PES payload IS the subtitling PES_data_field.
        let field = PesDataField::parse(pkt.payload).expect("PES_data_field parses");
        fields += 1;
        total_segments += field.segments.len();
        for seg in &field.segments {
            if !matches!(seg, AnySegment::Unknown { .. }) {
                known_segments += 1;
            }
            if matches!(seg, AnySegment::ClutDefinition(_)) {
                clut_segments += 1;
            }
        }
        // Byte-exact round-trip on real captured bytes — this is the gate that
        // bites: serialize must reconstruct the field from the parsed segments.
        assert_eq!(
            field.to_bytes(),
            pkt.payload,
            "subtitle PES_data_field must round-trip byte-identically"
        );
    }

    assert!(fields >= 1, "expected at least one PES_data_field");
    assert!(
        total_segments >= 1,
        "expected at least one subtitling segment, got {total_segments}"
    );
    assert!(
        known_segments >= 1,
        "expected at least one recognised segment type, got {known_segments}"
    );
    // A complete CLUT_definition_segment is present in this capture; it must be
    // decoded as a typed segment, not silently degraded to Unknown.
    assert!(
        clut_segments >= 1,
        "expected a typed CLUT_definition segment, got {clut_segments} (degraded to Unknown?)"
    );
    eprintln!(
        "fixture_subtitle: {fields} PES data fields, {total_segments} segments ({known_segments} typed, {clut_segments} CLUT)"
    );
}
