//! ST 2038 Table 2 pins every optional PES header field beyond the PTS to
//! absent. Each one must be rejected when it is the *only* field present, not
//! just when several are: the check is a chain of `||`s, so a single flipped
//! one would otherwise let that field through and the packet would reserialize
//! without it (no longer byte-exact).

use mpeg_pes::{
    AdditionalCopyInfo, Dts, EsRate, Escr, PesExtension, PesHeader, PesPacket, Pts, StreamId,
    TrickMode,
};
use st291::{AncDataPacket, Error};

const ANC_STREAM_ID: u8 = 0xBD;

fn base_header() -> PesHeader<'static> {
    let mut header = PesHeader::new();
    header.data_alignment_indicator = true;
    header.pts = Some(Pts::try_new(0x1_2345_6789).unwrap());
    header
}

fn fixture_payload() -> Vec<u8> {
    let data = test_fixtures::read("rtp/anc.bin");
    mpeg_pes::PesPacket::parse(&data).unwrap().payload.to_vec()
}

fn pes_with(payload: &[u8], header: PesHeader<'_>) -> Vec<u8> {
    let mut packet = PesPacket::new(StreamId::new(ANC_STREAM_ID), payload);
    packet.header = Some(header);
    let mut out = vec![0u8; packet.serialized_len()];
    packet.serialize_into(&mut out).unwrap();
    out
}

#[test]
fn the_unmodified_header_still_parses() {
    let payload = fixture_payload();
    let buf = pes_with(&payload, base_header());
    assert!(AncDataPacket::parse(&buf).is_ok());
}

#[test]
fn each_forbidden_header_field_is_rejected_on_its_own() {
    let payload = fixture_payload();
    let mut variants: Vec<(&'static str, PesHeader<'static>)> = Vec::new();

    let mut h = base_header();
    h.dts = Some(Dts::try_new(0x1_2345_6789).unwrap());
    variants.push(("DTS", h));

    let mut h = base_header();
    h.escr = Some(Escr::from_27mhz(27_000_000).unwrap());
    variants.push(("ESCR", h));

    let mut h = base_header();
    h.es_rate = Some(EsRate::try_new(1_000).unwrap());
    variants.push(("ES_rate", h));

    let mut h = base_header();
    h.dsm_trick_mode = Some(TrickMode::SlowMotion { rep_cntrl: 1 });
    variants.push(("DSM_trick_mode", h));

    let mut h = base_header();
    h.additional_copy_info = Some(AdditionalCopyInfo::new(0x12));
    variants.push(("additional_copy_info", h));

    let mut h = base_header();
    h.pes_crc = Some(0x1234);
    variants.push(("PES_CRC", h));

    let mut h = base_header();
    h.pes_extension = Some(PesExtension::new());
    variants.push(("PES_extension", h));

    let mut h = base_header();
    h.header_stuffing = &[0xFF];
    variants.push(("header stuffing", h));

    for (what, header) in variants {
        let buf = pes_with(&payload, header);
        match AncDataPacket::parse(&buf) {
            Err(Error::ReservedBits(rb)) => assert_eq!(
                rb.field, "ST 2038 optional PES header fields beyond the PTS",
                "{what} must be rejected on its own"
            ),
            other => panic!("{what}: expected ReservedBits, got {other:?}"),
        }
    }
}

#[test]
fn missing_data_alignment_indicator_is_rejected_with_expected_one() {
    let payload = fixture_payload();
    let mut h = base_header();
    h.data_alignment_indicator = false;
    let buf = pes_with(&payload, h);
    match AncDataPacket::parse(&buf) {
        Err(Error::ReservedBits(rb)) => {
            assert_eq!(rb.field, "data_alignment_indicator");
            assert_eq!(rb.expected, 1);
            assert_eq!(rb.found, 0);
        }
        other => panic!("expected ReservedBits for data_alignment_indicator, got {other:?}"),
    }
}
