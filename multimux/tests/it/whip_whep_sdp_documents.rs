//! The two SDP documents a WHIP/WHEP endpoint renders as a SIGNAL: the answer
//! (`application/sdp`) and the ICE-restart answer fragment
//! (`application/trickle-ice-sdpfrag`). Recorded byte for byte from `main`
//! before issue #2363 moved the session assembly into `webrtc_runtime::sdp`,
//! so the move must reproduce them without regenerating.
//!
//! Every input is fixed (addresses, ICE credentials, fingerprint, codec
//! config, candidate lines) and nothing is random: `plan_restart` draws its
//! credentials from the `token` function it is handed, so the test passes a
//! fixed one. See `fixtures/golden/webrtc/PROVENANCE.md`.

use std::fs;

use multimux::output::whep::WhepAnswerParams;
use multimux::output::whep::render_whep_answer_for_test;
use multimux::source::whip::render_answer_for_test;
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

const LOCAL_ADDR: &str = "127.0.0.1:54321";
const LOCAL_UFRAG: &str = "localufrag";
const LOCAL_PWD: &str = "localpwdlocalpwdlocalpwd12";
const FINGERPRINT: &str = "00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff";

/// The restart fragment's offer: current generation `oldufrag`, and the peer's
/// new one.
const RESTART_FRAGMENT: &str = "a=ice-ufrag:newufrag\r\n\
a=ice-pwd:newpwdnewpwdnewpwdnewpwd12\r\n";

fn candidates() -> Vec<String> {
    vec!["0 1 udp 2130706431 127.0.0.1 54321 typ host".to_string()]
}

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

fn check(name: &str, actual: &[u8]) {
    if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
        let path = std::path::PathBuf::from(dir).join(name);
        fs::create_dir_all(path.parent().expect("golden directory")).expect("create golden dir");
        fs::write(&path, actual).expect("write golden");
        return;
    }
    let path = test_fixtures::path(name);
    let expected = fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    assert_eq!(actual, expected, "{name} differs from the golden output");
}

#[test]
fn whip_answer_matches_golden() {
    let answer = render_answer_for_test(
        OFFER,
        LOCAL_ADDR.parse().expect("local address"),
        LOCAL_UFRAG,
        LOCAL_PWD,
        FINGERPRINT,
        &candidates(),
    )
    .expect("render the answer");
    check("golden/webrtc/whip-answer.golden", answer.as_bytes());
}

#[test]
fn whep_answer_matches_golden() {
    let answer = render_whep_answer_for_test(
        OFFER,
        WhepAnswerParams::new(
            LOCAL_ADDR.parse().expect("local address"),
            LOCAL_UFRAG,
            LOCAL_PWD,
            SetupRole::Active,
            &config(),
            FINGERPRINT,
            &candidates(),
        ),
    )
    .expect("render the answer");
    check("golden/webrtc/whep-answer.golden", answer.as_bytes());
}

#[test]
fn whip_restart_fragment_matches_golden() {
    let answer = multimux::webrtc_ice::restart_fragment_for_test(
        RESTART_FRAGMENT.as_bytes(),
        "oldufrag",
        &candidates(),
        |len| "1".repeat(len),
    )
    .expect("plan a restart");
    check("golden/webrtc/whip-restart-fragment.sdpfrag", &answer);
}
