//! Serialize/parse consistency of the DVB-TA stream-event payload and the
//! compact encrypted-packet CRC (adversarial review, signalling-captions).

use broadcast_common::{Parse, Serialize};
use scte35_splice::dvb_ta::{CompactSpliceInsert, Scte35Carriage, StreamEventPayload, TimelineType};

const NAME: &[u8] = b"dvb://1.2.3/x";

fn payload(
    timeline_type: TimelineType,
    temi: Option<(u8, u8)>,
    reserved: &[u8],
) -> StreamEventPayload<'_> {
    StreamEventPayload {
        timeline_type,
        temi,
        reserved_dvb_data: reserved,
        private_data: None,
        carriage: Scte35Carriage::CarouselObject(NAME),
    }
}

#[test]
fn reserved_dvb_data_after_temi_round_trips() {
    let p = payload(TimelineType::Temi, Some((7, 9)), &[0xAA, 0xBB, 0xCC]);
    let bytes = p.try_to_bytes().unwrap();
    // DVB_data_length = flags(1) + temi(2) + 3 reserved bytes.
    assert_eq!(bytes[0], 6);
    assert_eq!(&bytes[4..7], &[0xAA, 0xBB, 0xCC]);
    assert_eq!(StreamEventPayload::parse(&bytes).unwrap(), p);
}

#[test]
fn reserved_timeline_aliasing_temi_needs_temi_fields() {
    // Reserved(2) writes the TEMI wire value, so the temi bytes are required.
    assert!(
        payload(TimelineType::Reserved(2), None, &[])
            .try_to_bytes()
            .is_err()
    );
    let ok = payload(TimelineType::Reserved(2), Some((1, 2)), &[]);
    let bytes = ok.try_to_bytes().unwrap();
    assert_eq!(
        StreamEventPayload::parse(&bytes).unwrap().timeline_type,
        TimelineType::Temi
    );
}

#[test]
fn in_range_reserved_timeline_still_round_trips() {
    let p = payload(TimelineType::Reserved(0xF), None, &[]);
    let bytes = p.try_to_bytes().unwrap();
    assert_eq!(StreamEventPayload::parse(&bytes).unwrap(), p);
}

#[test]
fn compact_splice_insert_e_crc_must_match_encrypted_flag() {
    let mut si = CompactSpliceInsert::parse(&{
        let base = CompactSpliceInsert {
            encrypted_packet: false,
            encryption_algorithm: 0,
            cw_index: 0,
            pts_time: 1,
            splice_event_id: 1,
            duration: 1,
            unique_program_id: 1,
            avail_num: 0,
            avails_expected: 0,
            reserved: scte35_splice::dvb_ta::SI_RESERVED_DEFAULT,
            das: None,
            e_crc_32: None,
        };
        base.try_to_bytes().unwrap()
    })
    .unwrap();
    si.e_crc_32 = Some(0xDEAD_BEEF); // not encrypted: a surplus CRC
    assert!(si.try_to_bytes().is_err());
    si.encrypted_packet = true;
    let bytes = si.try_to_bytes().unwrap();
    assert_eq!(CompactSpliceInsert::parse(&bytes).unwrap(), si);
    si.e_crc_32 = None; // encrypted: a missing CRC
    assert!(si.try_to_bytes().is_err());
}
