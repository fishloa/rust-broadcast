//! Manual browser-interop smoke test for the `media` feature (issues
//! #740/#743): a minimal WHIP-lite HTTP endpoint that hands the negotiated
//! SDP off to [`webrtc_runtime::media::MediaTransport`] and waits for a real
//! browser to publish to it.
//!
//! This is **not** a CI-run example — `required-features = ["media"]` keeps
//! it out of a plain `cargo build --examples`, and even with `media` on,
//! nothing here asserts success on its own: it needs an independent WebRTC
//! peer on the other end of the UDP socket. It exists to reproduce, against
//! this crate's own implementation, the same proof the feasibility spike
//! established against the narrow `rtc-ice`/`rtc-dtls`/`rtc-srtp` crates
//! directly: a real browser's SRTP decrypted and its RTP header parsed by
//! this workspace's own `rtp-packet`.
//!
//! Run:
//!
//! ```text
//! cargo run -p webrtc-runtime --features media --example whip_media_smoke
//! ```
//!
//! then, separately, serve a page that does a WHIP POST of an audio-only
//! SDP offer to `http://127.0.0.1:8787/whip` and open it in a browser
//! launched with WebRTC test-automation flags, e.g. Chrome:
//!
//! ```text
//! google-chrome \
//!   --use-fake-ui-for-media-stream --use-fake-device-for-media-stream \
//!   --force-webrtc-ip-handling-policy=default_public_and_private_interfaces
//! ```

use std::io::{Read, Write};
use std::net::{TcpListener, UdpSocket};
use std::time::{Duration, Instant};

use webrtc_runtime::media::{MediaEvent, MediaTransport, MediaTransportConfig, SetupRole};

/// The WHIP-lite signalling port this smoke test listens on.
const SIGNALLING_PORT: u16 = 8787;

/// A short pseudo-random token for ICE ufrag/pwd, seeded from
/// [`std::collections::hash_map::RandomState`] (itself OS-random per
/// process) rather than pulling in a `rand` dependency for one call site.
fn rand_token(len: usize) -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;

    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let state = RandomState::new();
    (0..len)
        .map(|i| {
            let idx = (state.hash_one(i) as usize) % CHARS.len();
            CHARS[idx] as char
        })
        .collect()
}

/// Extremely small HTTP/1.1 request reader: one request, headers + body.
fn read_http_request(stream: &mut std::net::TcpStream) -> (String, String) {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = stream.read(&mut tmp).unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&buf[..pos]).to_string();
            let content_length = headers
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            let body_start = pos + 4;
            while buf.len() < body_start + content_length {
                let n = stream.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            let method_line = headers.lines().next().unwrap_or("").to_string();
            let body_end = (body_start + content_length).min(buf.len());
            let body = String::from_utf8_lossy(&buf[body_start..body_end]).to_string();
            return (method_line, body);
        }
    }
    (String::new(), String::new())
}

fn sdp_line<'a>(sdp: &'a str, prefix: &str) -> Option<&'a str> {
    sdp.lines().find_map(|l| l.strip_prefix(prefix))
}

fn main() {
    let udp = UdpSocket::bind("127.0.0.1:0").expect("bind udp");
    let local_addr = udp.local_addr().unwrap();
    println!("[smoke] UDP media socket bound at {local_addr}");

    let listener =
        TcpListener::bind(("127.0.0.1", SIGNALLING_PORT)).expect("bind whip-lite http port");
    println!("[smoke] WHIP-lite signalling listening on http://127.0.0.1:{SIGNALLING_PORT}/whip");

    // ---- 1. Wait for the browser's SDP offer -------------------------------
    let (offer_sdp, mut stream) = loop {
        let (mut stream, _) = listener.accept().expect("accept");
        let (method_line, body) = read_http_request(&mut stream);
        if method_line.starts_with("OPTIONS") {
            let resp = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nContent-Length: 0\r\n\r\n";
            let _ = stream.write_all(resp.as_bytes());
            continue;
        }
        if method_line.starts_with("POST") {
            break (body, stream);
        }
    };
    println!("[smoke] received SDP offer ({} bytes)", offer_sdp.len());

    let remote_ufrag = sdp_line(&offer_sdp, "a=ice-ufrag:")
        .unwrap_or("")
        .trim()
        .to_string();
    let remote_pwd = sdp_line(&offer_sdp, "a=ice-pwd:")
        .unwrap_or("")
        .trim()
        .to_string();
    let remote_fingerprint = sdp_line(&offer_sdp, "a=fingerprint:")
        .unwrap_or("")
        .trim()
        .to_string();
    let mid = sdp_line(&offer_sdp, "a=mid:")
        .unwrap_or("0")
        .trim()
        .to_string();
    println!("[smoke] remote ice-ufrag={remote_ufrag} fingerprint={remote_fingerprint}");

    let remote_candidates: Vec<String> = offer_sdp
        .lines()
        .filter_map(|l| l.strip_prefix("a=candidate:"))
        .map(str::to_string)
        .collect();
    println!(
        "[smoke] remote offered {} candidate(s)",
        remote_candidates.len()
    );

    let m_audio_line = offer_sdp
        .lines()
        .find(|l| l.starts_with("m=audio"))
        .unwrap_or("m=audio 9 UDP/TLS/RTP/SAVPF 111")
        .to_string();
    let codec_lines: Vec<&str> = offer_sdp
        .lines()
        .filter(|l| {
            l.starts_with("a=rtpmap:") || l.starts_with("a=fmtp:") || l.starts_with("a=rtcp-fb:")
        })
        .collect();

    // ---- 2. Build the media transport (our own crate's public API) --------
    let local_ice_ufrag = rand_token(8);
    let local_ice_pwd = rand_token(24);
    let mut media = MediaTransport::new(MediaTransportConfig {
        local_addr,
        local_ice_ufrag: local_ice_ufrag.clone(),
        local_ice_pwd: local_ice_pwd.clone(),
        remote_ice_ufrag: remote_ufrag,
        remote_ice_pwd: remote_pwd,
        is_controlling: false,
        local_setup: SetupRole::Passive,
        stun_server: None,
    })
    .expect("build media transport");
    println!(
        "[smoke] local DTLS cert fingerprint sha-256 {}",
        media.local_fingerprint()
    );

    for raw in &remote_candidates {
        match media.add_remote_candidate(raw) {
            Ok(()) => println!("[smoke] added remote candidate: {raw}"),
            Err(e) => println!("[smoke] skipping unparseable candidate {raw:?}: {e}"),
        }
    }

    // ---- 3. Send the SDP answer ---------------------------------------------
    let answer_candidate_line = format!(
        "0 1 udp 2130706431 {} {} typ host",
        local_addr.ip(),
        local_addr.port()
    );
    let mut answer = String::new();
    answer.push_str("v=0\r\n");
    answer.push_str("o=- 0 0 IN IP4 127.0.0.1\r\n");
    answer.push_str("s=-\r\n");
    answer.push_str("t=0 0\r\n");
    answer.push_str(&format!("{m_audio_line}\r\n"));
    answer.push_str("c=IN IP4 127.0.0.1\r\n");
    answer.push_str("a=rtcp:9 IN IP4 0.0.0.0\r\n");
    for l in &codec_lines {
        answer.push_str(l);
        answer.push_str("\r\n");
    }
    answer.push_str("a=recvonly\r\n");
    answer.push_str(&format!("a=mid:{mid}\r\n"));
    answer.push_str("a=rtcp-mux\r\n");
    answer.push_str(&format!("a=ice-ufrag:{local_ice_ufrag}\r\n"));
    answer.push_str(&format!("a=ice-pwd:{local_ice_pwd}\r\n"));
    answer.push_str(&format!(
        "a=fingerprint:sha-256 {}\r\n",
        media.local_fingerprint()
    ));
    answer.push_str("a=setup:passive\r\n");
    answer.push_str(&format!("a=candidate:{answer_candidate_line}\r\n"));
    answer.push_str("a=end-of-candidates\r\n");

    let http_resp = format!(
        "HTTP/1.1 201 Created\r\nContent-Type: application/sdp\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Expose-Headers: Location\r\nLocation: /whip/1\r\nContent-Length: {}\r\n\r\n{}",
        answer.len(),
        answer
    );
    stream
        .write_all(http_resp.as_bytes())
        .expect("write answer");
    let _ = stream.shutdown(std::net::Shutdown::Write);
    println!("[smoke] SDP answer sent, {} bytes", answer.len());

    // ---- 4. Drive the media transport until a real RTP packet decrypts ----
    println!("[smoke] entering media loop -- waiting for ICE connectivity + DTLS handshake ...");
    udp.set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(30);
    let mut buf = [0u8; 2048];
    let mut success = false;

    while Instant::now() < deadline && !success {
        while let Some(dgram) = media.poll_transmit() {
            let _ = udp.send_to(&dgram.bytes, dgram.peer);
        }

        match udp.recv_from(&mut buf) {
            Ok((n, peer)) => {
                let events = match media.handle_datagram(Instant::now(), peer, &buf[..n]) {
                    Ok(events) => events,
                    Err(e) => {
                        println!("[smoke] handle_datagram error: {e}");
                        continue;
                    }
                };
                for event in events {
                    match event {
                        MediaEvent::LocalCandidateGathered(c) => {
                            println!("[smoke] gathered local candidate: {c}");
                        }
                        MediaEvent::IceStateChanged(s) => {
                            println!("[smoke] ICE state changed: {s}");
                        }
                        MediaEvent::DtlsHandshakeComplete => {
                            println!("[smoke] DTLS handshake complete with {peer}");
                        }
                        MediaEvent::Rtp(pkt) => {
                            println!(
                                "[smoke] DECRYPTED inbound SRTP packet from {peer}: {} bytes plaintext payload",
                                pkt.payload.len()
                            );
                            println!(
                                "[smoke] RTP header (parsed by workspace rtp-packet crate): marker={} pt={} seq={} ts={} ssrc=0x{:08x} csrc_count={}",
                                pkt.marker,
                                pkt.payload_type,
                                pkt.sequence_number,
                                pkt.timestamp,
                                pkt.ssrc,
                                pkt.csrc.len()
                            );
                            success = true;
                        }
                        MediaEvent::Rtcp(compound) => {
                            println!(
                                "[smoke] decrypted inbound SRTCP compound packet: {} sub-packet(s)",
                                compound.packets.len()
                            );
                        }
                        _ => {}
                    }
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                media.handle_timeout(Instant::now());
            }
            Err(e) => println!("[smoke] udp recv error: {e}"),
        }
    }

    if success {
        println!("[smoke] SUCCESS: decrypted a real inbound SRTP packet via MediaTransport.");
    } else {
        println!("[smoke] TIMED OUT without decrypting an SRTP packet -- see log above.");
        std::process::exit(1);
    }
}
