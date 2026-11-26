//! SpliceInfoSection header packing: the 12-bit `tier` beside the 12-bit
//! `splice_command_length` across bytes 10-12, a multi-byte
//! `descriptor_loop_length`, `tier_is_ignored`, and every truncation boundary
//! (ANSI/SCTE 35 2023r1 §9.6.1, Table 5).

use broadcast_core::{Parse, Serialize};
use scte35_splice::Error;
use scte35_splice::commands::{AnyCommand, PrivateCommand};
use scte35_splice::descriptor_loop::SpliceDescriptorLoop;
use scte35_splice::section::{SpliceInfoSection, TIER_IGNORE};

const COMMAND_BODY_BYTES: usize = 300;
const DESCRIPTOR_LOOP_BYTES: usize = 300;

const COMMAND_BODY: [u8; COMMAND_BODY_BYTES] = [0xA5; COMMAND_BODY_BYTES];

const DESCRIPTOR_LOOP: [u8; DESCRIPTOR_LOOP_BYTES] = {
    let mut bytes = [0u8; DESCRIPTOR_LOOP_BYTES];
    let mut i = 0;
    while i < DESCRIPTOR_LOOP_BYTES {
        bytes[i] = 0x00;
        bytes[i + 1] = 0x01;
        bytes[i + 2] = 0x00;
        i += 3;
    }
    bytes
};

fn section_with_tier(tier: u16) -> SpliceInfoSection<'static> {
    let command = PrivateCommand {
        identifier: 0x4355_4549,
        private_bytes: &COMMAND_BODY,
    };
    let mut section = SpliceInfoSection::new_clear(
        AnyCommand::from(command),
        SpliceDescriptorLoop::new(&DESCRIPTOR_LOOP),
    );
    section.tier = tier;
    section
}

fn seal(bytes: &mut [u8]) {
    let crc_pos = bytes.len() - 4;
    let crc = broadcast_common::crc32_mpeg2::compute(&bytes[..crc_pos]);
    bytes[crc_pos..].copy_from_slice(&crc.to_be_bytes());
}

#[test]
fn tier_and_command_length_pack_across_bytes_ten_to_twelve() {
    let section = section_with_tier(0x123);
    let bytes = section.try_to_bytes().unwrap();
    assert_eq!(bytes.len(), 13 + 1 + 304 + 2 + 300 + 4);
    // tier 0x123 -> [0x12, 0x3_]; splice_command_length 304 (0x130) -> low
    // nibble of byte 11 and byte 12: [0x12, 0x31, 0x30].
    assert_eq!(&bytes[10..13], &[0x12, 0x31, 0x30]);
    assert_eq!(&bytes[13..14], &[0xFF], "private_command type");
    // descriptor_loop_length 300 = 0x012C.
    let dll_at = 13 + 1 + 304;
    assert_eq!(&bytes[dll_at..dll_at + 2], &[0x01, 0x2C]);

    let parsed = SpliceInfoSection::parse(&bytes).unwrap();
    assert_eq!(parsed, section);
    assert_eq!(parsed.tier, 0x123);
    assert!(!parsed.tier_is_ignored());
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}

#[test]
fn the_ignore_tier_is_reported_as_ignored() {
    let section = section_with_tier(TIER_IGNORE);
    assert_eq!(TIER_IGNORE, 0x0FFF);
    let bytes = section.try_to_bytes().unwrap();
    assert_eq!(&bytes[10..12], &[0xFF, 0xF1]);
    let parsed = SpliceInfoSection::parse(&bytes).unwrap();
    assert_eq!(parsed.tier, 0x0FFF);
    assert!(parsed.tier_is_ignored());
}

#[test]
fn every_truncation_of_the_section_is_an_error() {
    let bytes = section_with_tier(0x123).try_to_bytes().unwrap();
    for cut in [
        bytes.len() - 1,
        bytes.len() - 4,
        bytes.len() - 5,
        13 + 1 + 304,
        13 + 1 + 304 - 1,
        17,
        16,
    ] {
        assert!(
            SpliceInfoSection::parse(&bytes[..cut]).is_err(),
            "truncated to {cut} bytes must not parse"
        );
    }
}

#[test]
fn a_section_shorter_than_the_fixed_header_and_crc_is_rejected() {
    // 18 bytes present, but section_length says 10: total 13 < 18.
    let mut bytes = vec![0u8; 18];
    bytes[0] = 0xFC;
    bytes[2] = 10;
    assert_eq!(
        SpliceInfoSection::parse(&bytes),
        Err(Error::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: 18,
                have: 13,
                what: "splice_info_section body + CRC",
            }
        ))
    );
}

#[test]
fn an_eighteen_byte_section_reaches_the_crc_check() {
    // total == 18 exactly: the header guard must pass and the CRC decide. The
    // hand-built CRC is wrong, so the section is refused as CrcMismatch, not
    // as a too-short body.
    let mut bytes = vec![0u8; 18];
    bytes[0] = 0xFC;
    bytes[2] = 15;
    *bytes.last_mut().unwrap() = 1;
    assert!(matches!(
        SpliceInfoSection::parse(&bytes),
        Err(Error::CrcMismatch(_))
    ));
}

#[test]
fn a_command_cut_short_of_its_declared_length_reports_the_exact_need() {
    let mut bytes = section_with_tier(0x123).try_to_bytes().unwrap();
    bytes.truncate(20);
    let section_length = bytes.len() - 3;
    bytes[1] = (bytes[1] & 0xF0) | ((section_length >> 8) as u8 & 0x0F);
    bytes[2] = (section_length & 0xFF) as u8;
    seal(&mut bytes);
    assert_eq!(
        SpliceInfoSection::parse(&bytes),
        Err(Error::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: 1 + 304 + 2,
                have: 3,
                what: "splice_info_section command + descriptor_loop_length",
            }
        ))
    );
}

#[test]
fn a_short_buffer_below_the_fixed_header_is_rejected() {
    assert_eq!(
        SpliceInfoSection::parse(&[0xFC, 0x30, 0x00]),
        Err(Error::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: 18,
                have: 3,
                what: "splice_info_section header",
            }
        ))
    );
}

const ENCRYPTED_PAYLOAD: [u8; 300] = [0x5A; 300];

#[test]
fn an_encrypted_section_packs_a_multibyte_command_length_beside_the_tier() {
    let section = SpliceInfoSection {
        sap_type: 0x3,
        protocol_version: 0,
        encrypted_packet: true,
        encryption_algorithm: 1,
        pts_adjustment: 0,
        cw_index: 1,
        tier: 0x123,
        clear: None,
        encrypted_payload: Some(&ENCRYPTED_PAYLOAD),
        encrypted_splice_command_length: Some(300),
    };
    let bytes = section.try_to_bytes().unwrap();
    // tier 0x123 -> byte 10 = 0x12, low nibble 3; splice_command_length 300
    // (0x12C) -> byte 11 = 0x31, byte 12 = 0x2C.
    assert_eq!(&bytes[10..13], &[0x12, 0x31, 0x2C]);
    assert_eq!(&bytes[13..313], &ENCRYPTED_PAYLOAD);

    let parsed = SpliceInfoSection::parse(&bytes).unwrap();
    assert_eq!(parsed.tier, 0x123);
    assert_eq!(parsed.encrypted_splice_command_length, Some(300));
    assert_eq!(parsed.encrypted_payload, Some(&ENCRYPTED_PAYLOAD[..]));
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}
