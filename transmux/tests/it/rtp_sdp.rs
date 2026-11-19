//! SDP media-section interpretation gates (RFC 4566 §5.14/§6, RFC 6184, RFC 3640).
//!
//! `sdp_track`, `first_encoding_name` and `is_played_encoding` are the public
//! predicates an RTSP/RTP pull uses to decide which `m=` sections are playable
//! and which payload format is played. Each row pins what the section's own
//! `a=rtpmap`/`a=fmtp` say rather than a value recomputed from the code.
#![cfg(feature = "std")]

use transmux::rtp_sdp::{
    first_encoding_name, fmtp_param, is_played_encoding, rtpmap_clock_rate, sdp_track,
};

const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
const AAC_FMTP: &str =
    "mode=AAC-hbr; config=1208; sizeLength=13; indexLength=3; indexDeltaLength=3";

fn media(body: &str) -> sdp_types::Media {
    let text = format!("v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n{body}");
    sdp_types::Session::parse(text.as_bytes())
        .expect("valid SDP")
        .medias
        .remove(0)
}

#[test]
fn a_video_section_with_a_valid_sprop_is_playable() {
    let m = media(&format!(
        "m=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\n\
         a=fmtp:96 packetization-mode=1; sprop-parameter-sets={SPROP}\r\n"
    ));
    assert!(sdp_track(&m).is_some());
}

#[test]
fn a_video_section_whose_sprop_is_garbage_is_not_playable() {
    let m = media(
        "m=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\n\
         a=fmtp:96 sprop-parameter-sets=not-base64-of-a-parameter-set\r\n",
    );
    assert_eq!(
        sdp_track(&m),
        None,
        "a present but unparsable sprop-parameter-sets must not degrade to a placeholder avcC"
    );
}

#[test]
fn a_video_section_with_an_unparsable_rtpmap_clock_is_not_playable() {
    let m = media(&format!(
        "m=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000x\r\n\
         a=fmtp:96 sprop-parameter-sets={SPROP}\r\n"
    ));
    assert_eq!(
        sdp_track(&m),
        None,
        "an rtpmap present but unparsable must not fall back to 90000"
    );
}

#[test]
fn a_video_section_without_an_rtpmap_defaults_to_a_90khz_clock() {
    let m = media(&format!(
        "m=video 0 RTP/AVP 96\r\na=fmtp:96 sprop-parameter-sets={SPROP}\r\n"
    ));
    let track = sdp_track(&m).expect("static video payload type");
    assert_eq!(track.clock_rate.get(), 90_000);
}

#[test]
fn an_audio_section_of_another_codec_is_not_playable_even_with_aac_fmtp() {
    for body in [
        // OPUS, with an fmtp that happens to carry every AAC-hbr key.
        format!(
            "m=audio 0 RTP/AVP 97\r\na=rtpmap:97 opus/48000/2\r\n\
             a=fmtp:97 {AAC_FMTP}\r\n"
        ),
        // PCMU with the AAC-hbr parameter list.
        format!(
            "m=audio 0 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\na=fmtp:0 {AAC_FMTP}\r\n"
        ),
    ] {
        assert_eq!(sdp_track(&media(&body)), None, "{body}");
    }
}

#[test]
fn an_aac_section_is_playable() {
    let m = media(&format!(
        "m=audio 0 RTP/AVP 97\r\na=rtpmap:97 mpeg4-generic/48000/2\r\na=fmtp:97 {AAC_FMTP}\r\n"
    ));
    assert!(sdp_track(&m).is_some());
}

#[test]
fn is_played_encoding_recognises_only_the_played_payloads() {
    assert!(is_played_encoding("video", "h264"));
    assert!(is_played_encoding("video", "H264"));
    assert!(!is_played_encoding("video", "AAC"));
    assert!(!is_played_encoding("video", "H265"));
    assert!(is_played_encoding("audio", "mpeg4-generic"));
    assert!(is_played_encoding("audio", "MPEG4-GENERIC"));
    assert!(!is_played_encoding("audio", "H264"));
    assert!(!is_played_encoding("text", "H264"));
    assert!(!is_played_encoding("text", "MPEG4-GENERIC"));
}

#[test]
fn first_encoding_name_reads_the_first_payload_types_rtpmap() {
    let m = media(
        "m=video 0 RTP/AVP 96 97\r\na=rtpmap:96 H264/90000\r\na=rtpmap:97 H265/90000\r\n",
    );
    assert_eq!(first_encoding_name(&m), Some("H264"));
    let no_rtpmap = media("m=video 0 RTP/AVP 96\r\n");
    assert_eq!(first_encoding_name(&no_rtpmap), None);
}

#[test]
fn a_numeric_leading_payload_token_is_stripped_from_a_fmtp_value() {
    assert_eq!(
        fmtp_param("96 profile-level-id=1", "profile-level-id"),
        Some("1")
    );
    assert_eq!(
        fmtp_param("96 packetization-mode=1;sprop-parameter-sets=aa", "packetization-mode"),
        Some("1")
    );
}

#[test]
fn a_non_numeric_first_word_is_not_stripped_from_a_fmtp_value() {
    assert_eq!(fmtp_param("abc profile=1", "profile"), None);
    assert_eq!(fmtp_param("abc profile=1", "abc profile"), Some("1"));
    assert_eq!(fmtp_param("packetization-mode=1", "packetization-mode"), Some("1"));
}

#[test]
fn rtpmap_clock_rate_strips_a_numeric_token_and_reads_the_rate() {
    assert_eq!(rtpmap_clock_rate("96 H264/90000"), Some(90_000));
    assert_eq!(rtpmap_clock_rate("H264/90000"), Some(90_000));
    assert_eq!(rtpmap_clock_rate("opus/48000/2"), Some(48_000));
    assert_eq!(rtpmap_clock_rate("96 H264/0"), None);
}
