use std::time::{Duration, Instant};

use broadcast_common::{Stage, Timestamp};
use media_plane::ingress::{Dialer, IngestSession, SessionEvent};
use multimux::MultimuxError;
use multimux::source::rtsp::{RtspDialer, RtspIngestSession};
use rtsp_runtime::{InterleavedFrame, TransportSpec};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const SPROP: &str = "Z0IAKeKQFAe2AtwEBAaQeJEV,aM48gA==";
const RTP_CHANNEL: u8 = 0;
const TEST_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_ROUNDS: usize = 64;

const DEST_PLACEHOLDER: &str = "<loopback>";

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

fn sdp_body() -> Vec<u8> {
    format!(
        "v=0\r\n\
         o=- 0 0 IN IP4 127.0.0.1\r\n\
         s=-\r\n\
         t=0 0\r\n\
         m=video 0 RTP/AVP 96\r\n\
         a=rtpmap:96 H264/90000\r\n\
         a=fmtp:96 packetization-mode=1;sprop-parameter-sets={SPROP}\r\n\
         a=control:streamid=0\r\n"
    )
    .into_bytes()
}

fn rtp_packet(seq: u16, timestamp: u32, marker: bool, nal: &[u8]) -> Vec<u8> {
    const PT_H264_DYNAMIC: u8 = 96;
    const SSRC: u32 = 0xCAFE_BABE;
    let mut pkt = Vec::with_capacity(12 + nal.len());
    pkt.push(0x80);
    pkt.push(if marker {
        0x80 | PT_H264_DYNAMIC
    } else {
        PT_H264_DYNAMIC
    });
    pkt.extend_from_slice(&seq.to_be_bytes());
    pkt.extend_from_slice(&timestamp.to_be_bytes());
    pkt.extend_from_slice(&SSRC.to_be_bytes());
    pkt.extend_from_slice(nal);
    pkt
}

async fn read_request(sock: &mut TcpStream) -> (String, u32) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let text = String::from_utf8_lossy(&buf[..pos]).to_string();
            let cseq = text
                .lines()
                .find_map(|l| l.strip_prefix("CSeq:"))
                .map(|v| v.trim().parse().expect("valid CSeq"))
                .expect("request carries a CSeq header");
            return (text, cseq);
        }
        let n = sock.read(&mut chunk).await.expect("read request bytes");
        assert!(n > 0, "peer closed before a full request arrived");
        buf.extend_from_slice(&chunk[..n]);
    }
}

async fn write_all(sock: &mut TcpStream, bytes: &[u8]) {
    sock.write_all(bytes).await.expect("write response");
    sock.flush().await.expect("flush response");
}

async fn serve_one_session(sock: TcpStream) -> Vec<String> {
    let mut sock = sock;
    let mut requests = Vec::new();

    let (req, cseq) = read_request(&mut sock).await;
    requests.push(req);
    let sdp = sdp_body();
    let describe_resp = format!(
        "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n",
        sdp.len()
    );
    write_all(&mut sock, describe_resp.as_bytes()).await;
    write_all(&mut sock, &sdp).await;

    let (req, cseq) = read_request(&mut sock).await;
    requests.push(req);
    let transport = TransportSpec::rtp_avp_tcp_interleaved(RTP_CHANNEL, RTP_CHANNEL + 1)
        .to_header_value()
        .unwrap();
    let setup_resp = format!(
        "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: 00000001\r\nTransport: {transport}\r\n\r\n"
    );
    write_all(&mut sock, setup_resp.as_bytes()).await;

    let (req, cseq) = read_request(&mut sock).await;
    requests.push(req);
    let play_resp = format!("RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: 00000001\r\n\r\n");
    write_all(&mut sock, play_resp.as_bytes()).await;

    let idr = [0x65u8, 0xAA, 0xBB];
    let non1 = [0x41u8, 0xAA, 0xBB];
    let non2 = [0x41u8, 0xCC, 0xDD];
    let aus: [(u32, &[u8]); 3] = [(1000, &idr), (4000, &non1), (7000, &non2)];
    for (i, (ts, nal)) in aus.into_iter().enumerate() {
        let pkt = rtp_packet(1 + i as u16, ts, true, nal);
        let frame = InterleavedFrame::new(RTP_CHANNEL, pkt)
            .to_bytes()
            .expect("frame fits the u16 length field");
        write_all(&mut sock, &frame).await;
    }

    requests
}

async fn drive_and_collect(
    session: &mut RtspIngestSession,
    stream: &mut TcpStream,
    want: impl Fn(&[SessionEvent]) -> bool,
) -> Result<Vec<SessionEvent>, MultimuxError> {
    let start = Instant::now();
    let mut buf = [0u8; 64 * 1024];
    let mut events = Vec::new();
    for _ in 0..MAX_ROUNDS {
        while let Some(bytes) = session.poll_transmit() {
            stream.write_all(&bytes).await.expect("write request");
        }
        while let Some(ev) = session.poll() {
            events.push(ev);
        }
        if want(&events) {
            return Ok(events);
        }
        let n = stream.read(&mut buf).await.expect("read response");
        assert!(n > 0, "peer closed before the expected events arrived");
        let now = Timestamp::from_instant(start, Instant::now());
        session.feed(&buf[..n], now)?;
        while let Some(ev) = session.poll() {
            events.push(ev);
        }
        if want(&events) {
            return Ok(events);
        }
    }
    panic!("drive_and_collect: exceeded {MAX_ROUNDS} rounds");
}

#[tokio::test]
async fn rtsp_pull_golden() {
    tokio::time::timeout(TEST_TIMEOUT, async {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.expect("accept");
            serve_one_session(sock).await
        });

        let mut dialer = RtspDialer::new(format!("rtsp://{addr}/test"), None);
        let mut session = dialer.dial().expect("dial");
        let mut stream = TcpStream::connect(addr).await.expect("connect");

        let events = drive_and_collect(&mut session, &mut stream, |evs| {
            evs.iter()
                .filter(|e| matches!(e, SessionEvent::Sample { .. }))
                .count()
                >= 2
        })
        .await
        .expect("rtsp pull must succeed");

        let requests = tokio::time::timeout(TEST_TIMEOUT, server)
            .await
            .expect("server task")
            .expect("server task");

        let samples: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                SessionEvent::Sample {
                    track_id, sample, ..
                } => Some((*track_id, sample)),
                _ => None,
            })
            .collect();

        let mut rendered = String::new();
        for req in &requests {
            rendered.push_str(req);
            rendered.push_str("\n===\n");
        }
        rendered.push_str("SAMPLES\n");
        for (track_id, sample) in &samples {
            let digest = sample.data.iter().fold(FNV_OFFSET, |h, b| {
                (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME)
            });
            let dts = sample
                .dts
                .map(|v| v.to_string())
                .unwrap_or_else(|| "none".to_string());
            let pts = sample
                .pts
                .map(|v| v.to_string())
                .unwrap_or_else(|| "none".to_string());
            rendered.push_str(&format!(
                "track={} dts={} pts={} sync={} len={} fnv={:016x}\n",
                track_id,
                dts,
                pts,
                sample.flags.is_sync,
                sample.data.len(),
                digest
            ));
        }

        let rendered = rendered.replace(&addr.to_string(), DEST_PLACEHOLDER);
        let file = "golden/rtsp/pull-multimux.txt";
        let path = test_fixtures::path(file);
        if std::env::var("REGENERATE_RTSP_PULL_GOLDEN").ok().as_deref() == Some("1") {
            if std::env::var("CI").is_ok() {
                panic!("refuse to regenerate golden in CI");
            }
            std::fs::write(&path, &rendered).expect("write golden");
            return;
        }
        let expected = std::fs::read_to_string(&path).expect("read golden");
        assert_eq!(rendered, expected, "{file} differs");
    })
    .await
    .expect("rtsp pull golden timed out");
}
