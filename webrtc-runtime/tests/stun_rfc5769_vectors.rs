//! RFC 5769 "Test Vectors for Session Traversal Utilities for NAT (STUN)"
//! — official IETF test vectors, transcribed at
//! `fixtures/webrtc/rfc5769-stun-vectors.md` (see
//! `fixtures/webrtc/PROVENANCE.md` for source/licence).
//!
//! Parses each sample message with `rtc_stun::message::Message` — the same
//! type `webrtc_runtime::media`'s `src/media/gather.rs` uses for its own
//! STUN Binding transaction — and checks MESSAGE-INTEGRITY, FINGERPRINT,
//! and XOR-MAPPED-ADDRESS against the RFC's stated parameters. This is
//! fixture-only test code: no production code in this crate is touched.
//!
//! Gated on the `media` feature (needs rustc >= 1.88 — see the crate
//! README's MSRV section) both via `#![cfg(...)]` here and
//! `required-features` in `Cargo.toml`, per the workspace's documented
//! feature-gated-test-file trap.
#![cfg(feature = "media")]

use rtc_stun::fingerprint::FingerprintAttr;
use rtc_stun::integrity::MessageIntegrity;
use rtc_stun::message::{Getter, Message};
use rtc_stun::xoraddr::XorMappedAddress;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// RFC 5769 §2.1 / Appendix A `req[]` — Sample Request.
#[rustfmt::skip]
const REQ: &[u8] = &[
    0x00, 0x01, 0x00, 0x58, 0x21, 0x12, 0xa4, 0x42,
    0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae,
    0x80, 0x22, 0x00, 0x10, 0x53, 0x54, 0x55, 0x4e, 0x20, 0x74, 0x65, 0x73,
    0x74, 0x20, 0x63, 0x6c, 0x69, 0x65, 0x6e, 0x74,
    0x00, 0x24, 0x00, 0x04, 0x6e, 0x00, 0x01, 0xff,
    0x80, 0x29, 0x00, 0x08, 0x93, 0x2f, 0xf9, 0xb1, 0x51, 0x26, 0x3b, 0x36,
    0x00, 0x06, 0x00, 0x09, 0x65, 0x76, 0x74, 0x6a, 0x3a, 0x68, 0x36, 0x76,
    0x59, 0x20, 0x20, 0x20,
    0x00, 0x08, 0x00, 0x14, 0x9a, 0xea, 0xa7, 0x0c, 0xbf, 0xd8, 0xcb, 0x56,
    0x78, 0x1e, 0xf2, 0xb5, 0xb2, 0xd3, 0xf2, 0x49, 0xc1, 0xb5, 0x71, 0xa2,
    0x80, 0x28, 0x00, 0x04, 0xe5, 0x7a, 0x3b, 0xcf,
];

/// RFC 5769 §2.2 / Appendix A `respv4[]` — Sample IPv4 Response.
#[rustfmt::skip]
const RESP_V4: &[u8] = &[
    0x01, 0x01, 0x00, 0x3c, 0x21, 0x12, 0xa4, 0x42,
    0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae,
    0x80, 0x22, 0x00, 0x0b, 0x74, 0x65, 0x73, 0x74, 0x20, 0x76, 0x65, 0x63,
    0x74, 0x6f, 0x72, 0x20,
    0x00, 0x20, 0x00, 0x08, 0x00, 0x01, 0xa1, 0x47, 0xe1, 0x12, 0xa6, 0x43,
    0x00, 0x08, 0x00, 0x14, 0x2b, 0x91, 0xf5, 0x99, 0xfd, 0x9e, 0x90, 0xc3,
    0x8c, 0x74, 0x89, 0xf9, 0x2a, 0xf9, 0xba, 0x53, 0xf0, 0x6b, 0xe7, 0xd7,
    0x80, 0x28, 0x00, 0x04, 0xc0, 0x7d, 0x4c, 0x96,
];

/// RFC 5769 §2.3 / Appendix A `respv6[]` — Sample IPv6 Response.
#[rustfmt::skip]
const RESP_V6: &[u8] = &[
    0x01, 0x01, 0x00, 0x48, 0x21, 0x12, 0xa4, 0x42,
    0xb7, 0xe7, 0xa7, 0x01, 0xbc, 0x34, 0xd6, 0x86, 0xfa, 0x87, 0xdf, 0xae,
    0x80, 0x22, 0x00, 0x0b, 0x74, 0x65, 0x73, 0x74, 0x20, 0x76, 0x65, 0x63,
    0x74, 0x6f, 0x72, 0x20,
    0x00, 0x20, 0x00, 0x14,
    0x00, 0x02, 0xa1, 0x47,
    0x01, 0x13, 0xa9, 0xfa, 0xa5, 0xd3, 0xf1, 0x79,
    0xbc, 0x25, 0xf4, 0xb5, 0xbe, 0xd2, 0xb9, 0xd9,
    0x00, 0x08, 0x00, 0x14, 0xa3, 0x82, 0x95, 0x4e, 0x4b, 0xe6, 0x7b, 0xf1,
    0x17, 0x84, 0xc9, 0x7c, 0x82, 0x92, 0xc2, 0x75, 0xbf, 0xe3, 0xed, 0x41,
    0x80, 0x28, 0x00, 0x04, 0xc8, 0xfb, 0x0b, 0x4c,
];

/// RFC 5769 §2.4 / Appendix A `reqltc[]` — Sample Request with Long-Term
/// Authentication.
#[rustfmt::skip]
const REQ_LTC: &[u8] = &[
    0x00, 0x01, 0x00, 0x60, 0x21, 0x12, 0xa4, 0x42,
    0x78, 0xad, 0x34, 0x33, 0xc6, 0xad, 0x72, 0xc0, 0x29, 0xda, 0x41, 0x2e,
    0x00, 0x06, 0x00, 0x12,
    0xe3, 0x83, 0x9e, 0xe3, 0x83, 0x88, 0xe3, 0x83, 0xaa, 0xe3, 0x83, 0x83,
    0xe3, 0x82, 0xaf, 0xe3, 0x82, 0xb9, 0x00, 0x00,
    0x00, 0x15, 0x00, 0x1c,
    0x66, 0x2f, 0x2f, 0x34, 0x39, 0x39, 0x6b, 0x39, 0x35, 0x34, 0x64, 0x36,
    0x4f, 0x4c, 0x33, 0x34, 0x6f, 0x4c, 0x39, 0x46, 0x53, 0x54, 0x76, 0x79,
    0x36, 0x34, 0x73, 0x41,
    0x00, 0x14, 0x00, 0x0b, 0x65, 0x78, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x2e,
    0x6f, 0x72, 0x67, 0x00,
    0x00, 0x08, 0x00, 0x14, 0xf6, 0x70, 0x24, 0x65, 0x6d, 0xd6, 0x4a, 0x3e,
    0x02, 0xb8, 0xe0, 0x71, 0x2e, 0x85, 0xc9, 0xa2, 0x8c, 0xa8, 0x96, 0x66,
];

/// Password from RFC 5769 §2.1/§2.2/§2.3 (shared across the three
/// short-term-credential messages).
const SHORT_TERM_PASSWORD: &str = "VOkJxbRl1RmTxUk/WvJxBt";

fn parse(bytes: &[u8]) -> Message {
    let mut msg = Message::new();
    msg.unmarshal_binary(bytes).expect("parse STUN message");
    msg
}

#[test]
fn request_message_integrity_and_fingerprint() {
    let mut msg = parse(REQ);
    MessageIntegrity::new_short_term_integrity(SHORT_TERM_PASSWORD.to_string())
        .check(&mut msg)
        .expect("MESSAGE-INTEGRITY must verify against the RFC 5769 password");
    FingerprintAttr
        .check(&msg)
        .expect("FINGERPRINT must verify");
}

#[test]
fn ipv4_response_xor_mapped_address_and_integrity() {
    let mut msg = parse(RESP_V4);
    MessageIntegrity::new_short_term_integrity(SHORT_TERM_PASSWORD.to_string())
        .check(&mut msg)
        .expect("MESSAGE-INTEGRITY must verify");
    FingerprintAttr
        .check(&msg)
        .expect("FINGERPRINT must verify");

    let mut xor_addr = XorMappedAddress::default();
    xor_addr.get_from(&msg).expect("read XOR-MAPPED-ADDRESS");
    assert_eq!(xor_addr.ip, IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)));
    assert_eq!(xor_addr.port, 32853);
}

#[test]
fn ipv6_response_xor_mapped_address_and_integrity() {
    let mut msg = parse(RESP_V6);
    MessageIntegrity::new_short_term_integrity(SHORT_TERM_PASSWORD.to_string())
        .check(&mut msg)
        .expect("MESSAGE-INTEGRITY must verify");
    FingerprintAttr
        .check(&msg)
        .expect("FINGERPRINT must verify");

    let mut xor_addr = XorMappedAddress::default();
    xor_addr.get_from(&msg).expect("read XOR-MAPPED-ADDRESS");
    assert_eq!(
        xor_addr.ip,
        IpAddr::V6(Ipv6Addr::new(
            0x2001, 0x0db8, 0x1234, 0x5678, 0x0011, 0x2233, 0x4455, 0x6677
        ))
    );
    assert_eq!(xor_addr.port, 32853);
}

/// Long-term credential MESSAGE-INTEGRITY (RFC 5389 §15.4): key is
/// `MD5(username ':' realm ':' password)` using the **post-SASLprep**
/// password ("TheMatrIX"), with the katakana username passed through
/// unaffected by SASLprep, per RFC 5769 §2.4's own stated parameters.
#[test]
fn long_term_auth_message_integrity() {
    let mut msg = parse(REQ_LTC);
    let username = "\u{30de}\u{30c8}\u{30ea}\u{30c3}\u{30af}\u{30b9}".to_string();
    MessageIntegrity::new_long_term_integrity(
        username,
        "example.org".to_string(),
        "TheMatrIX".to_string(),
    )
    .check(&mut msg)
    .expect("MESSAGE-INTEGRITY must verify with the SASLprep'd long-term key");
}

// ---------------------------------------------------------------------------
// Bite tests: corrupt one byte, prove the checks fail, restore, prove they
// pass again (per the task's bite-proof requirement).
// ---------------------------------------------------------------------------

#[test]
fn corrupted_fingerprint_byte_fails_ipv4_response() {
    let mut bytes = RESP_V4.to_vec();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01; // flip one bit of the FINGERPRINT CRC-32
    let msg = parse(&bytes);
    assert!(
        FingerprintAttr.check(&msg).is_err(),
        "corrupted FINGERPRINT byte must fail verification"
    );

    // Restore and prove it passes again.
    bytes[last] ^= 0x01;
    let msg = parse(&bytes);
    FingerprintAttr
        .check(&msg)
        .expect("restored FINGERPRINT must verify");
}

#[test]
fn corrupted_message_integrity_byte_fails_request() {
    // Locate the MESSAGE-INTEGRITY attribute's value (right after its 4-byte
    // `00 08 00 14` type+length header) by scanning for that header, rather
    // than hardcoding a byte offset that would silently go stale if the
    // fixture changed.
    let needle = [0x00u8, 0x08, 0x00, 0x14];
    let pos = REQ
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("REQ must contain a MESSAGE-INTEGRITY attribute header");
    let tag_start = pos + needle.len();

    let mut bytes = REQ.to_vec();
    bytes[tag_start] ^= 0x01;
    let mut msg = parse(&bytes);
    assert!(
        MessageIntegrity::new_short_term_integrity(SHORT_TERM_PASSWORD.to_string())
            .check(&mut msg)
            .is_err(),
        "corrupted MESSAGE-INTEGRITY byte must fail verification"
    );

    bytes[tag_start] ^= 0x01;
    let mut msg = parse(&bytes);
    MessageIntegrity::new_short_term_integrity(SHORT_TERM_PASSWORD.to_string())
        .check(&mut msg)
        .expect("restored MESSAGE-INTEGRITY must verify");
}
