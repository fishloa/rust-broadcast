//! `client_server_loopback` — drive the sans-IO RTSP client against the sans-IO
//! server entirely in memory (RFC 2326 Appendix A state machines).
//!
//! There is no socket: each request the [`ClientSession`] produces is handed to
//! [`ServerSession::handle_request`], and each response goes straight back into
//! [`ClientSession::handle_data_strict`]. The run walks
//! OPTIONS -> SETUP -> PLAY -> (interleaved RTP) -> TEARDOWN and prints the
//! client/server state after every step.
//!
//! ```text
//! cargo run -p rtsp-runtime --example client_server_loopback
//! ```

use rtsp_runtime::{
    ClientEvent, ClientSession, InterleavedFrame, RtspClientConfig, RtspServerConfig, ServerEvent,
    ServerSession, SessionIdSource, Transport, TransportSpec,
};

const URL: &str = "rtsp://127.0.0.1/stream";
const TRACK: &str = "rtsp://127.0.0.1/stream/trackID=0";

/// One request/response exchange: client bytes -> server -> client events.
fn exchange(
    step: &str,
    client: &mut ClientSession,
    server: &mut ServerSession,
    request: Vec<u8>,
) -> Vec<ClientEvent> {
    let (response, server_events) = server
        .handle_request(&request)
        .expect("server accepts the request");
    let client_events = client
        .handle_data_strict(&response)
        .expect("client accepts the response");
    println!(
        "{step:<9} client={:?} server={:?} ({} server event(s), {} client event(s))",
        client.state(),
        server.state(),
        server_events.len(),
        client_events.len()
    );
    for ev in &server_events {
        if let ServerEvent::SessionSetup {
            session_id,
            transport,
        } = ev
        {
            println!(
                "          session {session_id}, transport {}",
                transport.to_header_value().expect("serialize transport")
            );
        }
    }
    client_events
}

fn main() {
    // A fixed id is fine for a demo; a real server draws from the CSPRNG
    // (`SessionIdSource::default()`).
    let mut server = ServerSession::new(
        RtspServerConfig::default()
            .with_session_ids(SessionIdSource::counter(0x0123_4567_89ab_cdef)),
    )
    .expect("a server session");
    let mut client = ClientSession::new(RtspClientConfig::default()).expect("a client session");

    let req = client.options(URL).expect("OPTIONS");
    exchange("OPTIONS", &mut client, &mut server, req);

    let transport = Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1));
    let req = client.setup(TRACK, &transport).expect("SETUP");
    exchange("SETUP", &mut client, &mut server, req);
    let session = client.session_id().expect("SETUP returned a session id");
    println!("          client captured Session: {session}");

    let req = client.play(URL).expect("PLAY");
    exchange("PLAY", &mut client, &mut server, req);

    // While playing, the server tunnels RTP on interleaved channel 0 over the
    // same connection. Feed the client one such `$`-frame.
    let rtp = [0x80, 0x60, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 1, 0xde, 0xad];
    let wire = InterleavedFrame::slice_to_bytes(0, &rtp).expect("frame RTP");
    for ev in client.handle_data_strict(&wire).expect("media frame") {
        if let ClientEvent::MediaData { channel, data } = ev {
            println!(
                "MEDIA     channel {channel}: {} bytes (round-trips: {})",
                data.len(),
                data.as_ref() == rtp
            );
            assert_eq!(data.as_ref(), rtp);
        }
    }

    let req = client.teardown(URL).expect("TEARDOWN");
    exchange("TEARDOWN", &mut client, &mut server, req);
}
