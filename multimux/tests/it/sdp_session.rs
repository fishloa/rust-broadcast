//! Goldens for the SDP session description, recorded from `main` before issue
//! #2357 moved the session builder into `rtsp_runtime::sdp`.
//!
//! multimux has both builders in scope (`transmux`'s RTP packetiser and its
//! own RTSP push). It lives here rather than in `rtsp-runtime` so the SDP crate
//! never depends on the two that produce media sections: `multimux` depends on
//! `rtsp-runtime`, never the reverse. `GOLDEN_BLESS=<dir>` writes instead.

use std::fs;

use broadcast_common::{Package, Unpackage};

const FIXTURE: &str = "ts/h264-aac.ts";
const MTU: usize = 1400;
const SSRC: u32 = 0x1234_5678;
/// RFC 3551 §6 static payload type for MPEG2 Transport Stream (RFC 2250 §2).
const MP2T_PT: u8 = 33;
/// RFC 2250 §2's fixed RTP clock rate for MP2T.
const RTP_CLOCK_HZ: u32 = 90_000;

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

/// transmux's `RtpOutput::medias` for the committed H.264 + AAC fixture
/// (`broadcast-client`'s RTSP capture fixture) — the media sections that
/// become the session's `m=` blocks.
#[test]
fn transmux_rtp_medias_match_golden() {
    let ts = test_fixtures::read(FIXTURE);
    let media = transmux::TsDemux::new()
        .unpackage(&ts[..])
        .expect("demux fixture");
    let mut packetiser = transmux::RtpPacketiser {
        mtu: MTU,
        ssrc: SSRC,
        ..transmux::RtpPacketiser::default()
    };
    let out = packetiser.package(&media).expect("packetise fixture");
    assert_eq!(out.medias.len(), out.streams.len());
    let origin = sdp_types::Origin::with_ip_addr(0, 0, std::net::Ipv4Addr::LOCALHOST);
    let connection =
        sdp_types::Connection::from_ip_addr(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    let sdp = rtsp_runtime::session_description("transmux RTP", origin, connection, out.medias)
        .expect("render session");
    check("golden/rtsp/sdp-transmux-h264-aac.sdp", &sdp);
}

/// multimux's MPEG-2 TS push media sections, as the ANNOUNCE body
/// (`multimux/src/push/rtsp.rs`) describes them. The whole body is pinned by
/// `golden/rtsp/rtsp-announce.sdp` (multimux's own test).
#[test]
fn multimux_rtsp_push_medias_match_golden() {
    let sdp = multimux::push::rtsp::push_sdp_for_test().expect("build push SDP");
    let announce = fs::read_to_string(test_fixtures::path("golden/rtsp/rtsp-announce.sdp"))
        .expect("read announce golden");
    assert_eq!(sdp, announce, "push body differs from rtsp-announce.sdp");
    let media = multimux::push::rtsp::push_media_for_test();
    assert_eq!(media.len(), 1, "one m= section for the whole TS");
    assert_eq!(media[0].media, "video");
    assert_eq!(media[0].port, 0);
    assert_eq!(media[0].proto, "RTP/AVP");
    assert_eq!(media[0].fmt, MP2T_PT.to_string());
    check(
        "golden/rtsp/sdp-push-mp2t-media.sdp",
        &format!(
            "m={} {} {} {}\r\na=rtpmap:{MP2T_PT} MP2T/{RTP_CLOCK_HZ}\r\na=control:trackID=0\r\n",
            media[0].media, media[0].port, media[0].proto, media[0].fmt
        ),
    );
    let origin = sdp_types::Origin::new(
        "0",
        0,
        sdp_types::NetType::In,
        sdp_types::AddrType::Ip4,
        "0.0.0.0",
    );
    let connection =
        sdp_types::Connection::new(sdp_types::NetType::In, sdp_types::AddrType::Ip4, "0.0.0.0");
    let session = rtsp_runtime::session_description("multimux push", origin, connection, media)
        .expect("render session");
    check("golden/rtsp/sdp-push-mp2t-session.sdp", &session);
}
