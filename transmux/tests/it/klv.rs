//! KLV codec + KLV-over-RTP integration tests (#478).
//!
//! Sources: SMPTE ST 336 framing (via MISB ST 0601 + RFC 6597), MISB ST 0601
//! UAS Datalink Local Set, RFC 6597 KLV-over-RTP. See
//! `private/specs-verified/mpeg/transmux/klv/klv-misb0601.md`.

use broadcast_common::error::{BufferTooShort, InvalidValue};
use broadcast_common::klv::{KlvItem, UNIVERSAL_LABEL_LEN};
use broadcast_common::{Parse, Serialize};
use transmux::Error;
use transmux::klv::{
    CHECKSUM_LEN, LocalSetItem, PRECISION_TIMESTAMP_LEN, TAG_CHECKSUM, TAG_PRECISION_TIMESTAMP,
    UAS_LS_KEY, UasLocalSet, ber_oid, checksum_bcc16, encode_ber_oid,
};
use transmux::rtp::{depacketise_klv, packetise_klv};

const RTP_HEADER_LEN: usize = 12;
const RTP_MARKER_MASK: u8 = 0x80;

// ---------------------------------------------------------------------------
// 1. KLV length field (BER framing bites)
// ---------------------------------------------------------------------------

#[test]
fn klv_item_value_grows_ber_length_to_long_form() {
    // The value length drives the on-wire BER length: 255 -> 81 FF, 256 -> 82 01 00.
    let at_255 = KlvItem::new(UAS_LS_KEY, &[0xAB; 255]);
    let b255 = at_255.try_to_bytes().unwrap();
    assert_eq!(
        &b255[UNIVERSAL_LABEL_LEN..UNIVERSAL_LABEL_LEN + 2],
        &[0x81, 0xFF]
    );
    assert_eq!(b255.len(), at_255.serialized_len());

    let at_256 = KlvItem::new(UAS_LS_KEY, &[0xAB; 256]);
    let b256 = at_256.try_to_bytes().unwrap();
    assert_eq!(
        &b256[UNIVERSAL_LABEL_LEN..UNIVERSAL_LABEL_LEN + 3],
        &[0x82, 0x01, 0x00]
    );
    assert_eq!(b256.len(), at_256.serialized_len());
    assert_eq!(KlvItem::parse(&b256).unwrap(), at_256);
}

#[test]
fn klv_item_indefinite_form_rejected() {
    // 0x80 alone (indefinite form) is not permitted in KLV -> error, no panic.
    let mut bytes = UAS_LS_KEY.to_vec();
    bytes.push(0x80);
    assert!(matches!(
        KlvItem::parse(&bytes),
        Err(broadcast_common::klv::KlvError::BerIndefiniteLength)
    ));
    // Long form promising bytes that aren't there -> error, no panic.
    bytes[UNIVERSAL_LABEL_LEN] = 0x82;
    bytes.push(0x01);
    assert!(matches!(
        KlvItem::parse(&bytes),
        Err(broadcast_common::klv::KlvError::BufferTooShort(..))
    ));
}

// ---------------------------------------------------------------------------
// 1b. BER-OID tags (ISO/IEC 8825-1 §8.19)
// ---------------------------------------------------------------------------

#[test]
fn ber_oid_u32_max_is_the_largest_accepted_tag() {
    // 0xFFFFFFFF needs five base-128 digits; the accumulator before the last
    // digit is exactly `u32::MAX >> 7`, the guard's inclusive boundary.
    let encoded = encode_ber_oid(u32::MAX);
    assert_eq!(encoded, vec![0x8F, 0xFF, 0xFF, 0xFF, 0x7F]);
    assert_eq!(ber_oid(&encoded), Ok((u32::MAX, 5)));
}

#[test]
fn ber_oid_rejects_an_over_long_tag_instead_of_wrapping() {
    // 28 bits before the final digit is past `u32::MAX >> 7`: shifting in one
    // more 7-bit group would wrap.
    assert!(matches!(
        ber_oid(&[0xFF, 0xFF, 0xFF, 0xFF, 0x7F]),
        Err(Error::InvalidValue(InvalidValue {
            field: "ber_oid",
            ..
        }))
    ));
}

#[test]
fn ber_oid_truncated_reports_the_real_need() {
    assert!(matches!(
        ber_oid(&[0x81]),
        Err(Error::BufferTooShort(BufferTooShort {
            need: 2,
            have: 1,
            ..
        }))
    ));
}

// ---------------------------------------------------------------------------
// 2. KLV item round-trip + computed length
// ---------------------------------------------------------------------------

#[test]
fn klv_item_round_trip() {
    let item = KlvItem::new(UAS_LS_KEY, b"hello KLV value");
    let bytes = item.try_to_bytes().unwrap();
    let parsed = KlvItem::parse(&bytes).unwrap();
    assert_eq!(parsed, item);
    // serialize -> parse -> equal, and the byte length is exactly serialized_len.
    assert_eq!(bytes.len(), item.serialized_len());
}

#[test]
fn klv_item_length_is_computed_not_stored() {
    // The serialized length byte is COMPUTED from the value: mutate the value
    // length and the on-wire length field must track it.
    let short = KlvItem::new(UAS_LS_KEY, &[0xAB; 5]);
    let long = KlvItem::new(UAS_LS_KEY, &[0xAB; 6]);
    let sb = short.try_to_bytes().unwrap();
    let lb = long.try_to_bytes().unwrap();
    // Byte at index 16 is the (short-form) BER length.
    assert_eq!(sb[UNIVERSAL_LABEL_LEN], 5);
    assert_eq!(lb[UNIVERSAL_LABEL_LEN], 6);

    // A value that forces a 2-byte BER length (>= 128).
    let big = KlvItem::new(UAS_LS_KEY, &[0xCD; 300]);
    let bb = big.try_to_bytes().unwrap();
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
    let bytes = ls.serialize_with_checksum().unwrap();
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

    let packet = ls.serialize_with_checksum().unwrap();

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
        .unwrap()
    };
    let a = mk(0x01);
    let b = mk(0x02);
    assert_ne!(a, b);
    assert!(UasLocalSet::verify_checksum(&a).unwrap());
    assert!(UasLocalSet::verify_checksum(&b).unwrap());
    // Precision timestamp length is what the spec fixes it at.
    assert_eq!(PRECISION_TIMESTAMP_LEN, 8);
}

/// `serialized_len`, `serialize_into`'s returned count and the produced bytes
/// must all agree with `serialize_with_checksum`.
fn assert_local_set_length(set: &UasLocalSet) {
    let expected = set.serialize_with_checksum().unwrap();
    assert_eq!(set.serialized_len(), expected.len(), "serialized_len");
    assert_eq!(
        set.try_to_bytes().unwrap().len(),
        expected.len(),
        "try_to_bytes"
    );
    let mut buf = vec![0u8; set.serialized_len()];
    assert_eq!(
        set.serialize_into(&mut buf).unwrap(),
        expected.len(),
        "serialize_into count"
    );
    assert_eq!(buf, expected, "serialize_into bytes");
}

#[test]
fn uas_local_set_lengths_cover_empty_short_long_and_checksum_items() {
    // (a) Empty: only the UL key, the packet length and the tag-1 checksum.
    assert_local_set_length(&UasLocalSet::new());

    // (b) Small items (one-byte tags and lengths).
    assert_local_set_length(&UasLocalSet::from_items(vec![
        LocalSetItem::new(TAG_PRECISION_TIMESTAMP, 0u64.to_be_bytes().to_vec()),
        LocalSetItem::new(10, vec![0xAB, 0xCD]),
    ]));

    // (c) A 200-byte value (two-byte BER length) under a 2-byte BER-OID tag.
    assert_local_set_length(&UasLocalSet::from_items(vec![LocalSetItem::new(
        200,
        vec![0x5A; 200],
    )]));

    // (d) An existing tag-1 item is replaced, not summed into the length.
    assert_local_set_length(&UasLocalSet::from_items(vec![
        LocalSetItem::new(10, vec![0x01]),
        LocalSetItem::new(TAG_CHECKSUM, vec![0xDE, 0xAD]),
    ]));
}

#[test]
fn verify_checksum_reports_the_minimum_packet_length() {
    // 16-byte UL key + 1-byte packet length + 2-byte checksum = 19.
    assert!(matches!(
        UasLocalSet::verify_checksum(&[0u8; 18]),
        Err(Error::BufferTooShort(BufferTooShort {
            need: 19,
            have: 18,
            ..
        }))
    ));
    // The minimum itself is long enough to check, not truncated.
    assert_eq!(UasLocalSet::verify_checksum(&[0u8; 19]), Ok(true));
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
    let unit = ls.serialize_with_checksum().unwrap();
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
    let unit = ls.serialize_with_checksum().unwrap();
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
    assert!(matches!(
        packetise_klv(&bytes::Bytes::new(), 98, 0, 0, 0, 1400),
        Err(transmux::error::Error::InvalidInput(..))
    ));
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
    let packet = UasLocalSet::from_items(items)
        .serialize_with_checksum()
        .unwrap();
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
        .unwrap()
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

fn raw_rtp(seq: u16, ts: u32, marker: bool, payload: &[u8]) -> Vec<u8> {
    let mut p = vec![0x80u8, if marker { RTP_MARKER_MASK | 98 } else { 98 }];
    p.extend_from_slice(&seq.to_be_bytes());
    p.extend_from_slice(&ts.to_be_bytes());
    p.extend_from_slice(&[0, 0, 0, 0]);
    p.extend_from_slice(payload);
    p
}

#[test]
fn klv_timestamp_change_after_an_empty_packet_emits_no_empty_unit() {
    let packets = vec![raw_rtp(0, 1, false, &[]), raw_rtp(1, 2, true, b"ab")];
    let units = depacketise_klv(&packets).unwrap();
    assert_eq!(units, vec![b"ab".to_vec()], "no empty unit is emitted");
}

#[test]
fn two_byte_ber_oid_tag_round_trips_in_a_local_set() {
    let set = UasLocalSet::from_items(vec![
        LocalSetItem::new(TAG_PRECISION_TIMESTAMP, 1_000_000u64.to_be_bytes().to_vec()),
        LocalSetItem::new(131, 42u64.to_be_bytes().to_vec()),
    ]);
    let packet = set.serialize_with_checksum().unwrap();
    let parsed = UasLocalSet::parse(&packet).unwrap();
    assert_eq!(parsed.items[1].tag, 131);
    assert_eq!(parsed.items[1].value, 42u64.to_be_bytes().to_vec());
    assert_eq!(encode_ber_oid(131), vec![0x81, 0x03]);
}
