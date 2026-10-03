//! Goldens for the RTP SDP and the HLS IV text, generated from unmodified
//! `main` (see `tests/golden/README.md`). `GOLDEN_BLESS=<dir>` writes.
#![cfg(feature = "std")]

use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

use broadcast_common::{Package, Unpackage};
use transmux::{RtpPacketiser, TsDemux, build_sdp_with_connection};

fn check(name: &str, actual: &str) {
    if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
        fs::create_dir_all(&dir).expect("create golden dir");
        fs::write(Path::new(&dir).join(name), actual).expect("write golden");
        return;
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    let expected = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    assert_eq!(actual, expected, "{name} differs from the golden output");
}

#[test]
fn rtp_session_sdp_matches_golden() {
    let ts = fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264_aac.ts"))
        .expect("ts fixture");
    let media = TsDemux::new().unpackage(&ts[..]).expect("demux");
    let mut p = RtpPacketiser {
        mtu: 1400,
        ssrc: 0x1234_5678,
        ..RtpPacketiser::default()
    };
    let out = p.package(&media).expect("packetise");
    check("rtp-sdp-h264-aac.sdp", &out.sdp);
}

/// A parseable session whose only role is to give the test an `sdp_types::Media`.
const MEDIA_BLOCK_SESSION: &str = concat!(
    "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=x\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n",
    "m=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\n",
    "a=fmtp:96 packetization-mode=1; profile-level-id=64000D; sprop-parameter-sets=Z2QADazZQUH7ARAAAAMAEAAAAwMg8UKZYA==,aOvjyyLA\r\n"
);

#[test]
fn connection_address_sdp_matches_golden() {
    let media: Vec<sdp_types::Media> = sdp_types::Session::parse(MEDIA_BLOCK_SESSION.as_bytes())
        .expect("parse media block")
        .medias;
    let v4 = build_sdp_with_connection(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7)), media);
    let v6 = build_sdp_with_connection(
        IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
        Vec::new(),
    );
    check("rtp-sdp-conn-v4.sdp", &v4);
    check("rtp-sdp-conn-v6.sdp", &v6);
}

#[cfg(feature = "sample-aes")]
#[test]
fn hls_iv_text_matches_golden() {
    let mut text = String::new();
    for iv in [
        [0u8; 16],
        [0xff; 16],
        [
            0x00, 0x01, 0x0a, 0x0b, 0xa0, 0xb0, 0xf0, 0x0f, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60,
            0x70, 0x80,
        ],
    ] {
        text.push_str(&transmux::sample_aes::format_iv(&iv));
        text.push('\n');
    }
    check("hls-iv.txt", &text);
}
