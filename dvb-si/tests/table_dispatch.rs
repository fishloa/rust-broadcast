//! Integration tests for [`dvb_si::tables::AnyTableSection`] dispatch.
//!
//! Every builder here is self-contained: bytes are constructed from scratch
//! matching the wire format expected by each module's parser.

use dvb_si::tables::AnyTableSection;

// ── helpers ──────────────────────────────────────────────────────────────────

fn crc32_mpeg2(data: &[u8]) -> u32 {
    dvb_common::crc32_mpeg2::compute(data)
}

/// Append a correct CRC_32 over all bytes already in `v`.
fn push_crc(v: &mut Vec<u8>) {
    let crc = crc32_mpeg2(v);
    v.extend_from_slice(&crc.to_be_bytes());
}

// ── PAT builder ──────────────────────────────────────────────────────────────

fn build_pat(tsid: u16, version: u8, entries: &[(u16, u16)]) -> Vec<u8> {
    let section_length = (5 + entries.len() * 4 + 4) as u16; // ext_hdr(5)+entries+crc(4)
    let mut v = vec![
        0x00u8,
        0xB0 | ((section_length >> 8) as u8 & 0x0F),
        (section_length & 0xFF) as u8,
        (tsid >> 8) as u8,
        (tsid & 0xFF) as u8,
        0xC0 | ((version & 0x1F) << 1) | 0x01,
        0x00,
        0x00,
    ];
    for &(pn, pid) in entries {
        v.extend_from_slice(&pn.to_be_bytes());
        v.push(0xE0 | ((pid >> 8) as u8 & 0x1F));
        v.push((pid & 0xFF) as u8);
    }
    push_crc(&mut v);
    v
}

// ── PMT builder ──────────────────────────────────────────────────────────────

fn build_pmt(program_number: u16, version: u8, pcr_pid: u16) -> Vec<u8> {
    let section_length = (5 + 2 + 2 + 4) as u16; // ext_hdr(5)+pcr_pid(2)+prog_info_len(2)+crc(4)
    let mut v = vec![
        0x02u8,
        0xB0 | ((section_length >> 8) as u8 & 0x0F),
        (section_length & 0xFF) as u8,
        (program_number >> 8) as u8,
        (program_number & 0xFF) as u8,
        0xC0 | ((version & 0x1F) << 1) | 0x01,
        0x00,
        0x00,
        0xE0 | ((pcr_pid >> 8) as u8 & 0x1F),
        (pcr_pid & 0xFF) as u8,
        0xF0, // program_info_length hi nibble
        0x00, // program_info_length = 0
    ];
    push_crc(&mut v);
    v
}

// ── SDT builder ──────────────────────────────────────────────────────────────

fn build_sdt(table_id: u8, tsid: u16, onid: u16) -> Vec<u8> {
    // section_length: ext_hdr(5) + onid(2) + reserved(1) + crc(4) = 12
    let section_length: u16 = 12;
    let mut v = vec![
        table_id,
        0xB0 | ((section_length >> 8) as u8 & 0x0F),
        (section_length & 0xFF) as u8,
        (tsid >> 8) as u8,
        (tsid & 0xFF) as u8,
        0xC0 | 0x01, // version=0, cni=1
        0x00,
        0x00,
        (onid >> 8) as u8,
        (onid & 0xFF) as u8,
        0xFF, // reserved_future_use
    ];
    push_crc(&mut v);
    v
}

// ── EIT builder ──────────────────────────────────────────────────────────────

fn build_eit(table_id: u8, service_id: u16, tsid: u16, onid: u16) -> Vec<u8> {
    // section_length: ext_hdr(5) + tsid(2) + onid(2) + seg_last(1) + last_tid(1) + crc(4) = 15
    let section_length: u16 = 15;
    let mut v = vec![
        table_id,
        0xB0 | ((section_length >> 8) as u8 & 0x0F),
        (section_length & 0xFF) as u8,
        (service_id >> 8) as u8,
        (service_id & 0xFF) as u8,
        0xC0 | 0x01, // version=0, cni=1
        0x00,
        0x00,
        (tsid >> 8) as u8,
        (tsid & 0xFF) as u8,
        (onid >> 8) as u8,
        (onid & 0xFF) as u8,
        0x00,     // segment_last_section_number
        table_id, // last_table_id
    ];
    push_crc(&mut v);
    v
}

// ── TDT builder ──────────────────────────────────────────────────────────────

fn build_tdt() -> Vec<u8> {
    // Short section: table_id(1) + section_syntax(2) + 5 UTC bytes.
    // Section length must equal 5 (UTC_TIME_LEN).
    vec![0x70, 0x70, 0x05, 0xE4, 0x09, 0x12, 0x34, 0x56]
}

// ── TOT builder ──────────────────────────────────────────────────────────────

fn build_tot() -> Vec<u8> {
    // header(3) + UTC(5) + desc_loop_len(2) + crc(4)
    let section_length: u16 = (5 + 2 + 4) as u16;
    let mut v = vec![
        0x73u8,
        0x70 | ((section_length >> 8) as u8 & 0x0F), // SSI=0 per EN 300 468 §5.2.6
        (section_length & 0xFF) as u8,
        0xE4,
        0x09,
        0x12,
        0x34,
        0x56, // UTC time
        0xF0,
        0x00, // descriptor_loop_length = 0
    ];
    push_crc(&mut v);
    v
}

// ── DSM-CC builder ───────────────────────────────────────────────────────────

fn build_dsmcc(table_id: u8) -> Vec<u8> {
    // section_length: ext_hdr(5) + payload(0) + crc(4) = 9
    let section_length: u16 = 9;
    let mut v = vec![
        table_id,
        0xB0 | ((section_length >> 8) as u8 & 0x0F),
        (section_length & 0xFF) as u8,
        0x00,
        0x01,        // extension_id
        0xC0 | 0x01, // version=0, cni=1
        0x00,        // section_number
        0x00,        // last_section_number
    ];
    push_crc(&mut v);
    v
}

// ── ProtectionMessage builder ─────────────────────────────────────────────────

fn build_protection_message() -> Vec<u8> {
    // Minimal auth-message body (from protection_message.rs test):
    // reference (1) + hash (4)
    let reference = [0x01u8];
    let hash = [0xAAu8, 0xBB, 0xCC, 0xDD];
    let mut hashes_loop: Vec<u8> = vec![(1u8 << 4) | (reference.len() as u8)];
    hashes_loop.extend_from_slice(&reference);
    hashes_loop.extend_from_slice(&hash);
    let loop_len = hashes_loop.len();

    let mut body: Vec<u8> = vec![
        0x00,                                  // section_hash_algorithm_identifier
        hash.len() as u8,                      // section_hash_length
        0x01,                                  // signature_algorithm_identifier
        0xF0 | ((loop_len >> 8) as u8 & 0x0F), // reserved | loop_length hi
        (loop_len & 0xFF) as u8,               // loop_length lo
    ];
    body.extend_from_slice(&hashes_loop);
    body.push(2);
    body.extend_from_slice(&[0xDE, 0xAD]); // extension_bytes
    body.push(3);
    body.extend_from_slice(&[0x11, 0x22, 0x33]); // key identifier
    body.extend_from_slice(&[0x90, 0x91, 0x92, 0x93, 0x94, 0x95]); // signature

    let section_length = (5 + body.len() + 4) as u16; // ext_hdr(5) + body + crc(4)
    let mut v = vec![
        0x7Bu8,
        0xB0 | ((section_length >> 8) as u8 & 0x0F),
        (section_length & 0xFF) as u8,
        0x00,
        0x01,        // extension (message_id)
        0xC0 | 0x01, // version=0, cni=1
        0x00,        // section_number
        0x00,        // last_section_number
    ];
    v.extend_from_slice(&body);
    push_crc(&mut v);
    v
}

// ─────────────────────────────────────────────────────────────────────────────
// Dispatch tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn dispatch_pat_table_id_0x00() {
    let bytes = build_pat(0x1234, 0, &[(1, 0x0100)]);
    let parsed = AnyTableSection::parse(&bytes).unwrap();
    assert!(
        matches!(parsed, AnyTableSection::PatSection(_)),
        "expected PatSection, got {parsed:?}"
    );
}

#[test]
fn dispatch_pmt_table_id_0x02() {
    let bytes = build_pmt(42, 0, 0x0100);
    let parsed = AnyTableSection::parse(&bytes).unwrap();
    assert!(
        matches!(parsed, AnyTableSection::PmtSection(_)),
        "expected PmtSection, got {parsed:?}"
    );
}

#[test]
fn dispatch_sdt_actual_table_id_0x42() {
    let bytes = build_sdt(0x42, 1, 0x0020);
    let parsed = AnyTableSection::parse(&bytes).unwrap();
    assert!(
        matches!(parsed, AnyTableSection::SdtSection(_)),
        "expected SdtSection, got {parsed:?}"
    );
}

#[test]
fn dispatch_sdt_other_table_id_0x46() {
    let bytes = build_sdt(0x46, 1, 0x0020);
    let parsed = AnyTableSection::parse(&bytes).unwrap();
    assert!(
        matches!(parsed, AnyTableSection::SdtSection(_)),
        "expected SdtSection (other), got {parsed:?}"
    );
}

#[test]
fn dispatch_eit_pf_actual_0x4e() {
    let bytes = build_eit(0x4E, 100, 1, 0x0020);
    let parsed = AnyTableSection::parse(&bytes).unwrap();
    assert!(
        matches!(parsed, AnyTableSection::EitSection(_)),
        "expected EitSection (p/f actual), got {parsed:?}"
    );
}

#[test]
fn dispatch_eit_schedule_segment_0x50() {
    let bytes = build_eit(0x50, 100, 1, 0x0020);
    let parsed = AnyTableSection::parse(&bytes).unwrap();
    assert!(
        matches!(parsed, AnyTableSection::EitSection(_)),
        "expected EitSection (schedule 0x50), got {parsed:?}"
    );
}

#[test]
fn dispatch_tdt_short_section_0x70() {
    let bytes = build_tdt();
    let parsed = AnyTableSection::parse(&bytes).unwrap();
    assert!(
        matches!(parsed, AnyTableSection::TdtSection(_)),
        "expected TdtSection, got {parsed:?}"
    );
}

#[test]
fn dispatch_tot_0x73() {
    let bytes = build_tot();
    let parsed = AnyTableSection::parse(&bytes).unwrap();
    assert!(
        matches!(parsed, AnyTableSection::TotSection(_)),
        "expected TotSection, got {parsed:?}"
    );
}

#[test]
fn dispatch_dsmcc_0x3b() {
    let bytes = build_dsmcc(0x3B);
    let parsed = AnyTableSection::parse(&bytes).unwrap();
    assert!(
        matches!(parsed, AnyTableSection::DsmccSection(_)),
        "expected DsmccSection, got {parsed:?}"
    );
}

/// 0x3E is routed to DsmccSection (NOT MpeDatagram) by the default dispatcher.
#[test]
fn dispatch_0x3e_routes_to_dsmcc_not_mpe() {
    let bytes = build_dsmcc(0x3E);
    let parsed = AnyTableSection::parse(&bytes).unwrap();
    assert!(
        matches!(parsed, AnyTableSection::DsmccSection(_)),
        "0x3E should dispatch to DsmccSection, got {parsed:?}"
    );
    // Must NOT be MpeDatagram via the default dispatcher.
    assert!(
        !matches!(parsed, AnyTableSection::MpeDatagram(_)),
        "0x3E must not auto-dispatch to MpeDatagram"
    );
}

#[test]
fn dispatch_protection_message_0x7b() {
    let bytes = build_protection_message();
    let parsed = AnyTableSection::parse(&bytes).unwrap();
    assert!(
        matches!(parsed, AnyTableSection::ProtectionMessage(_)),
        "expected ProtectionMessage, got {parsed:?}"
    );
}

#[test]
fn dispatch_unknown_table_id_0x90() {
    // 0x90 is not defined; should produce Unknown.
    let bytes = [0x90u8, 0x01, 0x00]; // minimal: header + 1 section_length byte
    let parsed = AnyTableSection::parse(&bytes).unwrap();
    match parsed {
        AnyTableSection::Unknown { table_id, raw } => {
            assert_eq!(table_id, 0x90);
            assert_eq!(raw, &[0x90u8, 0x01, 0x00]);
        }
        other => panic!("expected Unknown, got {other:?}"),
    }
}

#[test]
fn dispatch_empty_input_returns_buffer_too_short() {
    let err = AnyTableSection::parse(&[]).unwrap_err();
    assert!(
        matches!(err, dvb_si::error::Error::BufferTooShort { .. }),
        "expected BufferTooShort, got {err:?}"
    );
}

/// DISPATCHED_RANGES must be contiguous (none overlap). The drift test in the
/// macro already checks this; this is an integration-level sanity check.
#[test]
fn dispatched_ranges_are_sorted_and_disjoint() {
    let ranges = AnyTableSection::DISPATCHED_RANGES;
    // Each range must be well-formed (lo <= hi).
    for &(lo, hi) in ranges {
        assert!(lo <= hi, "malformed range ({lo:#04x}, {hi:#04x})");
    }
    // Sorted and non-overlapping (same algorithm as the macro-internal test).
    let mut sorted = ranges.to_vec();
    sorted.sort_by_key(|r| r.0);
    for w in sorted.windows(2) {
        let (_, prev_hi) = w[0];
        let (next_lo, _) = w[1];
        assert!(next_lo > prev_hi, "overlapping dispatch ranges: {w:?}");
    }
}

/// `parse_as` bypasses dispatch and lets callers get the typed MPE view for
/// a `0x3E` section.
#[test]
fn parse_as_mpe_datagram_for_0x3e() {
    use dvb_si::tables::mpe::{MacAddress, MpeDatagramSection};

    // Build a minimal valid MPE datagram_section (table_id=0x3E).
    // header(3) + extension(9) + payload(0) + trailer(4) = 16 bytes.
    // SSI=0 (private-section framing), private=1, reserved=11.
    let section_length = 9u16 + 4; // extension(9) + trailer(4), no payload
    let mut v = vec![
        0x3Eu8,
        0x70 | ((section_length >> 8) as u8 & 0x0F), // SSI=0, private=1, reserved=11
        (section_length & 0xFF) as u8,
        0xAA, // MAC_address_6 (LSB)
        0xBB, // MAC_address_5
        0xC1, // reserved=11 | payload_sc=0 | address_sc=0 | llc_snap=0 | cni=1
        0x00, // section_number
        0x00, // last_section_number
        0x11, // MAC_address_4
        0x22, // MAC_address_3
        0x33, // MAC_address_2
        0x44, // MAC_address_1 (MSB)
    ];
    // Verbatim checksum (SSI=0 path: trailer bytes are preserved as-is).
    v.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

    let mpe =
        AnyTableSection::parse_as::<MpeDatagramSection>(&v).expect("valid MPE section must parse");
    // MAC is reassembled network-order: MAC_1..MAC_6
    assert_eq!(
        mpe.mac_address,
        MacAddress([0x44, 0x33, 0x22, 0x11, 0xBB, 0xAA])
    );
    assert!(mpe.payload.is_empty());
}

/// Every `TableId` variant maps to a byte covered by `AnyTableSection::DISPATCHED_RANGES`.
#[test]
fn every_table_id_variant_is_dispatched() {
    let ranges = dvb_si::tables::AnyTableSection::DISPATCHED_RANGES;
    for b in 0u8..=0xFF {
        if dvb_si::TableId::try_from(b).is_ok() {
            let covered = ranges.iter().any(|&(lo, hi)| b >= lo && b <= hi);
            assert!(
                covered,
                "TableId byte {b:#04x} is a known variant but is not covered by \
                 AnyTableSection::DISPATCHED_RANGES"
            );
        }
    }
}

/// `AnyTableSection::name()` reflects the contained type's `TableDef::NAME`.
#[test]
fn name_maps_variant_to_tabledef_name() {
    let pat = build_pat(0x0001, 0, &[(1, 0x0100)]);
    let table = AnyTableSection::parse(&pat).expect("valid PAT");
    assert_eq!(table.name(), "PROGRAM_ASSOCIATION");

    let unknown = AnyTableSection::parse(&[0x90, 0x01, 0x00]).expect("unknown ok");
    assert_eq!(unknown.name(), "UNKNOWN");
}
