//! RTSP push wire golden (issue #2368): the exact requests and interleaved
//! RTP packets `RtspTransport` puts on the wire, recorded once against
//! rtsp-runtime's own sans-IO `ServerSession` and never regenerated.
//!
//! The recording pins the whole push choreography — `OPTIONS`, `ANNOUNCE`
//! (RFC 2326 §10.3) with its SDP body, `SETUP` (§10.4) on the `trackID=0`
//! control URL with `mode=record`, `RECORD` (§10.11) and every interleaved
//! `$`-framed RTP packet (§10.12) — so moving the RTP header, the RFC 2250 §2
//! MP2T packing and the RECORD flow into `rtp-packet`, `transmux` and
//! `rtsp-runtime` cannot change a single byte a real server sees.
//!
//! # Why a relay, and why `ServerSession` rather than `AsyncRtspServer`
//!
//! `AsyncRtspServer::next_request` returns parsed events, not the request
//! bytes a recording must pin. So the push talks to a loopback `TcpListener`
//! that reads every complete RTSP request off the socket, keeps its raw bytes,
//! hands them to a real `ServerSession` (which answers them) and writes that
//! response back; interleaved `$` frames pass straight through to it and are
//! recorded as they are. Both the requests and the payloads in the golden are
//! therefore the bytes that crossed the socket, produced by the real client.
//!
//! The SSRC and the 90 kHz clock origin are injected
//! (`RtspTransportConfig::test_wire`), so the RTP timestamps are
//! deterministic: both batches are pushed inside the instant the connection
//! is made and share timestamp zero, and the second batch exercises the
//! sequence-number increment.
//!
//! `GOLDEN_BLESS=<dir>` writes the recording instead of comparing it.

use std::time::{Duration, Instant};

use crate::support::shared_it::fixture_path;
use broadcast_common::Package as _;
use multimux::push::rtsp::RtpWireState;
use multimux::push::{PushTransport, RtspTransport, RtspTransportConfig};
use rtsp_runtime::ServerSession;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// Seconds to wait after RECORD, so its response is read before the first
/// media packet and every frame lands under the right request.
const BEFORE_MEDIA: Duration = Duration::from_millis(100);

/// Boundary between two recorded requests.
const REQUEST_SEPARATOR: &str = "\n===\n";
/// The separator before one interleaved payload.
const FRAME_SEPARATOR: &str = "\n--- frame ";

/// Stands in for the loopback address in the golden, so the recording keeps
/// its exact bytes without pinning an ephemeral port.
const DEST_PLACEHOLDER: &str = "<loopback>";

/// Placeholder for the interleaved RTP timestamp (RFC 3550 §5.1), the one
/// field a wall-clock read still sets: it is the elapsed time since the
/// connection, so only its presence is pinned, never a value in it.
const RTP_TIMESTAMP_PLACEHOLDER: &str = "xxxx";

/// The bytes of the RTP fixed header up to the timestamp, and of one field.
const RTP_HEADER_BEFORE_TIMESTAMP: usize = 4;
const RTP_FIELD_LEN: usize = 4;

/// The pinned SSRC and clock origin (see the module doc).
const GOLDEN_SSRC: u32 = 0x5EED_2368;

/// Bound on the whole loopback exchange, so a wiring bug fails rather than
/// hangs.
const GUARD: Duration = Duration::from_secs(20);

/// RFC 2326 §10.12: the `$` byte that starts an interleaved frame, whose
/// next byte is the channel and the two after it the payload length.
const INTERLEAVED_PREFIX: u8 = b'$';
/// Bytes of a frame between its `$` and its payload.
const INTERLEAVED_HEADER_LEN: usize = 4;

/// The response `session` answers `request` with, or `None` when the session
/// rejects it outright.
fn answer(session: &mut ServerSession, request: &[u8]) -> Option<Vec<u8>> {
    session
        .handle_request(request)
        .ok()
        .map(|(response, _)| response)
}

/// The recorded media batch: one second of `fixtures/ts/h264-aac.ts`, so the
/// packetiser covers whole seven-packet RTP packets and a final partial one.
fn ts_batch() -> Vec<bytes::Bytes> {
    let ts = std::fs::read(fixture_path("ts/h264-aac.ts")).expect("h264-aac.ts fixture");
    let media = transmux::TsDemux::new()
        .demux(&ts)
        .expect("demux h264-aac.ts");
    let mut second = media.clone();
    for track in &mut second.tracks {
        track.samples.retain(|sample| {
            sample
                .timing.map(|t| t.pts)
                .is_some_and(|pts| u64::try_from(pts).is_ok_and(|p| p < SECOND))
        });
    }
    let bytes = transmux::TsMux::new()
        .package(&second)
        .expect("re-mux one second");
    vec![bytes.into()]
}

/// One second on the fixture's 90 kHz clock.
const SECOND: u64 = 90_000;

/// Masks the RTP timestamp of an interleaved payload's hex rendering.
fn mask_timestamp(hex: &str) -> String {
    let start = RTP_HEADER_BEFORE_TIMESTAMP * 2;
    let end = start + RTP_FIELD_LEN * 2;
    match hex.get(..start).zip(hex.get(end..)) {
        Some((before, after)) => format!("{before}{RTP_TIMESTAMP_PLACEHOLDER}{after}"),
        None => hex.to_string(),
    }
}

/// Lowercase hex with no separators.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// One recorded request with the interleaved payloads that followed it.
#[derive(Default)]
struct Recorded {
    head: String,
    frames: Vec<(u8, Vec<u8>)>,
}

impl Recorded {
    /// Appends this request's head and payloads to `out`.
    fn render(&self, out: &mut String) {
        out.push_str(&self.head);
        for (channel, payload) in &self.frames {
            out.push_str(FRAME_SEPARATOR);
            let hex = mask_timestamp(&hex(payload));
            out.push_str(&format!("channel {channel} === {hex}\n"));
        }
        out.push_str(REQUEST_SEPARATOR);
    }
}

/// Replaces CRLF with LF and drops a trailing blank line, so the golden is
/// diff-friendly; the request bytes themselves are unchanged otherwise.
fn flatten(head: &[u8]) -> String {
    String::from_utf8_lossy(head)
        .replace("\r\n", "\n")
        .trim_end()
        .to_string()
}

/// Serves one push connection until the client closes it, returning every
/// request with the interleaved payloads that followed it.
async fn serve(mut sock: tokio::net::TcpStream) -> Vec<Recorded> {
    let mut session = ServerSession::new(|| 1);
    let mut recorded: Vec<Recorded> = Vec::new();
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; 8192];
    loop {
        let n = match sock.read(&mut chunk).await {
            Ok(0) | Err(_) => return recorded,
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..n]);
        loop {
            if buf.first() == Some(&INTERLEAVED_PREFIX) {
                if buf.len() < INTERLEAVED_HEADER_LEN {
                    break;
                }
                let channel = buf[1];
                let len = usize::from(buf[2]) << 8 | usize::from(buf[3]);
                if buf.len() < INTERLEAVED_HEADER_LEN + len {
                    break;
                }
                let payload = buf[INTERLEAVED_HEADER_LEN..INTERLEAVED_HEADER_LEN + len].to_vec();
                buf.drain(..INTERLEAVED_HEADER_LEN + len);
                if let Some(last) = recorded.last_mut() {
                    last.frames.push((channel, payload));
                }
                continue;
            }
            let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
                break;
            };
            let head_len = end + 4;
            let headers = String::from_utf8_lossy(&buf[..head_len]).into_owned();
            let body_len = headers
                .lines()
                .find_map(|l| {
                    let (name, value) = l.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            if buf.len() < head_len + body_len {
                break;
            }
            let request: Vec<u8> = buf.drain(..head_len + body_len).collect();
            recorded.push(Recorded {
                head: flatten(&request),
                frames: Vec::new(),
            });
            let Some(response) = answer(&mut session, &request) else {
                return recorded;
            };
            if sock.write_all(&response).await.is_err() {
                return recorded;
            }
        }
    }
}

#[tokio::test]
async fn push_wire_matches_the_recorded_golden() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (sock, _) = listener.accept().await.unwrap();
        serve(sock).await
    });

    let url = format!("rtsp://{addr}/live/key");
    let cfg = RtspTransportConfig {
        test_wire: Some(RtpWireState {
            ssrc: GOLDEN_SSRC,
            started: Instant::now(),
        }),
        ..Default::default()
    };
    let mut transport = tokio::time::timeout(GUARD, RtspTransport::connect(&url, &cfg))
        .await
        .expect("connect must not hang")
        .expect("connect");
    tokio::time::timeout(GUARD, transport.setup(&[]))
        .await
        .expect("setup must not hang")
        .expect("setup");

    tokio::time::sleep(BEFORE_MEDIA).await;

    let batch = ts_batch();
    for _ in 0..2 {
        for message in &batch {
            tokio::time::timeout(GUARD, transport.send(message))
                .await
                .expect("send must not hang")
                .expect("send");
        }
    }
    transport.close();
    let recorded = tokio::time::timeout(GUARD, server)
        .await
        .expect("server must finish")
        .expect("server task");
    let mut rendered = String::new();
    for entity in &recorded {
        entity.render(&mut rendered);
    }

    let file = "golden/rtsp/push-wire.golden";
    if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
        std::fs::create_dir_all(&dir).expect("create golden dir");
        std::fs::write(std::path::Path::new(&dir).join(file), &rendered).expect("write");
        return;
    }
    let recorded = rendered.replace(&addr.to_string(), DEST_PLACEHOLDER);
    let expected = std::fs::read_to_string(test_fixtures::path(file)).expect("read golden");
    assert_eq!(recorded, expected, "{file} differs from the recorded wire");
}
