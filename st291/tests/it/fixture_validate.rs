//! Explicit ST 291-1:2011 validation (`AncPacket::validate`) against the
//! committed ST 2038 and RFC 8331 fixtures.
//!
//! No real ST 2038 / ST 2110-40 capture exists in this repo:
//! `fixtures/st291/anc.bin` is a synthetic, hand-constructed ST 2038 Table 2
//! PES and `anc_rtp.bin` re-wraps the same ANC words. They are regenerated so
//! that every ANC packet **conforms to ST 291-1** (`Data_Count` carries its
//! parity bits `0x102`/`0x203`; `Checksum_Word` is the §6.7 sum `0x134`/`0x1AC`),
//! so `is_valid()` is asserted on both. Parsing stays tolerant, so the bite
//! (that `validate` really can fail) comes from a separate hand-built
//! NON-conformant vector below, not from the fixtures.

use st291::{AncDataPacket, AncPacket, ValidationReport};

#[test]
fn synthetic_st2038_fixture_conforms_to_st291() {
    let data = test_fixtures::read("st291/anc.bin");
    let pkt = AncDataPacket::parse(&data).unwrap();
    assert!(!pkt.anc_packets.is_empty());
    for (i, p) in pkt.anc_packets.iter().enumerate() {
        let r = p.validate();
        assert_eq!(
            r,
            ValidationReport::new(true, true, true, true, true),
            "ANC packet {i}"
        );
        assert!(r.is_valid(), "ANC packet {i}");
    }
}

/// The bite: a hand-built packet with the two defects the old fixtures had
/// (`Data_Count` without parity bits, checksum off by one) must be reported,
/// independently of any committed fixture.
#[test]
fn hand_built_nonconformant_packet_is_reported() {
    // DID 0x161 / SDID 0x101 are parity-correct; DC 0x002 is not (0x02 has one
    // set bit -> b8 = 1, b9 = 0 -> 0x102). With the correct DC 0x102 the
    // checksum is 0x134 (0x161+0x101+0x102+0x2CF+0x101 = 0x734 -> 0x134, b8 = 1
    // -> b9 = 0); 0x233 is wrong.
    let bad = AncPacket {
        c_not_y_channel_flag: false,
        line_number: 9,
        horizontal_offset: 0,
        did: 0x161,
        sdid: 0x101,
        data_count: 0x002,
        user_data_words: vec![0x2CF, 0x101],
        checksum: 0x233,
    };
    let r = bad.validate();
    assert_eq!(r, ValidationReport::new(true, true, false, true, false));
    assert!(!r.is_valid());

    // Fixing exactly those two words makes the same packet valid.
    let good = AncPacket {
        data_count: 0x102,
        checksum: 0x134,
        ..bad
    };
    assert!(good.validate().is_valid());
}

#[cfg(feature = "rtp")]
#[test]
fn synthetic_rtp_fixture_conforms_to_st291() {
    use broadcast_common::Parse;
    use st291::AncRtpPayload;

    let data = test_fixtures::read("st291/anc_rtp.bin");
    // 12-byte RTP header (no CSRC/extension) precedes the RFC 8331 payload.
    let payload = AncRtpPayload::parse(&data[12..]).unwrap();
    assert!(!payload.anc_packets.is_empty());
    for p in &payload.anc_packets {
        assert!(p.content.validate().is_valid());
    }
}
