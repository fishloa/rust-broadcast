//! KLV codec + KLV-over-RTP integration tests (#478).
//!
//! Sources: SMPTE ST 336 framing (via MISB ST 0601 + RFC 6597), MISB ST 0601
//! UAS Datalink Local Set, RFC 6597 KLV-over-RTP. See
//! `transmux/docs/klv/klv-misb0601.md`.

use broadcast_common::{Parse, Serialize};
use transmux::klv::{
    CHECKSUM_LEN, KlvItem, LocalSetItem, PRECISION_TIMESTAMP_LEN, TAG_CHECKSUM,
    TAG_PRECISION_TIMESTAMP, UAS_LS_KEY, UNIVERSAL_LABEL_LEN, UasLocalSet, ber_length,
    checksum_bcc16, encode_ber_length,
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
// 3. UAS Local Set + checksum (ST 0601 running-sum vector)
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
    // Expected bytes: MISB ST 0601 §7.1 "Lower 16-bits of summation" over
    // bytes[..29] (the UL key, BER length and both items' tag+length).
    let expected: [u8; 31] = [
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00,
        0x00, // UL
        0x0E, // BER length 14
        0x02, 0x08, 0x00, 0x06, 0x0D, 0xD7, 0x10, 0x21, 0x20, 0x00, // tag 2, len 8, ts
        0x01, 0x02, 0x5C, 0x90, // tag 1, len 2, checksum = 0x5C90
    ];
    assert_eq!(packet.as_slice(), &expected);

    // The hand-computed ST 0601 running-sum value.
    let split = packet.len() - CHECKSUM_LEN;
    assert_eq!(checksum_bcc16(&packet[..split]), 0x5C90);

    // Round-trip parse: timestamp reads back, checksum verifies.
    let parsed = UasLocalSet::parse(&packet).unwrap();
    assert_eq!(parsed.precision_timestamp(), Some(ts));
    assert_eq!(parsed.stored_checksum(), Some(0x5C90));
    assert!(UasLocalSet::verify_checksum(&packet).unwrap());

    // Corrupt a value byte → checksum fails (bites).
    let mut bad = packet.clone();
    bad[20] ^= 0xFF; // a timestamp byte
    assert!(!UasLocalSet::verify_checksum(&bad).unwrap());
    // Corrupt a checksum byte → also fails.
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
    let unit_bytes = bytes::Bytes::from(unit.clone());
    let packets = packetise_klv(&unit_bytes, 98, 0, ts_rtp, 0xCAFEBABE, mtu).unwrap();
    assert!(
        packets.len() >= 2,
        "expected fragmentation, got {}",
        packets.len()
    );

    // All fragments share the timestamp; marker set ONLY on the last.
    for (i, pkt) in packets.iter().enumerate() {
        let h = &pkt.header;
        let ts_field = u32::from_be_bytes([h[4], h[5], h[6], h[7]]);
        assert_eq!(ts_field, ts_rtp, "fragment {i} timestamp");
        let marker = h[1] & RTP_MARKER_MASK != 0;
        assert_eq!(marker, i == packets.len() - 1, "marker on fragment {i}");
    }

    // Depacketise → exact original KLV bytes.
    let contiguous: Vec<Vec<u8>> = packets.iter().map(|p| p.as_contiguous().to_vec()).collect();
    let units = depacketise_klv(&contiguous).unwrap();
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
    let unit_bytes = bytes::Bytes::from(unit.clone());
    let packets = packetise_klv(&unit_bytes, 98, 7, 42, 0x1234, 1400).unwrap();
    assert_eq!(packets.len(), 1);
    // Single packet: marker MUST be set.
    assert!(packets[0].header[1] & RTP_MARKER_MASK != 0);
    // Payload is exactly the KLV unit (no payload header — RFC 6597 §6.1).
    assert_eq!(&packets[0].payload[..], unit.as_slice());
    // Round-trip.
    let contiguous: Vec<Vec<u8>> = packets.iter().map(|p| p.as_contiguous().to_vec()).collect();
    assert_eq!(depacketise_klv(&contiguous).unwrap(), vec![unit]);
}

#[test]
fn klv_rtp_empty_unit_rejected() {
    assert!(packetise_klv(&bytes::Bytes::new(), 98, 0, 0, 0, 1400).is_err());
}

// ---------------------------------------------------------------------------
// 3b. Real ST 0601 packets with published checksums (audit r04-W10)
// ---------------------------------------------------------------------------
//
// The checksum is MISB ST 0601 §7.1's "lower 16-bits of summation" — a running
// big-endian 16-bit sum from the first byte of the 16-byte UL key through the
// checksum item's own length byte. ST 0601's change history records removing
// the earlier "CRC-16" wording, because tag 1 "represents a checksum and not a
// cyclic redundancy check"; a CRC over these packets yields a different value,
// so this vector is exactly what the r04-W10 regression broke.
//
// The packets below are published third-party vectors (jmisb's `KlvParserTest`
// fixtures, which carry their own expected tag-1 bytes), so neither side of the
// assertion comes from this crate.

/// Packet: three sensor-geometry tags and a checksum. 37 bytes.
const JMISB_LATLONALT_PACKET: &[u8] = &[
    0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00,
    0x00, // UL key
    0x14, // BER length 20
    0x0D, 0x04, 0x3C, 0x4E, 0xAD, 0xFA, // tag 13 Sensor Latitude
    0x0E, 0x04, 0xCD, 0x6B, 0x78, 0x4E, // tag 14 Sensor Longitude
    0x0F, 0x02, 0x1B, 0xC4, // tag 15 Sensor True Altitude
    0x01, 0x02, 0x2D, 0xC4, // tag 1, len 2, checksum = 0x2DC4
];

/// Packet: checksum only. 21 bytes.
const JMISB_CHECKSUM_ONLY_PACKET: &[u8] = &[
    0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00,
    0x00, // UL key
    0x04, // BER length 4
    0x01, 0x02, 0x4C, 0x51, // tag 1, len 2, checksum = 0x4C51
];

/// Packet: an unknown tag between the UL key and the checksum. 26 bytes.
const JMISB_UNKNOWN_TAG_PACKET: &[u8] = &[
    0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00,
    0x00, // UL key
    0x09, // BER length 9
    0x90, 0x00, 0x02, 0x0A, 0x0B, // unknown tag
    0x01, 0x02, 0x5A, 0xEF, // tag 1, len 2, checksum = 0x5AEF
];

/// Every published vector's checksum must verify, and its tag-1 value must be
/// the one the packet carries.
#[test]
fn published_st0601_packets_verify_against_the_running_sum() {
    for (name, packet, expected) in [
        ("latlonalt", JMISB_LATLONALT_PACKET, 0x2DC4u16),
        ("checksum-only", JMISB_CHECKSUM_ONLY_PACKET, 0x4C51),
        ("unknown-tag", JMISB_UNKNOWN_TAG_PACKET, 0x5AEF),
    ] {
        // The sum over the packet up to (excluding) the 2 value bytes is the
        // published value — this is the assertion a CRC implementation fails.
        let split = packet.len() - CHECKSUM_LEN;
        assert_eq!(
            checksum_bcc16(&packet[..split]),
            expected,
            "{name}: recomputed running sum must equal the published tag-1 value"
        );
        assert!(
            UasLocalSet::verify_checksum(packet).expect("verify"),
            "{name}: verify_checksum must accept a real ST 0601 packet"
        );
        // The stored value must read back as the same number.
        let parsed = UasLocalSet::parse(packet).expect("parse real ST 0601 packet");
        assert_eq!(parsed.stored_checksum(), Some(expected), "{name}");
    }
}

/// A CRC-16 of these packets gives a *different* number, so a regression to the
/// pre-r04-W10 algorithm cannot pass the vector above. This is asserted
/// directly, as documentation of what the fix changed.
#[test]
fn published_st0601_checksum_is_not_a_crc16() {
    // CRC-16/CCITT-FALSE (poly 0x1021, init 0xFFFF), the algorithm the audit
    // found in place of the running sum.
    let crc16_ccitt_false = |data: &[u8]| -> u16 {
        let mut crc: u16 = 0xFFFF;
        for &byte in data {
            crc ^= u16::from(byte) << 8;
            for _ in 0..8 {
                crc = if crc & 0x8000 != 0 {
                    (crc << 1) ^ 0x1021
                } else {
                    crc << 1
                };
            }
        }
        crc
    };
    let packet = JMISB_LATLONALT_PACKET;
    let split = packet.len() - CHECKSUM_LEN;
    assert_eq!(checksum_bcc16(&packet[..split]), 0x2DC4);
    assert_eq!(crc16_ccitt_false(&packet[..split]), 0xC945);
    assert_ne!(
        checksum_bcc16(&packet[..split]),
        crc16_ccitt_false(&packet[..split]),
        "ST 0601 tag 1 is a running sum, not a CRC (§5.5, §7.1)"
    );
}

/// A round trip through this crate's serializer must reproduce a published
/// packet byte-for-byte, checksum included — the strongest form of the check,
/// since it exercises both the checksum *and* the item encoding.
#[test]
fn serializing_the_published_items_reproduces_a_real_packet() {
    // The latlonalt packet's own items, in wire order.
    let items = vec![
        LocalSetItem::new(13, vec![0x3C, 0x4E, 0xAD, 0xFA]),
        LocalSetItem::new(14, vec![0xCD, 0x6B, 0x78, 0x4E]),
        LocalSetItem::new(15, vec![0x1B, 0xC4]),
    ];
    let packet = UasLocalSet::from_items(items).serialize_with_checksum();
    assert_eq!(
        packet.as_slice(),
        JMISB_LATLONALT_PACKET,
        "serialize_with_checksum must reproduce a real ST 0601 packet exactly, \
         checksum included"
    );
}

/// The checksum must be recomputed (never echoed) and must change when a data
/// byte changes, including the *last* item before it.
#[test]
fn checksum_covers_every_item_before_it() {
    let mk = |v: u8| {
        UasLocalSet::from_items(vec![
            LocalSetItem::new(TAG_PRECISION_TIMESTAMP, 1u64.to_be_bytes().to_vec()),
            LocalSetItem::new(3, vec![v]),
        ])
        .serialize_with_checksum()
    };
    let a = mk(0x41);
    let b = mk(0x42);
    assert_ne!(
        a[a.len() - CHECKSUM_LEN..],
        b[b.len() - CHECKSUM_LEN..],
        "a changed item must change the checksum"
    );
    assert!(UasLocalSet::verify_checksum(&a).unwrap());
    assert!(UasLocalSet::verify_checksum(&b).unwrap());

    // And a single flipped byte anywhere before the checksum must be caught.
    for i in 0..a.len() - CHECKSUM_LEN {
        let mut bad = a.clone();
        bad[i] ^= 0x01;
        assert!(
            !UasLocalSet::verify_checksum(&bad).unwrap(),
            "a flipped byte at {i} must fail verification"
        );
    }
}
