//! `SiMux`/`SectionPacketiser` caller-facing API: a PMT registered with
//! `upsert_pmt` is emitted on its PID, `upsert_pmt` refuses another table, the
//! `usize` packet counts are asserted, the datagram-size constants are pinned,
//! and every `SectionRejection` label is exact.

use core::time::Duration;

use broadcast_common::stage::Timestamp;
use broadcast_core::Parse;
use mpeg_ts::mux::{
    MIN_SECTION_INTERVAL, SectionInterval, SectionPacketiser, SectionRejection, SiMux,
};
use mpeg_ts::pid::Pid;
use mpeg_ts::section::Section;
use mpeg_ts::ts::{SectionReassembler, TS_PACKET_SIZE};
use mpeg_ts::{TS_BYTES_PER_DATAGRAM, TS_PACKETS_PER_DATAGRAM};

fn long_form(table_id: u8, extension_id: u16, payload: &[u8]) -> Vec<u8> {
    let section_length = 5 + payload.len() + 4;
    let mut b = vec![0u8; 3];
    b[0] = table_id;
    b[1] = 0x80 | 0x30 | ((section_length >> 8) & 0x0F) as u8;
    b[2] = (section_length & 0xFF) as u8;
    b.push((extension_id >> 8) as u8);
    b.push((extension_id & 0xFF) as u8);
    b.push(0xC1);
    b.push(0x00);
    b.push(0x00);
    b.extend_from_slice(payload);
    let crc = broadcast_common::crc32_mpeg2::compute(&b);
    b.extend_from_slice(&crc.to_be_bytes());
    b
}

#[test]
fn datagram_sizing_constants_are_the_wire_values() {
    assert_eq!(TS_PACKETS_PER_DATAGRAM, 7);
    assert_eq!(TS_BYTES_PER_DATAGRAM, 1316);
    assert_eq!(mpeg_ts::MAX_DATAGRAM_PAYLOAD, 1472);
    assert_eq!(MIN_SECTION_INTERVAL, Duration::from_millis(25));
}

#[test]
fn upsert_pmt_emits_the_registered_section_on_its_pid() {
    let pmt = long_form(0x02, 0x0001, &[0xE1, 0x00, 0xF0, 0x00]);
    let expected = Section::parse(&pmt)
        .expect("parse")
        .serialize()
        .expect("serialize");

    let mut mux = SiMux::new();
    mux.upsert_pmt(Pid::from_const(0x100), pmt)
        .expect("PMT accepted");

    let mut out = Vec::new();
    let written = mux.poll_into(Timestamp::ZERO, &mut out);
    assert_eq!(written, out.len());
    assert_eq!(out.len(), 1);
    assert_eq!(out[0][1] & 0x1F, 0x01);
    assert_eq!(out[0][2], 0x00);

    let mut r = SectionReassembler::default();
    for pkt in &out {
        let pusi = pkt[1] & 0x40 != 0;
        r.feed(&pkt[4..], pusi);
    }
    let got = r.pop_section().expect("one section");
    assert_eq!(&got[..], &expected[..]);
}

#[test]
fn upsert_pmt_rejects_a_section_of_another_table() {
    let pat = long_form(0x00, 0x0001, &[]);
    let mut mux = SiMux::new();
    assert_eq!(
        mux.upsert_pmt(Pid::from_const(0x100), pat),
        Err(SectionRejection::WrongTableId {
            expected: 0x02,
            found: Some(0x00),
        })
    );
    assert_eq!(mux.rejected().len(), 1);
}

#[test]
fn packetise_into_and_poll_into_return_the_packets_they_appended() {
    let pmt = long_form(0x02, 0x0001, &[0xE1, 0x00, 0xF0, 0x00]);
    let section = Section::parse(&pmt)
        .expect("parse")
        .serialize()
        .expect("serialize");

    let mut packetiser = SectionPacketiser::new(Pid::from_const(0x200));
    let mut out = vec![[0u8; TS_PACKET_SIZE]; 3];
    let n = packetiser.packetise_into(&[&section], &mut out);
    assert_eq!(n, 1);
    assert_eq!(n, out.len());
    assert_eq!(packetiser.continuity_counter().value(), 1);

    let mut mux = SiMux::new();
    mux.upsert_pmt(Pid::from_const(0x200), pmt)
        .expect("PMT accepted");
    let mut polled = Vec::new();
    let n = mux.poll_into(Timestamp::ZERO, &mut polled);
    assert_eq!(n, 1);
    assert_eq!(n, polled.len());
}

#[test]
fn section_interval_too_short_names_both_durations() {
    let err = SectionInterval::try_new(Duration::from_millis(1)).expect_err("below the floor");
    assert_eq!(
        err.to_string(),
        "section interval 1ms is below the 25ms minimum (EN 300 468 §5.1.4.1)"
    );
}

#[test]
fn section_rejection_labels_are_pinned() {
    let cases = [
        (SectionRejection::NotASection, "not_a_section"),
        (
            SectionRejection::MoreThanOneSection,
            "more_than_one_section",
        ),
        (SectionRejection::DuplicateSection, "duplicate_section"),
        (SectionRejection::Malformed, "malformed"),
        (
            SectionRejection::WrongTableId {
                expected: 0x42,
                found: None,
            },
            "wrong_table_id",
        ),
    ];
    for (rejection, label) in cases {
        assert_eq!(rejection.name(), label);
        assert_eq!(rejection.to_string(), label);
    }
}
