//! `UleReceiver` reassembly across TS packets (RFC 4326 §6/§7): an SNDU that
//! spans several packets, a header split at a packet boundary, packed follow-on
//! SNDUs, and the `PUSI=0` leftover rules (ULE-W4, #1120).

use ule::{Sndu, TypeField, UleReceiver};

fn make_sndu(pdu: &[u8]) -> Vec<u8> {
    let sndu = Sndu::new(TypeField::EtherType(0x0800), None, pdu);
    let mut bytes = vec![0u8; sndu.serialized_len()];
    sndu.serialize_into(&mut bytes).unwrap();
    bytes
}

#[test]
fn a_400_byte_sndu_reassembles_across_three_packets() {
    let pdu = vec![0x5A; 392];
    let sndu = make_sndu(&pdu);
    assert_eq!(sndu.len(), 400);

    let mut rx = UleReceiver::new();
    let mut p1 = vec![0x00];
    p1.extend_from_slice(&sndu[..183]);
    assert!(rx.push(&p1, true).is_empty(), "still reassembling");
    assert!(rx.in_reassembly());

    assert!(
        rx.push(&sndu[183..367], false).is_empty(),
        "still reassembling"
    );
    assert!(rx.in_reassembly());

    let done = rx.push(&sndu[367..], false);
    assert_eq!(done, vec![sndu.clone()]);
    assert!(!rx.in_reassembly());
    assert_eq!(Sndu::parse(&done[0]).unwrap().pdu(), &pdu[..]);
}

#[test]
fn a_header_split_one_byte_before_the_packet_boundary_completes() {
    let pdu = [0x11u8; 4];
    let sndu = make_sndu(&pdu);
    assert_eq!(sndu.len(), 12);

    let mut rx = UleReceiver::new();
    let mut p1 = vec![0x00];
    p1.push(sndu[0]);
    assert!(rx.push(&p1, true).is_empty());
    assert!(rx.in_reassembly());

    let done = rx.push(&sndu[1..], false);
    assert_eq!(done, vec![sndu]);
}

#[test]
fn an_empty_continuation_does_not_finish_a_single_byte_header() {
    let mut rx = UleReceiver::new();
    let mut p1 = vec![0x00];
    p1.push(0x80);
    assert!(rx.push(&p1, true).is_empty());
    assert!(rx.in_reassembly());

    assert!(rx.push(&[], false).is_empty());
    assert!(rx.in_reassembly(), "the header is still incomplete");
}

#[test]
fn a_payload_pointer_at_the_end_of_the_region_completes_the_sndu() {
    let pdu = vec![0x33u8; 46];
    let sndu = make_sndu(&pdu);
    assert_eq!(sndu.len(), 54);

    let mut rx = UleReceiver::new();
    let mut p1 = vec![0x00];
    p1.extend_from_slice(&sndu[..20]);
    assert!(rx.push(&p1, true).is_empty());

    let rest = &sndu[20..];
    let mut p2 = vec![rest.len() as u8];
    p2.extend_from_slice(rest);
    let done = rx.push(&p2, true);
    assert_eq!(done, vec![sndu], "PP points at the end, no new SNDU starts");
}

#[test]
fn a_complete_sndu_and_a_packed_follow_on_both_emit() {
    let a = make_sndu(&[0xAA; 5]);
    let b = make_sndu(&[0xBB; 7]);

    let mut p = vec![0x00];
    p.extend_from_slice(&a);
    p.extend_from_slice(&b);
    p.extend_from_slice(&[0xFF, 0xFF, 0xFF]);

    let mut rx = UleReceiver::new();
    let done = rx.push(&p, true);
    assert_eq!(done, vec![a, b]);
}

#[test]
fn a_pusi0_completion_followed_by_padding_leaves_the_receiver_idle() {
    let sndu = make_sndu(&[0x11u8; 10]);
    let split = sndu.len() / 2;

    let mut rx = UleReceiver::new();
    let mut p1 = vec![0x00];
    p1.extend_from_slice(&sndu[..split]);
    assert!(rx.push(&p1, true).is_empty());

    let mut p2 = sndu[split..].to_vec();
    p2.extend_from_slice(&[0xFF, 0xFF]);
    let done = rx.push(&p2, false);
    assert_eq!(done, vec![sndu]);
    assert!(!rx.in_reassembly());
}

#[test]
fn a_pusi0_completion_with_junk_discards_it_and_the_next_pusi_starts_fresh() {
    let sndu = make_sndu(&[0x11u8; 10]);
    let split = sndu.len() / 2;

    let mut rx = UleReceiver::new();
    let mut p1 = vec![0x00];
    p1.extend_from_slice(&sndu[..split]);
    assert!(rx.push(&p1, true).is_empty());

    let mut p2 = sndu[split..].to_vec();
    p2.extend_from_slice(&[0x12, 0x34]);
    let done = rx.push(&p2, false);
    assert_eq!(done, vec![sndu]);
    assert!(!rx.in_reassembly(), "the junk is not a new partial SNDU");

    let b = make_sndu(&[0x22u8; 4]);
    let mut p3 = vec![0x00];
    p3.extend_from_slice(&b);
    p3.push(0xFF);
    assert_eq!(rx.push(&p3, true), vec![b]);
}

#[test]
fn a_four_byte_region_is_a_header_not_a_straddle() {
    // D=1, Length=0: the framing layer walks exactly the 4-byte base header
    // rather than buffering it as a header that only straddles the boundary.
    let region = [0x80u8, 0x00, 0x08, 0x00];
    let mut payload = vec![0x00];
    payload.extend_from_slice(&region);

    let mut rx = UleReceiver::new();
    let done = rx.push(&payload, true);
    assert_eq!(done, vec![region.to_vec()]);
    assert!(!rx.in_reassembly());
}
