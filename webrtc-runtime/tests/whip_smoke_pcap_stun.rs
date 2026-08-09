//! Parses the real STUN Binding request/response messages out of the
//! generated browser-interop capture at
//! `fixtures/webrtc/whip-ice-dtls-srtp-loopback.pcap` (see
//! `fixtures/webrtc/PROVENANCE.md` for how it was generated and verified).
//!
//! Unlike `stun_rfc5769_vectors.rs` (hand-transcribed IETF spec bytes), this
//! test walks a genuine libpcap capture of a real Chrome `RTCPeerConnection`
//! negotiating against this crate's own `media::MediaTransport`
//! (`examples/whip_media_smoke.rs`) and decodes the STUN messages Chrome
//! actually put on the wire with `rtc_stun::message::Message` — the same
//! type `media::gather` uses in production.
//!
//! Gated on the `media` feature both via `#![cfg(...)]` and
//! `required-features` in `Cargo.toml`, per the workspace's documented
//! feature-gated-test-file trap.
#![cfg(feature = "media")]

use rtc_stun::attributes::{
    ATTR_ICE_CONTROLLED, ATTR_ICE_CONTROLLING, ATTR_MESSAGE_INTEGRITY, ATTR_USERNAME,
};
use rtc_stun::message::{BINDING_REQUEST, BINDING_SUCCESS, Message, is_stun_message};

/// libpcap (classic, not pcapng) global header magic for little-endian,
/// microsecond-resolution captures — the format `tcpdump -w` wrote here.
const PCAP_MAGIC_LE: u32 = 0xa1b2_c3d4;

/// `DLT_NULL` / "BSD loopback" link type: each packet record is prefixed
/// with a 4-byte address-family word (host byte order) instead of an
/// Ethernet header, because the capture was taken on `lo0`.
const DLT_NULL: u32 = 0;

/// The `sa_family_t` value for `AF_INET` on macOS/BSD, as written by the
/// capturing host into the `DLT_NULL` 4-byte prefix.
const AF_INET_BSD: u32 = 2;

const UDP_PROTOCOL: u8 = 17;

fn fixture_path() -> String {
    format!(
        "{}/../fixtures/webrtc/whip-ice-dtls-srtp-loopback.pcap",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// Minimal classic-pcap walker: yields the UDP payload of every `DLT_NULL`
/// / `AF_INET` / UDP packet in the file. Written by hand rather than adding
/// a `pcap` dependency for one test file — the workspace has no existing
/// pcap-parsing crate in its dependency graph (checked before writing this).
fn udp_payloads(data: &[u8]) -> Vec<&[u8]> {
    assert!(data.len() >= 24, "pcap file too short for global header");
    let magic = u32::from_le_bytes(data[0..4].try_into().unwrap());
    assert_eq!(
        magic, PCAP_MAGIC_LE,
        "not a little-endian classic pcap file"
    );
    let linktype = u32::from_le_bytes(data[20..24].try_into().unwrap());
    assert_eq!(linktype, DLT_NULL, "fixture must be a DLT_NULL/lo0 capture");

    let mut out = Vec::new();
    let mut off = 24usize;
    while off + 16 <= data.len() {
        let caplen = u32::from_le_bytes(data[off + 8..off + 12].try_into().unwrap()) as usize;
        let rec_start = off + 16;
        assert!(rec_start + caplen <= data.len(), "truncated packet record");
        let rec = &data[rec_start..rec_start + caplen];
        off = rec_start + caplen;

        if rec.len() < 4 {
            continue;
        }
        let family = u32::from_le_bytes(rec[0..4].try_into().unwrap());
        if family != AF_INET_BSD {
            continue; // IPv6/other loopback traffic (mDNS-less here) - not needed
        }
        let ip = &rec[4..];
        if ip.len() < 20 {
            continue;
        }
        let ihl = ((ip[0] & 0x0F) as usize) * 4;
        if ip.len() < ihl + 8 || ip[9] != UDP_PROTOCOL {
            continue;
        }
        let udp = &ip[ihl..];
        let udp_len = u16::from_be_bytes([udp[4], udp[5]]) as usize;
        if udp_len < 8 || udp.len() < udp_len {
            continue;
        }
        out.push(&udp[8..udp_len]);
    }
    out
}

#[test]
fn fixture_contains_real_stun_binding_transaction() {
    let data = std::fs::read(fixture_path()).expect("read whip-ice-dtls-srtp-loopback.pcap");
    let payloads = udp_payloads(&data);
    assert!(
        !payloads.is_empty(),
        "expected at least one UDP datagram in the fixture"
    );

    let stun_payloads: Vec<&&[u8]> = payloads.iter().filter(|p| is_stun_message(p)).collect();
    assert!(
        stun_payloads.len() >= 2,
        "expected at least a STUN request and a response, got {}",
        stun_payloads.len()
    );

    let mut saw_request = false;
    let mut saw_success_response = false;

    for payload in &stun_payloads {
        let mut msg = Message::new();
        msg.unmarshal_binary(payload)
            .expect("decode a genuine Chrome-emitted STUN message");

        // Every request/response pair in this trace is a Binding
        // transaction (ICE connectivity check) - no other STUN method is
        // used by ICE.
        if msg.typ == BINDING_REQUEST {
            saw_request = true;
            // Every ICE connectivity-check request in this trace carries
            // the short-term credential (USERNAME) and is authenticated
            // (MESSAGE-INTEGRITY, RFC 5389 §15.4). This is full ICE (not
            // ice-lite), so BOTH sides send Binding requests as part of
            // their own check list -- Chrome's requests carry
            // ICE-CONTROLLING (it is the controlling agent per the
            // offer/answer role assignment), while this crate's own
            // `MediaTransport` (configured `is_controlling: false` in the
            // example) sends its checks with ICE-CONTROLLED instead (RFC
            // 8445 §7.1.1) -- exactly one of the two must be present, never
            // both and never neither.
            assert!(
                msg.contains(ATTR_USERNAME),
                "STUN Binding request missing USERNAME"
            );
            assert!(
                msg.contains(ATTR_MESSAGE_INTEGRITY),
                "STUN Binding request missing MESSAGE-INTEGRITY"
            );
            let controlling = msg.contains(ATTR_ICE_CONTROLLING);
            let controlled = msg.contains(ATTR_ICE_CONTROLLED);
            assert!(
                controlling != controlled,
                "STUN Binding request must carry exactly one of \
                 ICE-CONTROLLING/ICE-CONTROLLED (controlling={controlling}, controlled={controlled})"
            );
        } else if msg.typ == BINDING_SUCCESS {
            saw_success_response = true;
            assert!(
                msg.contains(ATTR_MESSAGE_INTEGRITY),
                "STUN Binding success response missing MESSAGE-INTEGRITY"
            );
        }
    }

    assert!(saw_request, "no STUN Binding request found in fixture");
    assert!(
        saw_success_response,
        "no STUN Binding success response found in fixture"
    );
}

/// Sanity check on the DTLS/SRTP side of the same capture: after the STUN
/// Binding transaction, later UDP payloads on the same 5-tuple flip from
/// STUN (magic cookie header) to DTLS record layer (content type 20-23) and
/// then to what is opaque SRTP ciphertext (RTP-shaped header, non-STUN,
/// non-DTLS). This doesn't decode DTLS/SRTP -- that's `rtc-dtls`/`rtc-srtp`'s
/// job, proven live by the `whip_media_smoke` example this fixture was
/// captured from -- it just confirms the fixture's later packets are wire
/// bytes consistent with a real handshake-then-media progression, not junk.
#[test]
fn fixture_progresses_from_stun_to_dtls_to_srtp_like_payloads() {
    let data = std::fs::read(fixture_path()).expect("read whip-ice-dtls-srtp-loopback.pcap");
    let payloads = udp_payloads(&data);

    // DTLS record layer: ContentType (1 byte, 20-23) + ProtocolVersion (2
    // bytes, {0xfe,0xfd} for DTLS 1.2) is the first 3 bytes of every record.
    let looks_like_dtls = |p: &[u8]| -> bool {
        p.len() >= 13 && (20..=23).contains(&p[0]) && p[1] == 0xfe && p[2] == 0xfd
    };

    let mut saw_stun = false;
    let mut saw_dtls = false;
    let mut saw_post_dtls_non_stun_non_dtls = false; // the SRTP-shaped tail

    for payload in &payloads {
        if is_stun_message(payload) {
            saw_stun = true;
        } else if looks_like_dtls(payload) {
            saw_dtls = true;
        } else if saw_dtls && payload.len() >= 12 {
            // After the handshake has started, a non-STUN/non-DTLS UDP
            // payload on this flow is SRTP: an RTP-shaped header (version
            // bits 10 in the first byte) whose payload is opaque ciphertext.
            let version = (payload[0] >> 6) & 0x03;
            if version == 2 {
                saw_post_dtls_non_stun_non_dtls = true;
            }
        }
    }

    assert!(saw_stun, "expected STUN packets before the DTLS handshake");
    assert!(
        saw_dtls,
        "expected DTLS record-layer packets in the fixture"
    );
    assert!(
        saw_post_dtls_non_stun_non_dtls,
        "expected RTP-shaped (SRTP) packets after the DTLS handshake"
    );
}
