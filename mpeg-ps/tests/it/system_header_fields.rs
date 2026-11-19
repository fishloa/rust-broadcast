//! Field-by-field system header (ISO/IEC 13818-1 §2.5.3.5, Table 2-40):
//! distinct flag values, the multi-entry stream loop (normal and 6-byte
//! extended entries), and every marker bit.

use broadcast_common::{Parse, Serialize};
use mpeg_ps::{Error, StdBufferBound, SystemHeader};

/// Hand-assembled Table 2-40 bytes:
/// `rate_bound` 0x2A5A5, `audio_bound` 0x15, fixed=1, CSPS=0, audio_lock=1,
/// video_lock=1, `video_bound` 0x1B, `packet_rate_restriction`=1,
/// three P-STD bounds (0xE0 scale=0 size=0x1ABC, 0xC0 scale=1 size=0x0123,
/// 0xB7 extension 0x45 scale=0 size=0x1555) and two trailing reserved bytes.
const WIRE: &[u8] = &[
    0x00, 0x00, 0x01, 0xBB, // system_header_start_code
    0x00, 0x14, // header_length = 20 (6 fixed + 12 loop + 2 trailing)
    0x85, 0x4B, 0x4B, // marker + rate_bound 0x2A5A5
    0x56, // audio_bound 0x15 | fixed 1 | CSPS 0
    0xFB, // audio_lock 1 | video_lock 1 | marker | video_bound 0x1B
    0xFF, // packet_rate_restriction 1 | reserved 0x7F
    0xE0, 0xDA, 0xBC, // 0xE0 scale=0 size=0x1ABC
    0xC0, 0xE1, 0x23, // 0xC0 scale=1 size=0x0123
    0xB7, 0xC0, 0x45, 0xB6, 0xD5, 0x55, // 0xB7 extension 0x45 scale=0 size=0x1555
    0x00, 0x11, // trailing reserved
];

fn expected() -> SystemHeader {
    SystemHeader {
        rate_bound: 0x2A5A5,
        audio_bound: 0x15,
        fixed_flag: true,
        csps_flag: false,
        system_audio_lock_flag: true,
        system_video_lock_flag: true,
        video_bound: 0x1B,
        packet_rate_restriction_flag: true,
        reserved_bits: 0x7F,
        std_buffer_bounds: vec![
            StdBufferBound {
                stream_id: 0xE0,
                stream_id_extension: None,
                buffer_bound_scale: false,
                buffer_size_bound: 0x1ABC,
            },
            StdBufferBound {
                stream_id: 0xC0,
                stream_id_extension: None,
                buffer_bound_scale: true,
                buffer_size_bound: 0x0123,
            },
            StdBufferBound {
                stream_id: 0xB7,
                stream_id_extension: Some(0x45),
                buffer_bound_scale: false,
                buffer_size_bound: 0x1555,
            },
        ],
        trailing_reserved: vec![0x00, 0x11],
    }
}

#[test]
fn every_field_round_trips_against_hand_computed_bytes() {
    let header = expected();
    let mut out = vec![0u8; header.serialized_len()];
    assert_eq!(header.serialize_into(&mut out).unwrap(), WIRE.len());
    assert_eq!(out, WIRE, "serialize must reproduce Table 2-40 bytes");

    assert_eq!(SystemHeader::parse(WIRE).unwrap(), header);
}

#[test]
fn each_marker_bit_is_rejected() {
    let cases: &[(usize, u8, &'static str)] = &[
        (6, 0x80, "system_header rate_bound marker 1"),
        (8, 0x01, "system_header rate_bound marker 2"),
        (10, 0x20, "system_header video_bound marker"),
    ];
    for &(index, mask, message) in cases {
        let mut bytes = WIRE.to_vec();
        bytes[index] &= !mask;
        assert_eq!(
            SystemHeader::parse(&bytes).unwrap_err(),
            Error::BadMarker(message),
            "marker at byte {index}"
        );
    }
}

#[test]
fn a_truncated_extended_entry_is_reported() {
    // Fixed body + one complete 3-byte entry, then 0xB7 with only four more
    // bytes (the 6-byte extension form needs five after the stream_id).
    let bytes: &[u8] = &[
        0x00, 0x00, 0x01, 0xBB, 0x00, 0x0D, 0x80, 0x00, 0x01, 0x04, 0x20, 0x00, 0xE0, 0xC0, 0x00,
        0xB7, 0xC0, 0x45, 0xB6,
    ];
    assert!(matches!(
        SystemHeader::parse(bytes),
        Err(Error::BufferTooShort(_))
    ));
}

#[test]
fn a_truncated_normal_entry_is_reported() {
    // Fixed body + 0xE0 and one more byte only.
    let bytes: &[u8] = &[
        0x00, 0x00, 0x01, 0xBB, 0x00, 0x08, 0x80, 0x00, 0x01, 0x04, 0x20, 0x00, 0xE0, 0xC0,
    ];
    assert!(matches!(
        SystemHeader::parse(bytes),
        Err(Error::BufferTooShort(_))
    ));
}

#[test]
fn a_normal_entry_with_a_bad_scale_prefix_is_rejected() {
    let bytes: &[u8] = &[
        0x00, 0x00, 0x01, 0xBB, 0x00, 0x09, 0x80, 0x00, 0x01, 0x04, 0x20, 0x00, 0xE0, 0x80, 0x00,
    ];
    assert_eq!(
        SystemHeader::parse(bytes).unwrap_err(),
        Error::BadMarker("P-STD_buffer_bound_scale prefix")
    );
}

#[test]
fn a_fixed_body_shorter_than_six_bytes_names_the_exact_need() {
    let bytes: &[u8] = &[0x00, 0x00, 0x01, 0xBB, 0x00, 0x03, 0x80, 0x00, 0x01];
    assert_eq!(
        SystemHeader::parse(bytes).unwrap_err(),
        Error::BufferTooShort(broadcast_common::error::BufferTooShort {
            need: 12,
            have: 9,
            what: "system_header fixed body",
        })
    );
}
