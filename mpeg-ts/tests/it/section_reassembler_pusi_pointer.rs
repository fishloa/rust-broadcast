//! `SectionReassembler` boundaries: the `pointer_field` tail that completes a
//! section spanning into a PUSI packet (ISO/IEC 13818-1 §2.4.4), the
//! `MAX_SECTION_SIZE` guards, a header split 1+2 across packets, and
//! `extract_ts_payload`'s exact-fit payload start.

use mpeg_ts::ts::{
    ContinuityCounter, SectionReassembler, TS_PACKET_SIZE, TS_SYNC_BYTE, extract_ts_payload,
};

fn section(total: usize, table_id: u8, fill: u8) -> Vec<u8> {
    assert!(
        total >= 4,
        "a short-form section needs a header and one byte"
    );
    let mut s = vec![0u8; total];
    s[0] = table_id;
    let section_length = total - 3;
    s[1] = ((section_length >> 8) as u8) & 0x0F;
    s[2] = (section_length & 0xFF) as u8;
    for b in &mut s[3..] {
        *b = fill;
    }
    s
}

fn drain(r: &mut SectionReassembler) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(s) = r.pop_section() {
        out.push(s.to_vec());
    }
    out
}

#[test]
fn pointer_field_tail_completes_section_spanning_into_pusi_packet() {
    let first = section(300, 0x70, 0x11);
    let second = section(20, 0x71, 0x22);

    let mut r = SectionReassembler::default();
    let mut a = vec![0x00u8];
    a.extend_from_slice(&first[..183]);
    r.feed(&a, true);
    assert!(r.pop_section().is_none(), "300-byte section is incomplete");
    assert_eq!(r.len(), 183);
    assert!(!r.is_empty());

    let mut b = vec![117u8];
    b.extend_from_slice(&first[183..]);
    b.extend_from_slice(&second);
    r.feed(&b, true);

    let got = drain(&mut r);
    assert_eq!(got.len(), 2, "both sections are complete");
    assert_eq!(got[0], first);
    assert_eq!(got[1], second);
    assert!(r.is_empty());
    assert_eq!(r.len(), 0);
}

#[test]
fn pointer_past_available_payload_drops_the_in_progress_section() {
    let partial = section(300, 0x72, 0x33);
    let mut r = SectionReassembler::default();
    let mut a = vec![0x00u8];
    a.extend_from_slice(&partial[..50]);
    r.feed(&a, true);
    assert_eq!(r.len(), 50);

    let mut b = vec![200u8];
    b.extend_from_slice(&[0xAAu8; 99]);
    assert_eq!(b.len(), 100);
    r.feed(&b, true);

    assert!(r.pop_section().is_none(), "the in-progress section is lost");
    assert!(r.is_empty());
    assert_eq!(r.len(), 0);
}

#[test]
fn pointer_field_with_empty_buffer_does_not_decode_its_tail_as_a_section() {
    let bogus = section(20, 0x73, 0x44);
    let real = section(20, 0x74, 0x55);
    let mut payload = vec![20u8];
    payload.extend_from_slice(&bogus);
    payload.extend_from_slice(&real);

    let mut r = SectionReassembler::default();
    r.feed(&payload, true);

    let got = drain(&mut r);
    assert_eq!(
        got.len(),
        1,
        "only the new section after 1 + pointer starts"
    );
    assert_eq!(got[0], real);
}

#[test]
fn header_split_one_plus_two_across_packets_is_completed_in_a_continuation() {
    let full = section(30, 0x75, 0x66);
    let mut r = SectionReassembler::default();
    let mut first = vec![0x00u8];
    first.extend_from_slice(&full[..1]);
    r.feed(&first, true);
    assert_eq!(r.len(), 1);
    assert!(!r.is_empty());

    r.feed(&full[1..], false);
    let got = drain(&mut r);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0], full);
    assert!(r.is_empty());
}

#[test]
fn a_complete_maximum_section_is_kept_when_the_pointer_tail_reaches_the_ceiling() {
    let full = section(4098, 0x76, 0x77);
    assert_eq!(full.len(), 4098);

    let mut r = SectionReassembler::default();
    let mut head = vec![0x00u8];
    head.extend_from_slice(&full[..183]);
    r.feed(&head, true);
    let mut fed = 183usize;
    while fed < 3915 {
        let end = (fed + 184).min(3915);
        r.feed(&full[fed..end], false);
        fed = end;
    }
    assert_eq!(r.len(), 3915);

    let mut tail = vec![183u8];
    tail.extend_from_slice(&full[3915..]);
    assert_eq!(tail.len(), 184);
    r.feed(&tail, true);

    let got = drain(&mut r);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0], full);
    assert!(r.is_empty());
}

#[test]
fn a_pointer_tail_that_would_exceed_the_ceiling_discards_the_partial_section() {
    let full = section(4098, 0x78, 0x88);

    let mut r = SectionReassembler::default();
    let mut head = vec![0x00u8];
    head.extend_from_slice(&full[..183]);
    r.feed(&head, true);
    let mut fed = 183usize;
    while fed < 4097 {
        let end = (fed + 184).min(4097);
        r.feed(&full[fed..end], false);
        fed = end;
    }
    assert_eq!(r.len(), 4097);

    let mut tail = vec![2u8];
    tail.extend_from_slice(&full[4097..]);
    tail.push(0x00);
    assert_eq!(tail.len(), 3);
    r.feed(&tail, true);

    assert!(r.pop_section().is_none());
    assert!(r.is_empty());
}

#[test]
fn a_pusi_payload_holding_exactly_the_maximum_section_is_accepted() {
    let full = section(4098, 0x79, 0x99);
    let mut payload = vec![0x00u8];
    payload.extend_from_slice(&full);

    let mut r = SectionReassembler::default();
    r.feed(&payload, true);
    let got = drain(&mut r);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0], full);
}

#[test]
fn a_pusi_payload_above_the_maximum_section_is_ignored() {
    let full = section(4098, 0x7A, 0xAA);
    let mut payload = vec![0x00u8];
    payload.extend_from_slice(&full);
    payload.extend_from_slice(&[0xBBu8; 100]);

    let mut r = SectionReassembler::default();
    r.feed(&payload, true);
    assert!(r.pop_section().is_none());
    assert!(r.is_empty());
}

#[test]
fn a_continuation_with_an_incomplete_header_is_bounded_by_the_section_ceiling() {
    let full = section(4098, 0x7B, 0xCC);
    let mut r = SectionReassembler::default();
    let mut first = vec![0x00u8];
    first.extend_from_slice(&full[..2]);
    r.feed(&first, true);
    assert_eq!(r.len(), 2);

    let mut continuation = vec![0u8; 5000];
    continuation[..4096].copy_from_slice(&full[2..]);
    r.feed(&continuation, false);

    let got = drain(&mut r);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0], full);
    assert!(r.is_empty());
}

#[test]
fn a_same_continuity_counter_packet_after_a_discontinuity_is_not_a_duplicate() {
    let sec = section(30, 0x7C, 0xDD);
    let cc = ContinuityCounter::try_new(5).expect("4-bit");
    let mut r = SectionReassembler::default();

    let mut start = vec![0x00u8];
    start.extend_from_slice(&sec[..10]);
    r.feed_with_continuity(&start, true, cc, false);
    assert!(r.pop_section().is_none());

    r.feed_with_continuity(&sec[10..], false, cc, true);
    let got = drain(&mut r);
    assert_eq!(got.len(), 1, "discontinuity permits the repeated counter");
    assert_eq!(got[0], sec);

    let next = section(30, 0x7D, 0xEE);
    let next_cc = ContinuityCounter::try_new(6).expect("4-bit");
    let mut start = vec![0x00u8];
    start.extend_from_slice(&next[..10]);
    r.feed_with_continuity(&start, true, next_cc, false);
    assert_eq!(r.len(), 10);

    r.feed_with_continuity(&next[10..], false, next_cc, false);
    assert!(r.pop_section().is_none(), "a true duplicate is dropped");
    assert_eq!(r.len(), 10);
}

fn packet_with_adaptation(af_len: u8, afc: u8) -> [u8; TS_PACKET_SIZE] {
    let mut pkt = [0u8; TS_PACKET_SIZE];
    pkt[0] = TS_SYNC_BYTE;
    pkt[1] = 0x01;
    pkt[2] = 0x00;
    pkt[3] = afc;
    pkt[4] = af_len;
    pkt
}

#[test]
fn extract_ts_payload_rejects_a_payload_start_at_the_packet_end() {
    let full = packet_with_adaptation(183, 0x30);
    assert!(extract_ts_payload(&full).is_none());

    let mut one = packet_with_adaptation(182, 0x30);
    one[187] = 0x5A;
    assert_eq!(extract_ts_payload(&one), Some(&[0x5A][..]));
}

#[test]
fn extract_ts_payload_of_a_three_byte_slice_is_none() {
    assert!(extract_ts_payload(&[TS_SYNC_BYTE, 0x00, 0x00]).is_none());
}
