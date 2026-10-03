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
//! (the signalling port is OS-assigned and printed, or the first CLI argument),
//! then, separately, serve a page that does a WHIP POST of an audio-only
//! SDP offer to `http://127.0.0.1:<port>/whip` and open it in a browser
//! launched with WebRTC test-automation flags, e.g. Chrome:
//!
//! ```text
//! google-chrome \
//!   --use-fake-ui-for-media-stream --use-fake-device-for-media-stream \
//!   --force-webrtc-ip-handling-policy=default_public_and_private_interfaces
//! ```

use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::post;
use sdp_types::{Attribute, Fingerprint, Session, TypedAttribute};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::{mpsc, oneshot};
use webrtc_runtime::media::{
    MAX_REMOTE_CANDIDATES, MediaEvent, MediaTransport, MediaTransportConfig, SetupRole,
    parse_remote_fingerprint,
};

/// The WHIP-lite signalling port: OS-assigned (0) unless the first CLI
/// argument names one, so parallel runs never collide on a fixed port.
const DEFAULT_SIGNALLING_PORT: u16 = 0;

/// How long the media loop waits for a real RTP packet before giving up.
const MEDIA_BUDGET: Duration = Duration::from_secs(30);

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

/// One WHIP POST handed from the HTTP task to the media task: the offer, and
/// the channel the SDP answer comes back on.
struct Offer {
    sdp: String,
    answer: oneshot::Sender<String>,
}

async fn whip(State(tx): State<mpsc::Sender<Offer>>, body: String) -> impl IntoResponse {
    let (answer, rx) = oneshot::channel();
    if tx.send(Offer { sdp: body, answer }).await.is_err() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    match rx.await {
        Ok(sdp) => (
            StatusCode::CREATED,
            [
                (header::CONTENT_TYPE, "application/sdp"),
                (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                (header::ACCESS_CONTROL_EXPOSE_HEADERS, "Location"),
                (header::LOCATION, "/whip/1"),
            ],
            sdp,
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn preflight() -> impl IntoResponse {
    (
        StatusCode::NO_CONTENT,
        [
            (
                header::ACCESS_CONTROL_ALLOW_ORIGIN,
                HeaderValue::from_static("*"),
            ),
            (
                header::ACCESS_CONTROL_ALLOW_METHODS,
                HeaderValue::from_static("POST, OPTIONS"),
            ),
            (
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                HeaderValue::from_static("Content-Type"),
            ),
        ],
    )
}

/// The attribute names an answer keeps from the offer's audio section: the
/// codec description and the media identity. Everything transport-related is
/// replaced with this side's own values.
const KEPT_ATTRIBUTES: &[&str] = &["rtpmap", "fmtp", "rtcp-fb", "mid", "rtcp-mux", "rtcp"];

/// The answer to `offer`: its first audio section (codec attributes kept),
/// receive-only, with this side's ICE credentials, DTLS fingerprint, passive
/// setup role and host candidate. Built from the parsed offer with
/// `sdp-types` and written back with `Session::write`.
fn build_answer(
    offer: &Session,
    ufrag: &str,
    pwd: &str,
    fingerprint: &str,
    candidate: &str,
) -> Vec<u8> {
    let mut answer = offer.clone();
    answer.attributes.clear();
    answer.medias.retain(|m| m.media == "audio");
    answer.medias.truncate(1);
    for media in &mut answer.medias {
        media
            .attributes
            .retain(|a| KEPT_ATTRIBUTES.contains(&a.attribute.as_str()));
        let mut attr = |name: &str, value: Option<String>| {
            media.attributes.push(Attribute {
                attribute: name.into(),
                value,
            });
        };
        attr("recvonly", None);
        attr("ice-ufrag", Some(ufrag.into()));
        attr("ice-pwd", Some(pwd.into()));
        attr("fingerprint", Some(format!("sha-256 {fingerprint}")));
        attr("setup", Some("passive".into()));
        attr("candidate", Some(candidate.into()));
        attr("end-of-candidates", None);
    }
    let mut out = Vec::new();
    answer.write(&mut out).expect("write answer to a Vec");
    out
}

#[tokio::main]
async fn main() {
    let udp = UdpSocket::bind("127.0.0.1:0").await.expect("bind udp");
    let local_addr = udp.local_addr().unwrap();
    println!("[smoke] UDP media socket bound at {local_addr}");

    // Signalling port: first argument, default OS-assigned and printed; never a fixed port.
    let port: u16 = std::env::args()
        .nth(1)
        .map(|a| a.parse().expect("first argument must be a port number"))
        .unwrap_or(DEFAULT_SIGNALLING_PORT);
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("bind whip-lite http port");
    println!(
        "[smoke] WHIP-lite signalling listening on http://{}/whip",
        listener.local_addr().expect("signalling local_addr")
    );
    let (tx, mut offers) = mpsc::channel::<Offer>(1);
    let app = Router::new()
        .route("/whip", post(whip).options(preflight))
        .with_state(tx);
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });

    // ---- 1. Wait for the browser's SDP offer -------------------------------
    let Offer {
        sdp: offer_sdp,
        answer,
    } = offers.recv().await.expect("an offer");
    println!("[smoke] received SDP offer ({} bytes)", offer_sdp.len());

    let offer = Session::parse(offer_sdp.as_bytes()).expect("the offer is a valid SDP");
    let audio = offer
        .medias
        .iter()
        .find(|m| m.media == "audio")
        .expect("the offer has an audio section");
    // ICE credentials: media-level first, then session-level (RFC 8839 §5.4).
    let attr = |name: &str| -> String {
        audio
            .get_first_attribute_value(name)
            .or_else(|| offer.get_first_attribute_value(name))
            .flatten()
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let remote_ufrag = attr("ice-ufrag");
    let remote_pwd = attr("ice-pwd");
    let remote_fingerprint = parse_remote_fingerprint(&offer_sdp)
        .unwrap_or_else(|| panic!("[smoke] offer has no a=fingerprint; refusing to publish"));
    println!("[smoke] remote ice-ufrag={remote_ufrag} fingerprint={remote_fingerprint}");

    let remote_candidates: Vec<String> = audio
        .attributes
        .iter()
        .filter(|a| a.attribute == "candidate")
        .filter_map(|a| a.value.clone())
        .collect();
    println!(
        "[smoke] remote offered {} candidate(s)",
        remote_candidates.len()
    );

    // ---- 2. Build the media transport (our own crate's public API) --------
    let local_ice_ufrag = rand_token(8);
    let local_ice_pwd = rand_token(24);
    let mut media = MediaTransport::new(
        MediaTransportConfig {
            local_addr,
            local_ice_ufrag: local_ice_ufrag.clone(),
            local_ice_pwd: local_ice_pwd.clone(),
            remote_ice_ufrag: remote_ufrag,
            remote_ice_pwd: remote_pwd,
            is_controlling: false,
            local_setup: SetupRole::Passive,
            stun_server: None,
            remote_fingerprint,
            max_remote_candidates: MAX_REMOTE_CANDIDATES,
        },
        std::time::Instant::now(),
    )
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
    let answer_sdp = build_answer(
        &offer,
        &local_ice_ufrag,
        &local_ice_pwd,
        media.local_fingerprint(),
        &media.local_candidates().remove(0),
    );
    // Sanity: the answer we are about to send carries our own typed fingerprint.
    debug_assert!(
        Session::parse(&answer_sdp)
            .ok()
            .and_then(|s| {
                s.medias[0]
                    .attributes
                    .iter()
                    .find(|a| a.attribute.eq_ignore_ascii_case(Fingerprint::NAME))
                    .cloned()
            })
            .is_some()
    );
    let answer_len = answer_sdp.len();
    let _ = answer.send(String::from_utf8(answer_sdp).expect("an SDP is UTF-8"));
    println!("[smoke] SDP answer sent, {answer_len} bytes");

    // ---- 4. Drive the media transport until a real RTP packet decrypts ----
    println!("[smoke] entering media loop -- waiting for ICE connectivity + DTLS handshake ...");
    // Event-driven: socket read, outbound drain, and the transport's own
    // `poll_timeout` deadline. No fixed tick, no read timeout.
    let deadline = tokio::time::Instant::now() + MEDIA_BUDGET;
    let mut buf = [0u8; 2048];
    let mut success = false;

    while !success && tokio::time::Instant::now() < deadline {
        while let Some(dgram) = media.poll_transmit() {
            let _ = udp.send_to(&dgram.bytes, dgram.peer).await;
        }
        let wake = media
            .poll_timeout()
            .map(tokio::time::Instant::from_std)
            .unwrap_or(deadline)
            .min(deadline);

        tokio::select! {
            r = udp.recv_from(&mut buf) => match r {
                Ok((n, peer)) => {
                    let events = match media.handle_datagram(std::time::Instant::now(), peer, &buf[..n]) {
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
                Err(e) => println!("[smoke] udp recv error: {e}"),
            },
            _ = tokio::time::sleep_until(wake) => {
                for event in media.handle_timeout(std::time::Instant::now()) {
                    if let MediaEvent::TimerError(msg) = event {
                        println!("[smoke] timer error: {msg}");
                    }
                }
            }
        }
    }

    if success {
        println!("[smoke] SUCCESS: decrypted a real inbound SRTP packet via MediaTransport.");
    } else {
        println!("[smoke] TIMED OUT without decrypting an SRTP packet -- see log above.");
        std::process::exit(1);
    }
}
