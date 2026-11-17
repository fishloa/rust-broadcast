//! `OwnedTsPacket` mutators and accessors: continuity-counter replacement,
//! OPCR rewrite with preserved reserved bits, the boolean accessors' `false`
//! cases, the payload offset at the packet end, and `adaptation_field()`.

use mpeg_ts::owned::OwnedTsPacket;
use mpeg_ts::ts::{
    ADAPTATION_FLAG, ContinuityCounter, PAYLOAD_FLAG, Pcr, TS_PACKET_SIZE, TS_SYNC_BYTE, TsPacket,
};

const AF_PCR_FLAG: u8 = 0x10;
const AF_OPCR_FLAG: u8 = 0x08;

fn raw_packet(pid: u16, byte3: u8) -> [u8; TS_PACKET_SIZE] {
    let mut raw = [0u8; TS_PACKET_SIZE];
    raw[0] = TS_SYNC_BYTE;
    raw[1] = ((pid >> 8) as u8) & 0x1F;
    raw[2] = (pid & 0xFF) as u8;
    raw[3] = byte3;
    raw
}

fn cc(value: u8) -> ContinuityCounter {
    ContinuityCounter::try_new(value).expect("4-bit counter")
}

#[test]
fn set_continuity_counter_replaces_only_the_low_nibble() {
    let mut raw = raw_packet(0x100, 0xBF);
    OwnedTsPacket::set_continuity_counter(&mut raw, cc(3));
    assert_eq!(raw[3], 0xB3);

    let mut raw = raw_packet(0x100, 0x1F);
    OwnedTsPacket::set_continuity_counter(&mut raw, cc(0));
    assert_eq!(raw[3], 0x10);
}

#[test]
fn set_continuity_counter_is_visible_through_the_parsed_header() {
    let mut raw = raw_packet(0x101, 0x1D);
    OwnedTsPacket::set_continuity_counter(&mut raw, cc(9));
    let pkt = TsPacket::parse(&raw).expect("parse");
    assert_eq!(pkt.header.continuity_counter.value(), 9);
    assert_eq!(pkt.header.pid.value(), 0x101);
    assert_eq!(
        pkt.header.scrambling,
        mpeg_ts::ts::ScramblingControl::NotScrambled
    );
}

fn packet_with_pcr_and_opcr() -> [u8; TS_PACKET_SIZE] {
    let mut raw = raw_packet(0x102, ADAPTATION_FLAG | PAYLOAD_FLAG);
    raw[4] = 13;
    raw[5] = AF_PCR_FLAG | AF_OPCR_FLAG;
    raw[6..12].copy_from_slice(&[0x11, 0x22, 0x33, 0x44, 0x7E, 0x55]);
    raw[12..18].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0x00, 0xEE]);
    raw[48] = 0x7E;
    raw
}

#[test]
fn set_opcr_preserves_the_pcr_and_the_opcr_reserved_bits() {
    let mut raw = packet_with_pcr_and_opcr();
    let new_opcr = Pcr::from_parts(0x1234_5678, 0x123).expect("in range");
    OwnedTsPacket::set_opcr(&mut raw, new_opcr).expect("opcr slot present");

    assert_eq!(&raw[6..12], &[0x11, 0x22, 0x33, 0x44, 0x7E, 0x55]);
    assert_eq!(raw[16] & 0x7E, 0x00);

    let pkt = TsPacket::parse(&raw).expect("parse");
    let af = pkt
        .adaptation_field()
        .expect("adaptation field")
        .expect("valid");
    assert_eq!(af.opcr, Some(new_opcr));
    assert_eq!(af.opcr_reserved, 0);
    assert_eq!(
        af.pcr.expect("pcr").base(),
        (0x11u64 << 25) | (0x22u64 << 17) | (0x33u64 << 9) | (0x44u64 << 1)
    );
}

#[test]
fn boolean_accessors_report_the_clear_cases() {
    let raw = raw_packet(0x103, ADAPTATION_FLAG);
    let owned = OwnedTsPacket::parse(raw).expect("parse");
    assert!(!owned.pusi());
    assert!(!owned.tei());
    assert!(!owned.has_payload());
    assert!(owned.has_adaptation());
    assert_eq!(owned.continuity_counter().value(), 0);
}

#[test]
fn payload_ends_exactly_at_the_packet_end() {
    let mut raw = raw_packet(0x104, ADAPTATION_FLAG | PAYLOAD_FLAG);
    raw[4] = 183;
    let owned = OwnedTsPacket::parse(raw).expect("parse");
    assert!(owned.payload().is_none());
    let mut owned = owned;
    assert!(owned.payload_mut().is_none());

    let mut raw = raw_packet(0x104, ADAPTATION_FLAG | PAYLOAD_FLAG);
    raw[4] = 182;
    raw[187] = 0x5A;
    let mut owned = OwnedTsPacket::parse(raw).expect("parse");
    assert_eq!(owned.payload(), Some(&[0x5A][..]));
    let payload = owned.payload_mut().expect("one payload byte");
    assert_eq!(payload.len(), 1);
    payload[0] = 0xA5;
    assert_eq!(owned.raw()[187], 0xA5);
}

#[test]
fn adaptation_field_is_some_for_a_present_field_and_none_without_one() {
    let mut raw = raw_packet(0x105, ADAPTATION_FLAG);
    raw[4] = 1;
    raw[5] = 0x00;
    let owned = OwnedTsPacket::parse(raw).expect("parse");
    assert!(matches!(owned.adaptation_field(), Some(Ok(_))));

    let raw = raw_packet(0x105, PAYLOAD_FLAG);
    let owned = OwnedTsPacket::parse(raw).expect("parse");
    assert!(owned.adaptation_field().is_none());
}
