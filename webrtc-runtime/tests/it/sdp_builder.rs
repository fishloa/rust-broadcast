//! The SDP documents `webrtc_runtime::sdp` renders for a WHIP/WHEP
//! endpoint, pinned byte for byte against the goldens multimux recorded from
//! its own four session assemblies before issue #2363 moved them here.

use std::fs;
use std::net::{IpAddr, Ipv4Addr};

use sdp_types::{Media, MediaType, TransportProto};
use webrtc_runtime::sdp::{
    AnswerCodec, AnswerTransport, CodecAttribute, CodecAttributeName, Direction,
    LocalMediaIdentity, offer_codec_attributes, restart_answer, session_answer,
};

const FINGERPRINT: &str = "00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff";
const MID: &str = "0";
/// RFC 3264 §5.1's port for a media stream that is not (yet) at its final
/// address, the one a bundle-only placeholder section names.
const FRAGMENT_PORT: u16 = 9;
/// RFC 3264 §5.1's `m=` port and payload type a section that is held (not
/// rejected) names: port `9` mirrors the transport's presence, the payload
/// type `0` names no format the offer had.
const HOLD_PAYLOAD_TYPE: u8 = 0;

fn check(name: &str, actual: &str) {
    if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
        let path = std::path::PathBuf::from(dir).join(name);
        fs::create_dir_all(path.parent().expect("golden directory")).expect("create golden dir");
        fs::write(&path, actual).expect("write golden");
        return;
    }
    let path = test_fixtures::path(name);
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    assert_eq!(actual, expected, "{name} differs from the golden output");
}

fn local() -> LocalMediaIdentity<'static> {
    LocalMediaIdentity {
        ice_ufrag: "localufrag",
        ice_pwd: "localpwdlocalpwdlocalpwd12",
        fingerprint: FINGERPRINT,
    }
}

fn candidates() -> Vec<String> {
    vec!["0 1 udp 2130706431 127.0.0.1 54321 typ host".to_string()]
}

fn offer_media() -> Media {
    let mut media = Media::new(MediaType::Video, 9, TransportProto::UdpTlsRtpSavpf, "");
    media.add_attribute_with_value("ice-ufrag", "abcd");
    media.add_attribute_with_value("rtpmap", "96 H264/90000");
    media.add_attribute_with_value(
        "fmtp",
        "96 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f",
    );
    media.add_attribute_with_value("rtpmap", "97 H264/90000");
    media.add_attribute_with_value(
        "fmtp",
        "97 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f",
    );
    media.add_attribute_with_value("rtpmap", "98 VP8/90000");
    media
}

fn transport(media: Media, payload_type: u8) -> AnswerTransport {
    AnswerTransport {
        offer: media,
        payload_type,
    }
}

#[test]
fn whip_session_answer_matches_golden() {
    let media = offer_media();
    let codec = AnswerCodec {
        direction: Direction::Receive,
        transport: transport(media.clone(), 96),
        codec_attributes: offer_codec_attributes(&media, 96),
    };
    let answer = session_answer(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        local(),
        MID,
        "passive",
        &codec,
        &candidates(),
    )
    .expect("render the answer");
    check("golden/webrtc/whip-answer.golden", &answer);
}

#[test]
fn whep_session_answer_matches_golden() {
    let codec = AnswerCodec {
        direction: Direction::Send,
        transport: transport(
            Media::new(MediaType::Video, 9, TransportProto::UdpTlsRtpSavpf, ""),
            96,
        ),
        codec_attributes: vec![
            CodecAttribute::new(CodecAttributeName::Rtpmap, String::from("96 H264/90000")),
            CodecAttribute::new(
                CodecAttributeName::Fmtp,
                String::from(
                    "96 packetization-mode=1;profile-level-id=42001F;sprop-parameter-sets=",
                ),
            ),
        ],
    };
    let answer = session_answer(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        local(),
        MID,
        "active",
        &codec,
        &candidates(),
    )
    .expect("render the answer");
    check("golden/webrtc/whep-answer.golden", &answer);
}

#[test]
fn whip_restart_answer_matches_golden() {
    let codec = AnswerCodec {
        direction: Direction::Send,
        transport: transport(
            Media::new(
                MediaType::Video,
                FRAGMENT_PORT,
                TransportProto::UdpTlsRtpSavpf,
                "",
            ),
            HOLD_PAYLOAD_TYPE,
        ),
        codec_attributes: Vec::new(),
    };
    let answer = restart_answer(
        LocalMediaIdentity {
            ice_ufrag: "11111111",
            ice_pwd: "111111111111111111111111",
            fingerprint: FINGERPRINT,
        },
        &codec,
        &candidates(),
    )
    .expect("render the fragment");
    check("golden/webrtc/whip-restart-fragment.sdpfrag", &answer);
}

/// The echo list follows the offer: only attributes whose value starts with
/// the chosen payload type, and only the names asked for.
#[test]
fn the_echo_list_names_only_the_chosen_payload_type() {
    let media = offer_media();
    let echo = offer_codec_attributes(&media, 97);
    let values: Vec<_> = echo.iter().map(|a| a.value.as_str()).collect();
    assert_eq!(values.len(), 2, "{values:?}");
    assert!(values.iter().all(|v| v.starts_with("97 ")), "{values:?}");
    let none = offer_codec_attributes(&media, 99);
    assert!(none.is_empty(), "a payload type the offer does not name");
}

#[test]
fn wildcard_rtcp_fb_is_echoed() {
    let mut media = offer_media();
    media.add_attribute_with_value("rtcp-fb", "* nack");
    media.add_attribute_with_value("rtcp-fb", "96 pli");
    media.add_attribute_with_value("rtcp-fb", "97 nack");
    let echo = offer_codec_attributes(&media, 96);
    let values: Vec<_> = echo.iter().map(|a| a.value.as_str()).collect();
    assert!(values.contains(&"* nack"), "{values:?}");
    assert!(values.contains(&"96 pli"), "{values:?}");
    assert!(!values.contains(&"97 nack"), "{values:?}");
}
