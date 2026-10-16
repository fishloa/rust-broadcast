//! Explicit ST 291-1:2011 validation (`AncPacket::validate`) against the
//! committed ST 2038 and RFC 8331 fixtures.
//!
//! HONEST FINDING: no real ST 2038 / ST 2110-40 capture exists in this repo;
//! `fixtures/st291/anc.bin` is hand-constructed from ST 2038 Table 2 and
//! `anc_rtp.bin` re-wraps the same ANC words. Those hand-typed words do NOT
//! satisfy ST 291-1: both packets' `Data_Count` words carry no parity bits
//! (`0x002`/`0x003` instead of `0x102`/`0x203`) and both `Checksum_Word`s are
//! off by one (`0x233` vs `0x234`, `0x1AB` vs `0x1AC`, hand-computed from
//! §6.7). Parsing stays tolerant and byte-exact (the fixtures are goldens), so
//! this test pins the exact failures, which documents the finding and also
//! proves `validate` bites on non-conformant input. Once a real capture is
//! added, assert `is_valid()` on it here.

use st291::{AncDataPacket, ValidationReport};

#[test]
fn synthetic_st2038_fixture_reports_its_known_nonconformances() {
    let data = test_fixtures::read("st291/anc.bin");
    let pkt = AncDataPacket::parse(&data).unwrap();

    // Packet 0: DID 0x161 / SDID 0x101 are parity-correct; DC 0x002 is not
    // (0x02 has one set bit -> b8 = 1, b9 = 0 -> 0x102); checksum
    // (0x161+0x101+0x002+0x0CF+0x101 = 0x434 -> 0x034, b9 = 1) is 0x234.
    let r0 = pkt.anc_packets[0].validate();
    assert_eq!(r0, ValidationReport::new(true, true, false, true, false));

    // Packet 1: DID 0x241 / SDID 0x102 correct; DC 0x003 not (0x203);
    // checksum (0x041+0x102+0x003+0x111+0x022+0x133 = 0x3AC -> 0x1AC) is
    // 0x1AC, stored 0x1AB.
    let r1 = pkt.anc_packets[1].validate();
    assert_eq!(r1, ValidationReport::new(true, true, false, true, false));
}

#[cfg(feature = "rtp")]
#[test]
fn synthetic_rtp_fixture_matches_the_st2038_findings() {
    use broadcast_common::Parse;
    use st291::AncRtpPayload;

    let data = test_fixtures::read("st291/anc_rtp.bin");
    // 12-byte RTP header (no CSRC/extension) precedes the RFC 8331 payload.
    let payload = AncRtpPayload::parse(&data[12..]).unwrap();
    for p in &payload.anc_packets {
        let r = p.content.validate();
        assert!(r.did_parity && r.sdid_parity && r.data_count_matches_udw);
        assert!(!r.data_count_parity && !r.checksum);
    }
}
