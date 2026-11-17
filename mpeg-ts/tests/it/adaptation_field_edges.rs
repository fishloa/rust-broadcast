//! Adaptation-field parse/serialize edges: every flag subset round-trips
//! through a real 188-byte packet, a truncated field body is rejected, fields
//! ending exactly at the packet end parse, and `set_pcr`/`set_opcr` report a
//! field one byte too short.

use mpeg_ts::OwnedTsPacket;
use mpeg_ts::ts::{
    ADAPTATION_FLAG, AF_PCR_FLAG, AdaptationField, AdaptationFieldExtension, Ltw, LtwOffset,
    PAYLOAD_FLAG, Pcr, PiecewiseRate, SeamlessSplice, TS_PACKET_SIZE, TS_SYNC_BYTE, TsPacket,
};
use mpeg_ts::ts_dup::{DuplicateVerdict, check_duplicate};

const AF_OPCR_FLAG: u8 = 0x08;
const AF_SPLICING_FLAG: u8 = 0x04;
const AF_TPD_FLAG: u8 = 0x02;
const AF_EXTENSION_FLAG: u8 = 0x01;

const TPD: [u8; 3] = [0xAB, 0xCD, 0xEF];

fn packet_with_body(body: &[u8], afc: u8) -> [u8; TS_PACKET_SIZE] {
    assert!(body.len() <= 183);
    let mut raw = [0u8; TS_PACKET_SIZE];
    raw[0] = TS_SYNC_BYTE;
    raw[1] = 0x01;
    raw[2] = 0x00;
    raw[3] = afc;
    raw[4] = body.len() as u8;
    raw[5..5 + body.len()].copy_from_slice(body);
    raw
}

fn extension() -> AdaptationFieldExtension<'static> {
    let mut ext = AdaptationFieldExtension::new();
    ext.ltw = Some(Ltw {
        ltw_valid_flag: true,
        ltw_offset: LtwOffset::new(0x1234).expect("15-bit"),
    });
    ext.piecewise_rate = Some(PiecewiseRate::new(0x2A_BCDE).expect("22-bit"));
    ext.piecewise_rate_reserved = 0b10;
    ext.seamless_splice = Some(SeamlessSplice::new(3, 0x1234_5678));
    ext
}

fn buffer_too_short(err: &mpeg_ts::Error) -> Option<(usize, usize)> {
    match err {
        mpeg_ts::Error::BufferTooShort(e) => Some((e.need, e.have)),
        _ => None,
    }
}

#[test]
fn every_adaptation_flag_subset_round_trips_through_a_packet() {
    let pcr = Pcr::from_parts(0x1234_5678, 0x123).expect("PCR in range");
    let opcr = Pcr::from_parts(0x0987_6543, 0x045).expect("OPCR in range");
    for combo in 0u32..32 {
        let mut af = AdaptationField::new();
        if combo & 1 != 0 {
            af.pcr = Some(pcr);
            af.pcr_reserved = 0b010101;
        }
        if combo & 2 != 0 {
            af.opcr = Some(opcr);
            af.opcr_reserved = 0b101010;
        }
        if combo & 4 != 0 {
            af.splice_countdown = Some(-3);
        }
        if combo & 8 != 0 {
            af.transport_private_data = Some(&TPD);
        }
        if combo & 16 != 0 {
            af.extension = Some(extension());
        }

        let mut body = vec![0u8; af.serialized_len()];
        let written = af.serialize_into(&mut body).expect("serialize");
        assert_eq!(written, body.len());

        let raw = packet_with_body(&body, ADAPTATION_FLAG);
        let pkt = TsPacket::parse(&raw).expect("packet parse");
        let parsed = pkt
            .adaptation_field()
            .expect("adaptation field present")
            .expect("adaptation field valid");
        assert_eq!(parsed, af, "combo {combo:#07b}");

        if body.len() > 1 {
            let short = packet_with_body(&body[..body.len() - 1], ADAPTATION_FLAG);
            let pkt = TsPacket::parse(&short).expect("packet parse");
            let err = pkt
                .adaptation_field()
                .expect("adaptation field present")
                .expect_err("truncated body rejected");
            assert!(
                buffer_too_short(&err).is_some(),
                "combo {combo:#07b}: {err:?}"
            );
        }
    }
}

#[test]
fn a_truncated_extension_body_is_rejected() {
    let body = [AF_EXTENSION_FLAG, 0x01];
    let raw = packet_with_body(&body, ADAPTATION_FLAG);
    let pkt = TsPacket::parse(&raw).expect("packet parse");
    let err = pkt
        .adaptation_field()
        .expect("adaptation field present")
        .expect_err("no flags byte inside the extension");
    assert_eq!(buffer_too_short(&err), Some((2, 1)));
}

#[test]
fn a_truncated_extension_reports_the_full_declared_length() {
    let body = [AF_EXTENSION_FLAG, 0x05, 0x00, 0x00];
    let raw = packet_with_body(&body, ADAPTATION_FLAG);
    let pkt = TsPacket::parse(&raw).expect("packet parse");
    let err = pkt
        .adaptation_field()
        .expect("adaptation field present")
        .expect_err("extension body shorter than its length");
    assert_eq!(buffer_too_short(&err), Some((6, 3)));
}

#[test]
fn a_truncated_ltw_field_inside_the_extension_is_rejected() {
    let body = [AF_EXTENSION_FLAG, 0x02, AFE_LTW_FLAG, 0x00];
    let raw = packet_with_body(&body, ADAPTATION_FLAG);
    let pkt = TsPacket::parse(&raw).expect("packet parse");
    let err = pkt
        .adaptation_field()
        .expect("adaptation field present")
        .expect_err("LTW truncated");
    assert_eq!(buffer_too_short(&err), Some((3, 2)));
}

const AFE_LTW_FLAG: u8 = 0x80;

#[test]
fn a_missing_splice_countdown_reports_the_byte_it_needs() {
    let body = [AF_SPLICING_FLAG];
    let raw = packet_with_body(&body, ADAPTATION_FLAG);
    let pkt = TsPacket::parse(&raw).expect("packet parse");
    let err = pkt
        .adaptation_field()
        .expect("adaptation field present")
        .expect_err("splice countdown absent");
    assert_eq!(buffer_too_short(&err), Some((2, 1)));
}

#[test]
fn a_missing_private_data_length_reports_the_byte_it_needs() {
    let body = [AF_TPD_FLAG];
    let raw = packet_with_body(&body, ADAPTATION_FLAG);
    let pkt = TsPacket::parse(&raw).expect("packet parse");
    let err = pkt
        .adaptation_field()
        .expect("adaptation field present")
        .expect_err("transparent private data length absent");
    assert_eq!(buffer_too_short(&err), Some((2, 1)));
}

#[test]
fn an_adaptation_field_ending_exactly_at_the_packet_end_parses() {
    let mut body = vec![0u8; 183];
    body[0] = 0x00;
    for b in &mut body[1..] {
        *b = 0xFF;
    }
    let raw = packet_with_body(&body, ADAPTATION_FLAG);
    let pkt = TsPacket::parse(&raw).expect("packet parse");
    let af = pkt
        .adaptation_field()
        .expect("adaptation field present")
        .expect("adaptation field valid");
    assert_eq!(af.stuffing.len(), 182);

    let raw = packet_with_body(&body, ADAPTATION_FLAG | PAYLOAD_FLAG);
    let pkt = TsPacket::parse(&raw).expect("packet parse");
    assert!(pkt.payload.is_none());
}

#[test]
fn set_pcr_and_set_opcr_reject_a_field_one_byte_too_short() {
    let mut raw = packet_with_body(&[AF_PCR_FLAG], ADAPTATION_FLAG | PAYLOAD_FLAG);
    raw[4] = 6;
    raw[5] = AF_PCR_FLAG;
    assert!(matches!(
        OwnedTsPacket::set_pcr(&mut raw, Pcr::from_parts(1, 0).expect("in range")),
        Err(mpeg_ts::Error::AdaptationFieldTooShort { need: 7, have: 6 })
    ));

    let mut raw = packet_with_body(
        &[AF_PCR_FLAG | AF_OPCR_FLAG],
        ADAPTATION_FLAG | PAYLOAD_FLAG,
    );
    raw[4] = 12;
    raw[5] = AF_PCR_FLAG | AF_OPCR_FLAG;
    assert!(matches!(
        OwnedTsPacket::set_opcr(&mut raw, Pcr::from_parts(1, 0).expect("in range")),
        Err(mpeg_ts::Error::AdaptationFieldTooShort { need: 13, have: 12 })
    ));
}

#[test]
fn a_pcr_field_ending_at_the_packet_end_is_located() {
    let mut prev = [0u8; 12];
    prev[0] = TS_SYNC_BYTE;
    prev[3] = ADAPTATION_FLAG | PAYLOAD_FLAG;
    prev[4] = 7;
    prev[5] = AF_PCR_FLAG;
    prev[6..12].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
    let mut curr = prev;
    curr[8] = 0xEE;
    assert_eq!(
        check_duplicate(&prev, &curr, false),
        DuplicateVerdict::Legal,
        "a differing byte inside the located PCR field is exempt"
    );
}
