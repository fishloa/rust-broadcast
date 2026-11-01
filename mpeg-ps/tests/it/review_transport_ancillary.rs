//! Adversarial review (transport/ancillary batch): FAILING regression tests for
//! confirmed `mpeg-ps` serializer defects. Each test states the expected
//! (correct) behaviour; they fail on the current code.
//!
//! 1. Silent width masks in `SystemHeader` / `PackHeader` serialize (the
//!    #1129/#1197 class the CHANGELOG says was fixed for `reserved_bits` only):
//!    an over-range hand-set value is written as a *different* value instead of
//!    being rejected.
//! 2. `SystemHeader::trailing_reserved` (new, #1119) and
//!    `StdBufferBound::stream_id` are not validated against the parser's loop
//!    condition (`nextbits() == '1'`, ISO/IEC 13818-1 Table 2-40), so a value
//!    serializes to bytes that parse back as a different header.

use broadcast_common::{Parse, Serialize};
use mpeg_ps::{PackHeader, Scr, StdBufferBound, SystemHeader};

fn header() -> SystemHeader {
    SystemHeader {
        rate_bound: 0x1234,
        audio_bound: 1,
        fixed_flag: false,
        csps_flag: false,
        system_audio_lock_flag: true,
        system_video_lock_flag: true,
        video_bound: 1,
        packet_rate_restriction_flag: false,
        reserved_bits: 0x7F,
        std_buffer_bounds: vec![StdBufferBound {
            stream_id: 0xE0,
            stream_id_extension: None,
            buffer_bound_scale: true,
            buffer_size_bound: 232,
        }],
        trailing_reserved: Vec::new(),
    }
}

fn ser(h: &SystemHeader) -> Result<Vec<u8>, mpeg_ps::Error> {
    let mut out = vec![0u8; h.serialized_len()];
    h.serialize_into(&mut out).map(|_| out)
}

/// `rate_bound` is 22 bits; `0x40_0000` was masked to `0` on the wire.
#[test]
fn system_header_rate_bound_over_22_bits_is_rejected() {
    let mut h = header();
    h.rate_bound = 0x40_0000;
    let r = ser(&h);
    assert!(
        r.is_err(),
        "rate_bound 0x40_0000 serialized as rate_bound {} instead of erroring",
        SystemHeader::parse(&r.unwrap()).unwrap().rate_bound
    );
}

/// `audio_bound` (6 bits) / `video_bound` (5 bits) are masked silently.
#[test]
fn system_header_audio_and_video_bound_over_width_are_rejected() {
    let mut h = header();
    h.audio_bound = 0x40;
    assert!(ser(&h).is_err(), "audio_bound 0x40 silently masked");
    let mut h = header();
    h.video_bound = 0x20;
    assert!(ser(&h).is_err(), "video_bound 0x20 silently masked");
}

/// `P-STD_buffer_size_bound` is 13 bits; `0x2000` was masked to `0`.
#[test]
fn system_header_buffer_size_bound_over_13_bits_is_rejected() {
    let mut h = header();
    h.std_buffer_bounds[0].buffer_size_bound = 0x2000;
    assert!(ser(&h).is_err(), "buffer_size_bound 0x2000 silently masked");
}

/// `PackHeader` SCR base is 33 bits: `1 << 33` was masked to base 0.
#[test]
fn pack_header_scr_base_over_33_bits_is_rejected() {
    let p = PackHeader {
        scr: Scr {
            base: 1 << 33,
            extension: 0,
        },
        program_mux_rate: 1,
        stuffing_length: 0,
        stuffing: &[],
        reserved: 0x1F,
    };
    let mut out = vec![0u8; p.serialized_len()];
    let r = p.serialize_into(&mut out);
    assert!(
        r.is_err(),
        "SCR base 2^33 serialized as base {}",
        PackHeader::parse(&out).unwrap().scr.base
    );
}

/// A `trailing_reserved` that starts with a byte whose MSB is set is
/// re-parsed as a stream-bound entry: serialize -> parse is not the identity.
#[test]
fn trailing_reserved_with_msb_set_value_round_trips_or_is_rejected() {
    let mut h = header();
    h.trailing_reserved = vec![0xC0, 0xC0, 0x00];
    match ser(&h) {
        Err(_) => {} // rejecting it is acceptable
        Ok(bytes) => {
            let back = SystemHeader::parse(&bytes).expect("own output parses");
            assert_eq!(back, h, "trailing_reserved re-parsed as a stream entry");
        }
    }
}

/// A `StdBufferBound` with `stream_id < 0x80` ends the parser's loop, so the
/// entry comes back as `trailing_reserved`: serialize -> parse is not the
/// identity and serialize does not reject it.
#[test]
fn std_buffer_bound_stream_id_without_msb_round_trips_or_is_rejected() {
    let mut h = header();
    h.std_buffer_bounds[0].stream_id = 0x00;
    match ser(&h) {
        Err(_) => {}
        Ok(bytes) => {
            let back = SystemHeader::parse(&bytes).expect("own output parses");
            assert_eq!(back, h, "stream_id 0x00 entry did not round-trip");
        }
    }
}
