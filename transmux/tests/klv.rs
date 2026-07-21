//! KLV codec + KLV-over-RTP integration tests (#478).
//!
//! Sources: SMPTE ST 336 framing (via MISB ST 0601 + RFC 6597), MISB ST 0601
//! UAS Datalink Local Set, RFC 6597 KLV-over-RTP. See
//! `transmux/docs/klv/klv-misb0601.md`.

use broadcast_common::{Parse, Serialize};
use transmux::klv::{
    CHECKSUM_LEN, KlvItem, LocalSetItem, PRECISION_TIMESTAMP_LEN, TAG_CHECKSUM,
    TAG_PRECISION_TIMESTAMP, UAS_LS_KEY, UNIVERSAL_LABEL_LEN, UasLocalSet, ber_length, crc16_ccitt,
    encode_ber_length,
};
use transmux::rtp::{depacketise_klv, packetise_klv};

const RTP_HEADER_LEN: usize = 12;
const RTP_MARKER_MASK: u8 = 0x80;

// ---------------------------------------------------------------------------
// 1. BER length round-trip (bites)
// ---------------------------------------------------------------------------

#[test]
fn ber_length_short_form() {
    // Short form: length 5 encodes as the single byte 0x05.
    assert_eq!(encode_ber_length(5), vec![0x05]);
    let (len, consumed) = ber_length(&[0x05, 0xFF, 0xFF]).unwrap();
    assert_eq!((len, consumed), (5, 1));
}

#[test]
fn ber_length_long_form_300() {
    // 300 needs 2 length bytes → 82 01 2C.
    let enc = encode_ber_length(300);
    assert_eq!(enc, vec![0x82, 0x01, 0x2C]);
    let (len, consumed) = ber_length(&enc).unwrap();
    assert_eq!((len, consumed), (300, 3));

    // The 2-length-byte boundary bites: 255 fits in 1 byte, 256 needs 2.
    assert_eq!(encode_ber_length(255), vec![0x81, 0xFF]);
    assert_eq!(encode_ber_length(256), vec![0x82, 0x01, 0x00]);
    assert_eq!(ber_length(&[0x82, 0x01, 0x00]).unwrap().0, 256);
}

#[test]
fn ber_length_mutation_bites() {
    // Mutating the encoded length must change the decoded value: this proves the
    // decoder reads the value bytes, not a fixed position.
    let mut enc = encode_ber_length(300);
    let original = ber_length(&enc).unwrap().0;
    enc[2] ^= 0x01; // 0x2C -> 0x2D
    let mutated = ber_length(&enc).unwrap().0;
    assert_ne!(original, mutated);
    assert_eq!(mutated, 301);
}

#[test]
fn ber_length_indefinite_form_rejected() {
    // 0x80 alone (indefinite form) is not permitted in KLV → error, no panic.
    assert!(ber_length(&[0x80]).is_err());
    // Long form promising bytes that aren't there → error, no panic.
    assert!(ber_length(&[0x82, 0x01]).is_err());
}

// ---------------------------------------------------------------------------
// 2. KLV item round-trip + computed length
// ---------------------------------------------------------------------------

#[test]
fn klv_item_round_trip() {
    let item = KlvItem::new(UAS_LS_KEY, b"hello KLV value".to_vec());
    let bytes = item.to_bytes();
    let parsed = KlvItem::parse(&bytes).unwrap();
    assert_eq!(parsed, item);
    // serialize -> parse -> equal, and the byte length is exactly serialized_len.
    assert_eq!(bytes.len(), item.serialized_len());
}

#[test]
fn klv_item_length_is_computed_not_stored() {
    // The serialized length byte is COMPUTED from the value: mutate the value
    // length and the on-wire length field must track it.
    let short = KlvItem::new(UAS_LS_KEY, vec![0xAB; 5]);
    let long = KlvItem::new(UAS_LS_KEY, vec![0xAB; 6]);
    let sb = short.to_bytes();
    let lb = long.to_bytes();
    // Byte at index 16 is the (short-form) BER length.
    assert_eq!(sb[UNIVERSAL_LABEL_LEN], 5);
    assert_eq!(lb[UNIVERSAL_LABEL_LEN], 6);

    // A value that forces a 2-byte BER length (>= 128).
    let big = KlvItem::new(UAS_LS_KEY, vec![0xCD; 300]);
    let bb = big.to_bytes();
    assert_eq!(
        &bb[UNIVERSAL_LABEL_LEN..UNIVERSAL_LABEL_LEN + 3],
        &[0x82, 0x01, 0x2C]
    );
    assert_eq!(KlvItem::parse(&bb).unwrap(), big);
}

#[test]
fn local_set_variable_item_not_last_parses() {
    // A >=2-item Local Set where a VARIABLE-length item is NOT last: this is the
    // real boundary the parser must walk (length-driven, not fixed offsets).
    // Item A: tag 3, 4-byte value (variable). Item B: tag 5, 2-byte value.
    let items = vec![
        LocalSetItem::new(3, vec![0xDE, 0xAD, 0xBE, 0xEF]),
        LocalSetItem::new(5, vec![0x01, 0x02]),
    ];
    // Wrap in a UAS LS (round-trip through serialize/parse), then check the
    // non-checksum items survived in order with exact values.
    let ls = UasLocalSet::from_items(items.clone());
    let bytes = ls.serialize_with_checksum();
    let parsed = UasLocalSet::parse(&bytes).unwrap();
    let non_checksum: Vec<_> = parsed
        .items
        .iter()
        .filter(|i| i.tag != TAG_CHECKSUM)
        .cloned()
        .collect();
    assert_eq!(non_checksum, items);
}

// ---------------------------------------------------------------------------
// 3. UAS Local Set + checksum (hand-computed CRC vector)
// ---------------------------------------------------------------------------

#[test]
fn uas_local_set_checksum_vector_and_verify() {
    // Tag 2 (Precision Time Stamp) = 2024-01-01T00:00:00Z = 1_704_067_200_000_000 µs.
    let ts: u64 = 1_704_067_200_000_000;
    let ls = UasLocalSet::from_items(vec![LocalSetItem::new(
        TAG_PRECISION_TIMESTAMP,
        ts.to_be_bytes().to_vec(),
    )]);

    let packet = ls.serialize_with_checksum();

    // Full packet is exactly 31 bytes (16 UL + 1 len + 10 tag2 + 4 tag1).
    assert_eq!(packet.len(), 31);
    // Hand-computed expected bytes (see docs; CRC-16/CCITT over bytes[..29]).
    let expected: [u8; 31] = [
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00,
        0x00, // UL
        0x0E, // BER length 14
        0x02, 0x08, 0x00, 0x06, 0x0D, 0xD7, 0x10, 0x21, 0x20, 0x00, // tag 2, len 8, ts
        0x01, 0x02, 0x08, 0x91, // tag 1, len 2, CRC = 0x0891
    ];
    assert_eq!(packet.as_slice(), &expected);

    // The hand-computed CRC-16/CCITT value.
    let split = packet.len() - CHECKSUM_LEN;
    assert_eq!(crc16_ccitt(&packet[..split]), 0x0891);

    // Round-trip parse: timestamp reads back, checksum verifies.
    let parsed = UasLocalSet::parse(&packet).unwrap();
    assert_eq!(parsed.precision_timestamp(), Some(ts));
    assert_eq!(parsed.stored_checksum(), Some(0x0891));
    assert!(UasLocalSet::verify_checksum(&packet).unwrap());

    // Corrupt a value byte → checksum fails (bites).
    let mut bad = packet.clone();
    bad[20] ^= 0xFF; // a timestamp byte
    assert!(!UasLocalSet::verify_checksum(&bad).unwrap());
    // Corrupt a CRC byte → also fails.
    let mut bad_crc = packet.clone();
    let last = bad_crc.len() - 1;
    bad_crc[last] ^= 0x01;
    assert!(!UasLocalSet::verify_checksum(&bad_crc).unwrap());
}

#[test]
fn uas_local_set_checksum_recomputed_on_change() {
    // Two sets differing only in a data tag must get different checksums.
    let mk = |v: u8| {
        UasLocalSet::from_items(vec![
            LocalSetItem::new(TAG_PRECISION_TIMESTAMP, 0u64.to_be_bytes().to_vec()),
            LocalSetItem::new(10, vec![v]),
        ])
        .serialize_with_checksum()
    };
    let a = mk(0x01);
    let b = mk(0x02);
    assert_ne!(a, b);
    assert!(UasLocalSet::verify_checksum(&a).unwrap());
    assert!(UasLocalSet::verify_checksum(&b).unwrap());
    // Precision timestamp length is what the spec fixes it at.
    assert_eq!(PRECISION_TIMESTAMP_LEN, 8);
}

// ---------------------------------------------------------------------------
// 4. KLV-over-RTP fragmentation (RFC 6597) — bites
// ---------------------------------------------------------------------------

#[test]
fn klv_rtp_fragmentation_round_trip() {
    // Build a KLV unit larger than one RTP payload budget.
    let ts: u64 = 1_704_067_200_000_000;
    let mut value = ts.to_be_bytes().to_vec();
    value.extend_from_slice(&[0xAB; 200]); // filler tag body
    let ls = UasLocalSet::from_items(vec![
        LocalSetItem::new(TAG_PRECISION_TIMESTAMP, ts.to_be_bytes().to_vec()),
        LocalSetItem::new(11, value),
    ]);
    let unit = ls.serialize_with_checksum();
    assert!(unit.len() > 100);

    // MTU that forces >= 2 fragments (header 12 + payload budget 50 = 62).
    let mtu = RTP_HEADER_LEN + 50;
    let ts_rtp = 90_000u32;
    let packets = packetise_klv(&unit, 98, 0, ts_rtp, 0xCAFEBABE, mtu).unwrap();
    assert!(
        packets.len() >= 2,
        "expected fragmentation, got {}",
        packets.len()
    );

    // All fragments share the timestamp; marker set ONLY on the last.
    for (i, pkt) in packets.iter().enumerate() {
        let ts_field = u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]);
        assert_eq!(ts_field, ts_rtp, "fragment {i} timestamp");
        let marker = pkt[1] & RTP_MARKER_MASK != 0;
        assert_eq!(marker, i == packets.len() - 1, "marker on fragment {i}");
    }

    // Depacketise → exact original KLV bytes.
    let units = depacketise_klv(&packets).unwrap();
    assert_eq!(units.len(), 1);
    assert_eq!(units[0], unit);
    // And it still parses + verifies.
    assert!(UasLocalSet::verify_checksum(&units[0]).unwrap());
}

#[test]
fn klv_rtp_small_unit_single_packet() {
    let ls = UasLocalSet::from_items(vec![LocalSetItem::new(
        TAG_PRECISION_TIMESTAMP,
        0u64.to_be_bytes().to_vec(),
    )]);
    let unit = ls.serialize_with_checksum();
    let packets = packetise_klv(&unit, 98, 7, 42, 0x1234, 1400).unwrap();
    assert_eq!(packets.len(), 1);
    // Single packet: marker MUST be set.
    assert!(packets[0][1] & RTP_MARKER_MASK != 0);
    // Payload is exactly the KLV unit (no payload header — RFC 6597 §6.1).
    assert_eq!(&packets[0][RTP_HEADER_LEN..], unit.as_slice());
    // Round-trip.
    assert_eq!(depacketise_klv(&packets).unwrap(), vec![unit]);
}

#[test]
fn klv_rtp_empty_unit_rejected() {
    assert!(packetise_klv(&[], 98, 0, 0, 0, 1400).is_err());
}
