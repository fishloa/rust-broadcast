//! Malformed-length error payloads: `Error::BufferTooShort { need, have, .. }`
//! and `Error::SectionLengthOverflow { declared, available }` are public API
//! (`dvb-tools` prints them), so each branch is pinned to the exact numbers
//! derived by hand from the byte layout — not just "it failed".

use broadcast_common::Parse;
use dvb_si::Error;
use dvb_si::carousel::messages::{MESSAGE_ID_DII, MESSAGE_ID_DSI};
use dvb_si::carousel::{GroupInfoIndication, UnMessage};
use dvb_si::tables::ait::AitSection;
use dvb_si::tables::cat::CatSection;
use dvb_si::tables::cit::CitSection;
use dvb_si::tables::nit::NitSection;
use dvb_si::tables::pat::PatSection;
use dvb_si::tables::pmt::PmtSection;
use dvb_si::tables::protection_message::ProtectionMessageSection;
use dvb_si::tables::rct::RctSection;
use dvb_si::tables::rnt::RntSection;
use dvb_si::tables::rst::RstSection;
use dvb_si::tables::sat::SatSection;
use dvb_si::tables::sdt::SdtSection;
use dvb_si::tables::sit::SitSection;
use dvb_si::tables::tot::TotSection;
use dvb_si::tables::unt::UntSection;

/// Fill in `section_length` from the final buffer size and append the CRC.
fn seal(mut bytes: Vec<u8>) -> Vec<u8> {
    let total = bytes.len() + 4;
    let section_length = (total - 3) as u16;
    bytes[1] = (bytes[1] & 0xF0) | ((section_length >> 8) as u8 & 0x0F);
    bytes[2] = (section_length & 0xFF) as u8;
    let crc = broadcast_common::crc32_mpeg2::compute(&bytes);
    bytes.extend_from_slice(&crc.to_be_bytes());
    bytes
}

fn too_short(need: usize, have: usize, what: &'static str) -> Error {
    Error::BufferTooShort(broadcast_common::error::BufferTooShort { need, have, what })
}

fn overflow(declared: usize, available: usize) -> Error {
    Error::SectionLengthOverflow {
        declared,
        available,
    }
}

fn nit(desc_len: u16, tail: &[u8]) -> Vec<u8> {
    let mut bytes = vec![
        0x40,
        0xF0,
        0x00,
        0x00,
        0x01,
        0xC1,
        0x00,
        0x00,
        0xF0 | ((desc_len >> 8) as u8 & 0x0F),
        (desc_len & 0xFF) as u8,
    ];
    bytes.extend_from_slice(tail);
    seal(bytes)
}

#[test]
fn nit_descriptor_loop_past_body_end() {
    // desc_start = 10, body_end = 12: a declared 4-byte common descriptor loop.
    let bytes = nit(4, &[0x00, 0x00]);
    assert_eq!(
        NitSection::parse(&bytes).unwrap_err(),
        overflow(4, 2),
        "declared common_descriptors_length 4, only 2 bytes before the CRC"
    );
}

#[test]
fn nit_descriptor_loop_reaching_body_end_leaves_no_loop_length_field() {
    // desc_end == body_end: the neighbouring check must report the missing
    // transport_stream_loop_length field, not a length overflow.
    let bytes = nit(2, &[0x00, 0x00]);
    assert_eq!(
        NitSection::parse(&bytes).unwrap_err(),
        too_short(2, 0, "NitSection transport_stream_loop")
    );
}

#[test]
fn nit_ts_loop_length_field_one_byte_short() {
    // desc_end = 11, body_end = 12: exactly one byte remains for a 2-byte field.
    let bytes = nit(1, &[0xAA, 0x00]);
    assert_eq!(
        NitSection::parse(&bytes).unwrap_err(),
        too_short(3, 1, "NitSection transport_stream_loop")
    );
}

#[test]
fn nit_ts_loop_past_body_end() {
    let bytes = nit(0, &[0xF0, 0x04]);
    assert_eq!(
        NitSection::parse(&bytes).unwrap_err(),
        overflow(4, 0),
        "declared transport_stream_loop_length 4 with nothing after the field"
    );
}

#[test]
fn nit_transport_stream_entry_header_short() {
    let bytes = nit(0, &[0xF0, 0x03, 0x00, 0x01, 0x00]);
    assert_eq!(
        NitSection::parse(&bytes).unwrap_err(),
        too_short(18, 15, "NitSection transport_stream_entry")
    );
}

#[test]
fn nit_transport_stream_entry_descriptors_past_loop_end() {
    let bytes = nit(0, &[0xF0, 0x06, 0x00, 0x01, 0x00, 0x02, 0xF0, 0x01]);
    assert_eq!(
        NitSection::parse(&bytes).unwrap_err(),
        overflow(1, 0),
        "transport_descriptors_length 1 after the entry header"
    );
}

#[test]
fn nit_minimal_section_accepted() {
    let bytes = nit(0, &[0xF0, 0x00]);
    let nit = NitSection::parse(&bytes).expect("minimal NIT");
    assert!(nit.transport_streams.is_empty());
}

fn ait(common_desc_len: u16, tail: &[u8]) -> Vec<u8> {
    let mut bytes = vec![
        0x74,
        0xF0,
        0x00,
        0x00,
        0x10,
        0xC1,
        0x00,
        0x00,
        0xF0 | ((common_desc_len >> 8) as u8 & 0x0F),
        (common_desc_len & 0xFF) as u8,
    ];
    bytes.extend_from_slice(tail);
    seal(bytes)
}

#[test]
fn ait_common_descriptors_past_application_loop_end() {
    let bytes = ait(3, &[0x00, 0x00]);
    assert_eq!(
        AitSection::parse(&bytes).unwrap_err(),
        overflow(3, 2),
        "common_descriptors_length 3 with 2 bytes before the CRC"
    );
}

#[test]
fn ait_common_descriptors_reaching_body_end_is_not_the_floor_error() {
    // common_desc_end == app_loop_end: the application-loop-length field then
    // starts inside the CRC, so the error must not be the floor overflow.
    let bytes = ait(2, &[0x00, 0x00]);
    assert_ne!(
        AitSection::parse(&bytes).unwrap_err(),
        overflow(2, 0),
        "an equality at the common-descriptor bound is not an overrun"
    );
}

#[test]
fn ait_application_loop_length_short_of_body_end() {
    // 18-byte section: app_loop_end = 14, loop declared 0 from byte 12.
    let bytes = ait(0, &[0xF0, 0x00, 0x00, 0x00]);
    assert_eq!(
        AitSection::parse(&bytes).unwrap_err(),
        too_short(2, 0, "AitSection trailing bytes after application loop")
    );
}

#[test]
fn ait_partial_application_header_in_loop() {
    // Application loop length 3: three bytes cannot hold a 9-byte app header.
    let bytes = ait(0, &[0xF0, 0x03, 0x00, 0x00, 0x00]);
    assert_eq!(
        AitSection::parse(&bytes).unwrap_err(),
        too_short(3, 0, "AitSection trailing application bytes")
    );
}

#[test]
fn cat_ca_descriptors_truncated_descriptor() {
    let bytes = seal(vec![
        0x01, 0xF0, 0x00, 0x00, 0x00, 0xC1, 0x00, 0x00, 0x09, 0x05, 0x00,
    ]);
    let cat = CatSection::parse(&bytes).expect("parse");
    assert_eq!(
        cat.ca_descriptors().unwrap_err(),
        too_short(7, 3, "CatSection ca_descriptors truncated descriptor")
    );
}

#[test]
fn cit_partial_crid_entry_header() {
    // 19-byte section: payload_end = 15, one 2-byte partial CRID entry at 13.
    let bytes = seal(vec![
        0x77, 0xF0, 0x00, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00,
    ]);
    assert_eq!(
        CitSection::parse(&bytes).unwrap_err(),
        too_short(17, 15, "CitSection crid_entry")
    );
}

#[test]
fn pat_trailing_entry_bytes() {
    // 17-byte PAT: entries end at byte 13 with one leftover byte.
    let bytes = seal(vec![
        0x00, 0xF0, 0x00, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xE0, 0x00, 0xAA,
    ]);
    assert_eq!(
        PatSection::parse(&bytes).unwrap_err(),
        too_short(4, 1, "PatSection trailing entry bytes")
    );
}

#[test]
fn pmt_trailing_stream_bytes() {
    // 17-byte PMT: stream loop starts at 12 with one leftover byte.
    let bytes = seal(vec![
        0x02, 0xF0, 0x00, 0x00, 0x01, 0xC1, 0x00, 0x00, 0xE0, 0x00, 0xF0, 0x00, 0xAA,
    ]);
    assert_eq!(
        PmtSection::parse(&bytes).unwrap_err(),
        too_short(1, 0, "PmtSection trailing stream bytes")
    );
}

fn protection(extension: u16, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![
        0x7B,
        0xF0,
        0x00,
        (extension >> 8) as u8,
        (extension & 0xFF) as u8,
        0xC1,
        0x00,
        0x00,
    ];
    bytes.extend_from_slice(body);
    seal(bytes)
}

#[test]
fn protection_hash_loop_past_body_end() {
    let bytes = protection(0x0001, &[0x00, 0x00, 0x00, 0xF0, 0x10]);
    assert_eq!(
        ProtectionMessageSection::parse(&bytes).unwrap_err(),
        overflow(0x010, 0)
    );
}

#[test]
fn protection_hash_loop_reaching_body_end_is_not_the_floor_error() {
    let bytes = protection(0x0001, &[0x00, 0x00, 0x00, 0xF0, 0x00]);
    assert_eq!(
        ProtectionMessageSection::parse(&bytes).unwrap_err(),
        too_short(6, 5, "ProtectionMessageSection::extension_bytes_length")
    );
}

#[test]
fn protection_hash_entry_past_loop_end() {
    let bytes = protection(0x0001, &[0x01, 0x00, 0x00, 0xF0, 0x01, 0x12]);
    assert_eq!(
        ProtectionMessageSection::parse(&bytes).unwrap_err(),
        overflow(2, 0)
    );
}

#[test]
fn protection_extension_bytes_past_body_end() {
    let bytes = protection(0x0001, &[0x00, 0x00, 0x00, 0xF0, 0x00, 0x04, 0xAA]);
    assert_eq!(
        ProtectionMessageSection::parse(&bytes).unwrap_err(),
        overflow(4, 1)
    );
}

#[test]
fn protection_key_id_length_field_inside_crc_region() {
    let bytes = protection(0x0001, &[0x00, 0x00, 0x00, 0xF0, 0x00, 0x01, 0xAA]);
    assert_eq!(
        ProtectionMessageSection::parse(&bytes).unwrap_err(),
        too_short(
            8,
            7,
            "ProtectionMessageSection::signature_key_identifier_length"
        )
    );
}

#[test]
fn protection_key_id_past_body_end() {
    let bytes = protection(0x0001, &[0x00, 0x00, 0x00, 0xF0, 0x00, 0x00, 0x03, 0xAA]);
    assert_eq!(
        ProtectionMessageSection::parse(&bytes).unwrap_err(),
        overflow(3, 1)
    );
}

#[test]
fn protection_certificate_past_body_end() {
    let bytes = protection(0x0100, &[0xF1, 0xF0, 0x03, 0xAA]);
    assert_eq!(
        ProtectionMessageSection::parse(&bytes).unwrap_err(),
        overflow(3, 1)
    );
}

#[test]
fn protection_certificate_trailing_bytes() {
    let bytes = protection(0x0100, &[0xF1, 0xF0, 0x01, 0xAA, 0xBB]);
    assert_eq!(
        ProtectionMessageSection::parse(&bytes).unwrap_err(),
        too_short(
            1,
            0,
            "ProtectionMessageSection::CertificateCollection trailing bytes"
        )
    );
}

fn rnt(common_desc_len: u16, rest: &[u8]) -> Vec<u8> {
    let mut bytes = vec![
        0x79,
        0xF0,
        0x00,
        0x00,
        0x01,
        0xC1,
        0x00,
        0x00,
        0x00,
        0xF0 | ((common_desc_len >> 8) as u8 & 0x0F),
        (common_desc_len & 0xFF) as u8,
    ];
    bytes.extend_from_slice(rest);
    seal(bytes)
}

#[test]
fn rnt_common_descriptors_past_body_end() {
    assert_eq!(RntSection::parse(&rnt(3, &[])).unwrap_err(), overflow(3, 0));
}

#[test]
fn rnt_resolution_provider_length_field_short() {
    assert_eq!(
        RntSection::parse(&rnt(0, &[0xAA])).unwrap_err(),
        too_short(13, 12, "RntSection resolution_provider_info_length")
    );
}

#[test]
fn rnt_resolution_provider_past_body_end() {
    // rp length field at 11, rp declared 4, payload ends at 16.
    assert_eq!(
        RntSection::parse(&rnt(0, &[0xF0, 0x04, 0xAA, 0xBB, 0xCC])).unwrap_err(),
        overflow(4, 3)
    );
}

#[test]
fn rnt_resolution_provider_name_length_short() {
    assert_eq!(
        RntSection::parse(&rnt(0, &[0xF0, 0x00, 0xAA])).unwrap_err(),
        too_short(14, 13, "RntSection resolution_provider_name_length")
    );
}

#[test]
fn rnt_resolution_provider_name_past_provider_end() {
    assert_eq!(
        RntSection::parse(&rnt(0, &[0xF0, 0x02, 0x03, 0xAA])).unwrap_err(),
        too_short(17, 15, "RntSection resolution_provider_name")
    );
}

#[test]
fn rnt_resolution_provider_descriptors_length_short() {
    assert_eq!(
        RntSection::parse(&rnt(0, &[0xF0, 0x02, 0x00, 0xAA])).unwrap_err(),
        too_short(16, 15, "RntSection resolution_provider_descriptors_length")
    );
}

#[test]
fn rnt_resolution_provider_descriptors_past_provider_end() {
    let bytes = rnt(0, &[0xF0, 0x04, 0x00, 0xF0, 0x03, 0xAA]);
    assert_eq!(RntSection::parse(&bytes).unwrap_err(), overflow(3, 1));
}

#[test]
fn rnt_crid_authority_name_past_provider_end() {
    let bytes = rnt(0, &[0xF0, 0x04, 0x00, 0xF0, 0x00, 0x02]);
    assert_eq!(
        RntSection::parse(&bytes).unwrap_err(),
        too_short(19, 17, "RntSection CRID_authority_name")
    );
}

#[test]
fn rnt_crid_authority_header_short() {
    let bytes = rnt(0, &[0xF0, 0x05, 0x00, 0xF0, 0x00, 0x00, 0xAA]);
    assert_eq!(
        RntSection::parse(&bytes).unwrap_err(),
        too_short(19, 18, "RntSection CRID_authority header")
    );
}

#[test]
fn rnt_crid_authority_descriptors_past_provider_end() {
    let bytes = rnt(0, &[0xF0, 0x06, 0x00, 0xF0, 0x00, 0x00, 0xF0, 0x03]);
    assert_eq!(RntSection::parse(&bytes).unwrap_err(), overflow(3, 0));
}

#[test]
fn rst_declared_length_longer_than_buffer() {
    let bytes = vec![0x71, 0xF0, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00];
    assert_eq!(RstSection::parse(&bytes).unwrap_err(), overflow(8, 5));
}

#[test]
fn rst_unaligned_entry_length() {
    let bytes = vec![0x71, 0xF0, 0x03, 0x00, 0x00, 0x00];
    assert_eq!(
        RstSection::parse(&bytes).unwrap_err(),
        too_short(9, 3, "RstSection entry alignment")
    );
}

#[test]
fn tot_descriptors_past_body_end() {
    let bytes = seal(vec![
        0x73, 0xF0, 0x00, 0xE4, 0x09, 0x12, 0x34, 0x56, 0xF0, 0x01,
    ]);
    assert_eq!(TotSection::parse(&bytes).unwrap_err(), overflow(1, 0));
}

#[test]
fn sdt_shorter_than_minimum() {
    let bytes = vec![
        0x42, 0xF0, 0x0F, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xFF, 0x00, 0x00, 0x00,
    ];
    assert_eq!(
        SdtSection::parse(&bytes).unwrap_err(),
        too_short(15, 14, "SdtSection")
    );
}

#[test]
fn sdt_trailing_service_bytes() {
    let bytes = seal(vec![
        0x42, 0xF0, 0x00, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xFF, 0xAA,
    ]);
    assert_eq!(
        SdtSection::parse(&bytes).unwrap_err(),
        too_short(1, 0, "SdtSection trailing service bytes")
    );
}

#[test]
fn sit_shorter_than_minimum() {
    let bytes = vec![
        0x7F, 0xF0, 0x0E, 0x00, 0x01, 0xC1, 0x00, 0x00, 0xF0, 0x00, 0x00, 0x00, 0x00,
    ];
    assert_eq!(
        SitSection::parse(&bytes).unwrap_err(),
        too_short(14, 13, "SitSection")
    );
}

#[test]
fn sit_service_header_short() {
    let bytes = seal(vec![
        0x7F, 0xF0, 0x00, 0x00, 0x01, 0xC1, 0x00, 0x00, 0xF0, 0x00, 0x00,
    ]);
    assert_eq!(
        SitSection::parse(&bytes).unwrap_err(),
        too_short(14, 11, "SitService header")
    );
}

#[test]
fn sit_service_descriptors_past_body_end() {
    let bytes = seal(vec![
        0x7F, 0xF0, 0x00, 0x00, 0x01, 0xC1, 0x00, 0x00, 0xF0, 0x00, 0x00, 0x00, 0xF0, 0x01,
    ]);
    assert_eq!(SitSection::parse(&bytes).unwrap_err(), overflow(1, 0));
}

fn rct(link_count: u8, rest: &[u8]) -> Vec<u8> {
    let mut bytes = vec![
        0x76, 0xB0, 0x00, 0x00, 0x64, 0xC1, 0x00, 0x00, 0x07, 0xD3, link_count,
    ];
    bytes.extend_from_slice(rest);
    seal(bytes)
}

fn rct_link(link: &[u8]) -> Vec<u8> {
    let mut rest = vec![
        0xF0 | ((link.len() >> 8) as u8 & 0x0F),
        (link.len() & 0xFF) as u8,
    ];
    rest.extend_from_slice(link);
    rest.extend_from_slice(&[0xF0, 0x00]);
    rct(1, &rest)
}

#[test]
fn rct_descriptor_loop_length_field_short() {
    // One minimal 7-byte link consumes the whole payload, leaving no room for
    // the trailing descriptor_loop_length field.
    assert_eq!(
        RctSection::parse(&rct(
            1,
            &[0xF0, 0x07, 0x30, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00]
        ))
        .unwrap_err(),
        too_short(22, 20, "RctSection descriptor_loop_length field")
    );
}

#[test]
fn rct_link_entry_past_payload_end() {
    assert_eq!(
        RctSection::parse(&rct(1, &[0xF0, 0x04])).unwrap_err(),
        overflow(4, 0)
    );
}

#[test]
fn rct_descriptor_loop_past_payload_end() {
    assert_eq!(
        RctSection::parse(&rct(0, &[0xF0, 0x03, 0xAA])).unwrap_err(),
        overflow(3, 1)
    );
}

#[test]
fn rct_trailing_bytes_after_descriptor_loop() {
    assert_eq!(
        RctSection::parse(&rct(0, &[0xF0, 0x00, 0xAA])).unwrap_err(),
        too_short(1, 0, "RctSection trailing bytes after descriptor loop")
    );
}

#[test]
fn rct_media_uri_length_missing() {
    let link = [0x0C, 0x00, 0x10, 0x00];
    assert_eq!(
        RctSection::parse(&rct_link(&link)).unwrap_err(),
        too_short(5, 4, "RctSection media_uri_length")
    );
}

#[test]
fn rct_media_uri_past_link_end() {
    let link = [0x0C, 0x00, 0x10, 0x00, 0x05];
    assert_eq!(
        RctSection::parse(&rct_link(&link)).unwrap_err(),
        too_short(10, 5, "RctSection media_uri")
    );
}

#[test]
fn rct_binary_locator_missing() {
    let link = [0x1F, 0x00, 0x10, 0x00];
    assert_eq!(
        RctSection::parse(&rct_link(&link)).unwrap_err(),
        too_short(5, 4, "RctSection dvb_binary_locator")
    );
}

#[test]
fn rct_number_items_missing() {
    let link = [0x3C, 0x00, 0x10, 0x00];
    assert_eq!(
        RctSection::parse(&rct_link(&link)).unwrap_err(),
        too_short(5, 4, "RctSection number_items")
    );
}

#[test]
fn rct_link_item_header_short() {
    let link = [0x3C, 0x00, 0x10, 0x00, 0xC1];
    assert_eq!(
        RctSection::parse(&rct_link(&link)).unwrap_err(),
        too_short(9, 5, "RctSection link_item")
    );
}

#[test]
fn rct_promotional_text_past_link_end() {
    let link = [0x3C, 0x00, 0x10, 0x00, 0xC1, b'e', b'n', b'g', 0x03];
    assert_eq!(
        RctSection::parse(&rct_link(&link)).unwrap_err(),
        too_short(12, 9, "RctSection promotional_text")
    );
}

#[test]
fn rct_icon_flag_short() {
    let link = [0x3C, 0x00, 0x10, 0x00, 0xC0];
    assert_eq!(
        RctSection::parse(&rct_link(&link)).unwrap_err(),
        too_short(7, 5, "RctSection icon/desc")
    );
}

#[test]
fn rct_link_descriptors_past_link_end() {
    let link = [0x3C, 0x00, 0x10, 0x00, 0xC0, 0x00, 0x03];
    assert_eq!(
        RctSection::parse(&rct_link(&link)).unwrap_err(),
        overflow(3, 0)
    );
}

#[test]
fn rct_link_info_trailing_bytes() {
    let link = [0x3C, 0x00, 0x10, 0x00, 0xC0, 0x00, 0x00, 0xAA];
    assert_eq!(
        RctSection::parse(&rct_link(&link)).unwrap_err(),
        too_short(1, 0, "RctSection link_info trailing bytes")
    );
}

fn unt(common_desc_len: u16, rest: &[u8]) -> Vec<u8> {
    let mut bytes = vec![
        0x4B,
        0xF0,
        0x00,
        0x01,
        0x00,
        0xC1,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0xF0 | ((common_desc_len >> 8) as u8 & 0x0F),
        (common_desc_len & 0xFF) as u8,
    ];
    bytes.extend_from_slice(rest);
    seal(bytes)
}

#[test]
fn unt_common_descriptors_past_body_end() {
    assert_eq!(UntSection::parse(&unt(1, &[])).unwrap_err(), overflow(1, 0));
}

#[test]
fn unt_compatibility_descriptor_length_short() {
    assert_eq!(
        UntSection::parse(&unt(0, &[0xAA])).unwrap_err(),
        too_short(16, 15, "UntSection compatibilityDescriptorLength")
    );
}

#[test]
fn unt_compatibility_descriptor_past_body_end() {
    assert_eq!(
        UntSection::parse(&unt(0, &[0x00, 0x03])).unwrap_err(),
        overflow(3, 0)
    );
}

#[test]
fn unt_platform_loop_length_short() {
    assert_eq!(
        UntSection::parse(&unt(0, &[0x00, 0x00, 0xAA])).unwrap_err(),
        too_short(18, 17, "UntSection platform_loop_length")
    );
}

#[test]
fn unt_platform_loop_past_body_end() {
    assert_eq!(
        UntSection::parse(&unt(0, &[0x00, 0x00, 0x00, 0x04])).unwrap_err(),
        overflow(4, 0)
    );
}

#[test]
fn unt_target_descriptor_loop_length_short() {
    assert_eq!(
        UntSection::parse(&unt(0, &[0x00, 0x00, 0x00, 0x01, 0xAA])).unwrap_err(),
        too_short(20, 19, "UntSection target_descriptor_loop length")
    );
}

#[test]
fn unt_target_descriptor_loop_past_platform_end() {
    let rest = [0x00, 0x00, 0x00, 0x03, 0xF0, 0x03, 0xAA];
    assert_eq!(
        UntSection::parse(&unt(0, &rest)).unwrap_err(),
        overflow(3, 1)
    );
}

#[test]
fn unt_operational_descriptor_loop_length_short() {
    let rest = [0x00, 0x00, 0x00, 0x03, 0xF0, 0x00, 0xAA];
    assert_eq!(
        UntSection::parse(&unt(0, &rest)).unwrap_err(),
        too_short(22, 21, "UntSection operational_descriptor_loop length")
    );
}

#[test]
fn unt_operational_descriptor_loop_past_platform_end() {
    let rest = [0x00, 0x00, 0x00, 0x05, 0xF0, 0x00, 0xF0, 0x03, 0xAA];
    assert_eq!(
        UntSection::parse(&unt(0, &rest)).unwrap_err(),
        overflow(3, 1)
    );
}

fn sat(satellite_table_id: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![
        0x4D,
        0xF0,
        0x00,
        satellite_table_id << 2,
        0x00,
        0xC1,
        0x00,
        0x00,
        0x00,
    ];
    bytes.extend_from_slice(body);
    seal(bytes)
}

#[test]
fn sat_position_v2_orbital_fields_short() {
    // 5-byte body: 24-bit id + 7 reserved + 1 position_system, 8 bits left of
    // the 24 an orbital position needs.
    assert_eq!(
        SatSection::parse(&sat(0, &[0, 0, 0, 0x00, 0x00])).unwrap_err(),
        too_short(3, 1, "SatSection PositionV2 Orbital fields")
    );
}

#[test]
fn sat_position_v2_sgp4_fields_short() {
    assert_eq!(
        SatSection::parse(&sat(0, &[0, 0, 0, 0x01, 0x00])).unwrap_err(),
        too_short(43, 1, "SatSection PositionV2 SGP4 fields")
    );
}

#[test]
fn sat_cell_fragment_centre_short() {
    // first_occurrence set: 6 bits left of the 76-bit centre block.
    assert_eq!(
        SatSection::parse(&sat(1, &[0, 0, 0, 0x00, 0x80])).unwrap_err(),
        too_short(10, 0, "SatSection CellFragment centre")
    );
}

#[test]
fn sat_cell_fragment_delivery_system_ids_short() {
    // dsid_count = 1 with no bits left.
    assert_eq!(
        SatSection::parse(&sat(1, &[0, 0, 0, 0x00, 0x00, 0x01])).unwrap_err(),
        too_short(4, 0, "SatSection CellFragment delivery_system_ids")
    );
}

#[test]
fn sat_cell_fragment_new_delivery_systems_short() {
    // nds_count = 1 with no bits left of the 129-bit entry.
    assert_eq!(
        SatSection::parse(&sat(1, &[0, 0, 0, 0x00, 0x00, 0x00, 0x00, 0x01])).unwrap_err(),
        too_short(10, 0, "SatSection CellFragment new_delivery_systems")
    );
}

#[test]
fn sat_cell_fragment_obsolescent_delivery_systems_short() {
    // ods_count = 1 with no bits left of the 129-bit entry.
    assert_eq!(
        SatSection::parse(&sat(
            1,
            &[0, 0, 0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]
        ))
        .unwrap_err(),
        too_short(
            10,
            0,
            "SatSection CellFragment obsolescent_delivery_systems"
        )
    );
}

#[test]
fn sat_time_association_body_short() {
    assert_eq!(
        SatSection::parse(&sat(2, &[0x00])).unwrap_err(),
        too_short(19, 1, "SatSection TimeAssociation body")
    );
}

#[test]
fn sat_beamhopping_plan_header_short() {
    // 18-byte body: the header reads reach bit 150 of the 144 available.
    assert_eq!(
        SatSection::parse(&sat(3, &[0x00; 18])).unwrap_err(),
        too_short(19, 18, "SatSection bit reader overrun")
    );
}

#[test]
fn sat_beamhopping_mode0_short() {
    assert_eq!(
        SatSection::parse(&sat(3, &[0x00; 19])).unwrap_err(),
        too_short(12, 0, "SatSection Beamhopping Mode0")
    );
}

#[test]
fn sat_beamhopping_mode1_header_short() {
    let mut body = [0x00u8; 19];
    body[6] = 0x01;
    assert_eq!(
        SatSection::parse(&sat(3, &body)).unwrap_err(),
        too_short(4, 0, "SatSection Beamhopping Mode1 header")
    );
}

#[test]
fn sat_beamhopping_mode1_bitmap_short() {
    let mut body = [0x00u8; 23];
    body[6] = 0x01;
    body[20] = 0x08;
    assert_eq!(
        SatSection::parse(&sat(3, &body)).unwrap_err(),
        too_short(1, 0, "SatSection Beamhopping Mode1 bitmap")
    );
}

#[test]
fn sat_beamhopping_mode2_short() {
    let mut body = [0x00u8; 19];
    body[6] = 0x02;
    assert_eq!(
        SatSection::parse(&sat(3, &body)).unwrap_err(),
        too_short(24, 0, "SatSection Beamhopping Mode2")
    );
}

#[test]
fn sat_position_v3_header_short() {
    assert_eq!(
        SatSection::parse(&sat(4, &[0x00])).unwrap_err(),
        too_short(8, 1, "SatSection PositionV3 body header")
    );
}

#[test]
fn sat_position_v3_metadata_short() {
    let mut body = [0x00u8; 12];
    body[11] = 0x10;
    assert_eq!(
        SatSection::parse(&sat(4, &body)).unwrap_err(),
        too_short(15, 0, "SatSection PositionV3 metadata")
    );
}

#[test]
fn sat_position_v3_usable_start_time_short() {
    let mut body = [0x00u8; 27];
    body[11] = 0x18;
    assert_eq!(
        SatSection::parse(&sat(4, &body)).unwrap_err(),
        too_short(7, 0, "SatSection PositionV3 usable_start_time")
    );
}

#[test]
fn sat_position_v3_usable_stop_time_short() {
    let mut body = [0x00u8; 27];
    body[11] = 0x14;
    assert_eq!(
        SatSection::parse(&sat(4, &body)).unwrap_err(),
        too_short(7, 0, "SatSection PositionV3 usable_stop_time")
    );
}

#[test]
fn sat_position_v3_ephemeris_entry_short() {
    let mut body = [0x00u8; 14];
    body[13] = 0x01;
    assert_eq!(
        SatSection::parse(&sat(4, &body)).unwrap_err(),
        too_short(31, 0, "SatSection PositionV3 ephemeris_data entry")
    );
}

#[test]
fn sat_position_v3_covariance_short() {
    let mut body = [0x00u8; 14];
    body[11] = 0x01;
    assert_eq!(
        SatSection::parse(&sat(4, &body)).unwrap_err(),
        too_short(91, 0, "SatSection PositionV3 covariance")
    );
}

fn group_info(bytes: &[u8]) -> Vec<u8> {
    bytes.to_vec()
}

#[test]
fn group_info_entry_header_short() {
    assert_eq!(
        GroupInfoIndication::parse(&group_info(&[0x00, 0x01, 0, 0, 0, 1])).unwrap_err(),
        too_short(10, 6, "GroupInfo GroupId/GroupSize")
    );
}

#[test]
fn group_info_compatibility_length_short() {
    assert_eq!(
        GroupInfoIndication::parse(&group_info(&[0x00, 0x01, 0, 0, 0, 1, 0, 0, 0, 2])).unwrap_err(),
        too_short(12, 10, "GroupCompatibility length field")
    );
}

#[test]
fn group_info_compatibility_past_end() {
    assert_eq!(
        GroupInfoIndication::parse(&group_info(&[
            0x00, 0x01, 0, 0, 0, 1, 0, 0, 0, 2, 0x00, 0x04
        ]))
        .unwrap_err(),
        overflow(4, 0)
    );
}

#[test]
fn group_info_length_field_short() {
    assert_eq!(
        GroupInfoIndication::parse(&group_info(&[
            0x00, 0x01, 0, 0, 0, 1, 0, 0, 0, 2, 0x00, 0x00, 0xAA
        ]))
        .unwrap_err(),
        too_short(14, 13, "GroupInfo GroupInfoLength")
    );
}

#[test]
fn group_info_bytes_past_end() {
    assert_eq!(
        GroupInfoIndication::parse(&group_info(&[
            0x00, 0x01, 0, 0, 0, 1, 0, 0, 0, 2, 0x00, 0x00, 0x00, 0x04, 0xAA, 0xAA
        ]))
        .unwrap_err(),
        overflow(4, 2)
    );
}

#[test]
fn group_info_private_data_length_field_short() {
    assert_eq!(
        GroupInfoIndication::parse(&group_info(&[
            0x00, 0x01, 0, 0, 0, 1, 0, 0, 0, 2, 0x00, 0x00, 0x00, 0x01, 0xAA, 0xAA
        ]))
        .unwrap_err(),
        too_short(17, 16, "GroupInfo PrivateDataLength")
    );
}

#[test]
fn group_info_private_data_past_end() {
    assert_eq!(
        GroupInfoIndication::parse(&group_info(&[
            0x00, 0x01, 0, 0, 0, 1, 0, 0, 0, 2, 0x00, 0x00, 0x00, 0x01, 0xAA, 0x00, 0x03, 0xAA,
            0xAA
        ]))
        .unwrap_err(),
        overflow(3, 2)
    );
}

fn un_message(message_id: u16, adaptation_length: u8, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![
        0x11,
        0x03,
        (message_id >> 8) as u8,
        (message_id & 0xFF) as u8,
        0x00,
        0x00,
        0x00,
        0x01,
        0x00,
        adaptation_length,
        0x00,
        0x00,
    ];
    bytes[10..12].copy_from_slice(&(body.len() as u16).to_be_bytes());
    bytes.extend_from_slice(body);
    bytes
}

#[test]
fn un_message_length_past_buffer() {
    let mut bytes = un_message(MESSAGE_ID_DII, 0, &[]);
    bytes[10..12].copy_from_slice(&5u16.to_be_bytes());
    assert_eq!(UnMessage::parse(&bytes).unwrap_err(), overflow(5, 0));
}

#[test]
fn un_message_adaptation_longer_than_message() {
    let bytes = un_message(MESSAGE_ID_DII, 3, &[0xAA]);
    assert_eq!(UnMessage::parse(&bytes).unwrap_err(), overflow(3, 1));
}

#[test]
fn dsi_compatibility_length_field_short() {
    let body = [0xAAu8; 21];
    let bytes = un_message(MESSAGE_ID_DSI, 0, &body);
    assert_eq!(
        UnMessage::parse(&bytes).unwrap_err(),
        too_short(22, 21, "compatibilityDescriptor in DSM-CC message")
    );
}

#[test]
fn dsi_compatibility_past_end() {
    let mut body = [0xAAu8; 23];
    body[20] = 0x00;
    body[21] = 0x04;
    let bytes = un_message(MESSAGE_ID_DSI, 0, &body);
    assert_eq!(UnMessage::parse(&bytes).unwrap_err(), overflow(4, 1));
}

#[test]
fn dsi_private_data_length_field_short() {
    let mut body = [0xAAu8; 23];
    body[20] = 0x00;
    body[21] = 0x00;
    let bytes = un_message(MESSAGE_ID_DSI, 0, &body);
    assert_eq!(
        UnMessage::parse(&bytes).unwrap_err(),
        too_short(24, 23, "DSM-CC 16-bit length field")
    );
}

#[test]
fn dsi_private_data_past_end() {
    let mut body = [0xAAu8; 25];
    body[20] = 0x00;
    body[21] = 0x00;
    body[22] = 0x00;
    body[23] = 0x03;
    let bytes = un_message(MESSAGE_ID_DSI, 0, &body);
    assert_eq!(UnMessage::parse(&bytes).unwrap_err(), overflow(3, 1));
}

#[test]
fn dii_module_count_field_short() {
    let mut body = [0x00u8; 18];
    body[16] = 0x00;
    body[17] = 0x00;
    let bytes = un_message(MESSAGE_ID_DII, 0, &body);
    assert_eq!(
        UnMessage::parse(&bytes).unwrap_err(),
        too_short(20, 18, "Dii numberOfModules")
    );
}

#[test]
fn dii_module_entry_short() {
    let mut body = [0x00u8; 27];
    body[16] = 0x00;
    body[17] = 0x00;
    body[18] = 0x00;
    body[19] = 0x01;
    let bytes = un_message(MESSAGE_ID_DII, 0, &body);
    assert_eq!(
        UnMessage::parse(&bytes).unwrap_err(),
        too_short(28, 27, "Dii module entry")
    );
}

#[test]
fn dii_module_info_past_end() {
    let mut body = [0x00u8; 29];
    body[16] = 0x00;
    body[17] = 0x00;
    body[18] = 0x00;
    body[19] = 0x01;
    body[27] = 0x03;
    let bytes = un_message(MESSAGE_ID_DII, 0, &body);
    assert_eq!(UnMessage::parse(&bytes).unwrap_err(), overflow(3, 1));
}

// MARKER
