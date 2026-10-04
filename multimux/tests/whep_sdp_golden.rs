#![cfg(feature = "whep")]
//! Byte-for-byte golden of the WHEP SDP answer as rendered by
//! `sdp_types::Session::write` (deterministic: the offer, local address, ICE
//! credentials, codec config, fingerprint and candidate lines are all caller
//! supplied). Pins the `o=`/`c=`/attribute line ordering, which differs from
//! main's hand-built text (see the CHANGELOG and `tests/golden/README.md`).

use webrtc_runtime::media::SetupRole;

const OFFER: &str = "v=0\r\n\
o=- 0 0 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 96 97 98\r\n\
c=IN IP4 0.0.0.0\r\n\
a=ice-ufrag:abcd\r\n\
a=ice-pwd:abcdefghijklmnopqrstuvwx\r\n\
a=fingerprint:sha-256 00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff\r\n\
a=setup:actpass\r\n\
a=mid:0\r\n\
a=rtcp-mux\r\n\
a=rtpmap:96 H264/90000\r\n\
a=fmtp:96 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n\
a=rtpmap:97 H264/90000\r\n\
a=fmtp:97 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n\
a=rtpmap:98 VP8/90000\r\n";

/// No live codec config: the golden just pins the structural order.
fn config() -> transmux::AVCDecoderConfigurationRecord {
    transmux::AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 0x42,
        profile_compatibility: 0,
        level_indication: 0x1f,
        length_size_minus_one: 3,
        sps: vec![],
        pps: vec![],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    }
}

#[test]
fn whep_answer_matches_golden() {
    let answer = multimux::output::whep::render_whep_answer_for_test(
        OFFER,
        "127.0.0.1:54321".parse().unwrap(),
        "localufrag",
        "localpwdlocalpwdlocalpwd12",
        SetupRole::Active,
        &config(),
        "00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
         00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff",
        &["0 1 udp 2130706431 127.0.0.1 54321 typ host".to_string()],
    );
    let file = "whep_answer.golden";
    if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
        std::fs::create_dir_all(&dir).expect("create golden dir");
        std::fs::write(std::path::Path::new(&dir).join(file), &answer).expect("write");
        return;
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(file);
    let expected = std::fs::read_to_string(&path).expect("read golden");
    assert_eq!(
        answer, expected,
        "WHEP answer must match the golden byte-for-byte"
    );
}
