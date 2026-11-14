use crate::support::udp_host::UdpHost;
use broadcast_client::{Config, Error, Event, Session, SourceKind};
use broadcast_common::Serialize;
use broadcast_common::stage::Timestamp;
use rtp_packet::RtpPacket;

/// The harness's microsecond clock as the session's `now`.
fn ts(us: u64) -> Timestamp {
    Timestamp::from_micros(us)
}

const URL: &str = "rtp://127.0.0.1:5004";
const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
const H264_PAYLOAD_TYPE: u8 = 96;
const SSRC: u32 = 0xCAFE_BABE;
const CLOCK_HZ: u32 = 90_000;

fn sdp() -> String {
    format!(
        "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\nm=video 0 RTP/AVP 96\r\n\
         a=rtpmap:96 H264/90000\r\na=fmtp:96 packetization-mode=1;sprop-parameter-sets={SPROP}\r\n"
    )
}

fn config(sdp: Option<String>) -> Config {
    let mut c = Config::relay();
    c.kind = SourceKind::Rtp;
    c.udp.sdp = sdp;
    c
}

fn packet(seq: u16, timestamp: u32, payload_type: u8, marker: bool, nal: &[u8]) -> Vec<u8> {
    let wire = RtpPacket {
        marker,
        payload_type,
        sequence_number: seq,
        timestamp,
        ssrc: SSRC,
        csrc: Vec::new(),
        extension: None,
        padding: None,
        payload: nal,
    };
    let mut bytes = vec![0u8; wire.serialized_len()];
    wire.serialize_into(&mut bytes).expect("serialize RTP");
    bytes
}

fn au(seq: u16, timestamp: u32, nal: &[u8]) -> Vec<u8> {
    packet(seq, timestamp, H264_PAYLOAD_TYPE, true, nal)
}

fn samples(host: &UdpHost) -> Vec<bool> {
    host.events
        .iter()
        .filter_map(|t| match &t.event {
            Event::Sample { sample, .. } => Some(sample.flags.is_sync),
            _ => None,
        })
        .collect()
}

#[test]
fn the_sdp_tracks_are_announced_when_the_socket_is_bound_before_any_packet() {
    let host = UdpHost::open(URL, config(Some(sdp())));
    let tracks: Vec<_> = host
        .events
        .iter()
        .filter_map(|t| match &t.event {
            Event::Tracks(specs) => Some(specs.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].len(), 1);
    assert_eq!(tracks[0][0].timescale, CLOCK_HZ);
    assert_eq!(host.session.payload_bytes(), 0);
}

#[test]
fn three_access_units_yield_two_completed_samples_idr_first() {
    let mut host = UdpHost::open(URL, config(Some(sdp())));
    host.datagram(&au(1, 1000, &[0x65, 0xAA, 0xBB]));
    host.datagram(&au(2, 4000, &[0x41, 0xAA, 0xBB]));
    host.datagram(&au(3, 7000, &[0x41, 0xCC, 0xDD]));
    assert_eq!(samples(&host), vec![true, false]);
    assert!(host.session.payload_bytes() > 0);
}

#[test]
fn a_payload_type_the_sdp_does_not_declare_is_ignored() {
    let mut host = UdpHost::open(URL, config(Some(sdp())));
    host.datagram(&packet(1, 1000, 111, true, &[0x65, 0xAA]));
    host.datagram(&packet(2, 4000, 111, true, &[0x41, 0xAA]));
    assert!(samples(&host).is_empty());
    assert_eq!(
        host.session.payload_bytes(),
        0,
        "an undeclared stream is not payload"
    );
}

#[test]
fn a_datagram_that_is_not_rtp_is_dropped_and_counted() {
    let mut host = UdpHost::open(URL, config(Some(sdp())));
    host.datagram(&[0x80, 0x60]);
    let stats = host.session.datagram_stats();
    assert_eq!((stats.datagrams, stats.malformed), (1, 1));
}

#[test]
fn a_reconnect_flushes_the_access_units_still_held() {
    let mut host = UdpHost::open(URL, config(Some(sdp())));
    host.datagram(&au(1, 1000, &[0x65, 0xAA]));
    host.datagram(&packet(2, 4000, H264_PAYLOAD_TYPE, false, &[0x41, 0xBB]));
    assert!(samples(&host).is_empty(), "nothing completed yet");
    host.close_socket();
    host.advance_to(host.now + 60_000_000);
    assert_eq!(
        samples(&host),
        vec![true, false],
        "both trailing units flushed"
    );
}

#[test]
fn an_sdp_needs_an_rtp_url_and_a_playable_media_section() {
    let open = |url: &str, sdp: &str| Session::open(url, config(Some(sdp.to_string())), ts(0));
    let mut udp = config(Some(sdp()));
    udp.kind = SourceKind::Udp;
    assert!(matches!(
        Session::open("udp://127.0.0.1:5004", udp, ts(0)),
        Err(Error::InvalidConfig { field: "udp.sdp" })
    ));
    assert!(matches!(
        open(URL, "v=0\r\ns=-\r\nt=0 0\r\n"),
        Err(Error::InvalidConfig { field: "udp.sdp" })
    ));
    assert!(matches!(
        open(URL, "not an sdp"),
        Err(Error::InvalidConfig { field: "udp.sdp" })
    ));
}
