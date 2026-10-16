//! `handshake_loopback` — an SRT HSv5 Caller/Listener handshake, in memory
//! (`draft-sharabayko-srt-01` §4.3.1).
//!
//! No sockets: the bytes each state machine emits are parsed with the
//! `ControlPacket` codec and fed straight to the other side, exactly as a
//! transport would. The run prints the induction -> conclusion exchange and the
//! parameters both sides agreed on (version, the greater-of-both latency rule,
//! cross-matched socket ids, the Stream ID).
//!
//! ```text
//! cargo run -p srt-runtime --example handshake_loopback
//! ```

use srt_runtime::HandshakeConfig;
use srt_runtime::caller::{CallerHandshake, CallerHandshakeState};
use srt_runtime::handshake_sm::HandshakeOutput;
use srt_runtime::listener::{ListenerHandshake, ListenerHandshakeState};
use srt_runtime::packet::ControlPacket;

const CALLER_SOCKET_ID: u32 = 0x1111_1111;
const LISTENER_SOCKET_ID: u32 = 0x2222_2222;
/// The listener's SYN cookie. A real listener derives it from the peer address
/// and a secret; a constant is fine for a demo.
const SYN_COOKIE: u32 = 0xC0FF_EE42;
const MAX_ROUND_TRIPS: usize = 10;

/// The first `Send` payload among a state machine's outputs, if any.
fn first_send(outputs: Vec<HandshakeOutput>) -> Option<Vec<u8>> {
    outputs.into_iter().find_map(|o| match o {
        HandshakeOutput::Send(bytes) => Some(bytes),
        _ => None,
    })
}

fn main() {
    let mut caller = CallerHandshake::new(
        CALLER_SOCKET_ID,
        HandshakeConfig {
            latency_ms: 180,
            stream_id: Some("live/camera-1".to_string()),
            ..HandshakeConfig::default()
        },
    );
    let mut listener = ListenerHandshake::new(
        LISTENER_SOCKET_ID,
        SYN_COOKIE,
        HandshakeConfig {
            latency_ms: 250,
            ..HandshakeConfig::default()
        },
    );

    let mut to_listener = caller.start().expect("caller start");
    for round in 1..=MAX_ROUND_TRIPS {
        let pkt = ControlPacket::parse(&to_listener).expect("caller wire bytes");
        println!(
            "round {round}: caller -> listener ({} bytes), caller {:?}",
            to_listener.len(),
            caller.state()
        );
        let outputs = listener.feed(&pkt).expect("listener feed");
        let Some(reply) = first_send(outputs) else {
            break;
        };
        let pkt = ControlPacket::parse(&reply).expect("listener wire bytes");
        println!(
            "         listener -> caller ({} bytes), listener {:?}",
            reply.len(),
            listener.state()
        );
        match first_send(caller.feed(&pkt).expect("caller feed")) {
            Some(next) => to_listener = next,
            None => break,
        }
    }

    assert_eq!(caller.state(), CallerHandshakeState::Connected);
    assert_eq!(listener.state(), ListenerHandshakeState::Connected);
    let c = caller.negotiated().expect("caller negotiated");
    let l = listener.negotiated().expect("listener negotiated");
    println!(
        "connected: version {}, latency {} ms (caller asked 180, listener 250), stream id {:?}",
        c.version, c.latency_ms, c.stream_id
    );
    println!(
        "socket ids: caller own=0x{:08X} peer=0x{:08X}; listener own=0x{:08X} peer=0x{:08X}",
        c.own_socket_id, c.peer_socket_id, l.own_socket_id, l.peer_socket_id
    );
    assert_eq!(c.latency_ms, l.latency_ms);
    assert_eq!(c.peer_socket_id, l.own_socket_id);
    assert_eq!(l.peer_socket_id, c.own_socket_id);
}
